//! Connecting accounts: turns stored accounts and secrets into rclone remotes
//! inside a running daemon, and adds new accounts safely.

use std::sync::Arc;

use bufonidae_rclone::rcd::Rcd;
use serde_json::{Map, Value};

use crate::account::{self, Account, AccountStore};
use crate::secrets::{self, SecretKind, SecretStore};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Account(#[from] account::Error),
    #[error(transparent)]
    Secrets(#[from] secrets::Error),
    #[error("rclone: {0}")]
    Rclone(#[from] bufonidae_rclone::Error),
    #[error("login failed for {email}: {source}")]
    Login {
        email: String,
        #[source]
        source: bufonidae_rclone::Error,
    },
    #[error("no stored password for account {0:?}")]
    MissingPassword(String),
    #[error("secret store task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

pub type SharedSecrets = Arc<dyn SecretStore>;

/// A Mega session: lets later starts skip the (single-use) 2FA code.
struct Session {
    id: String,
    master_key: String,
}

async fn secret_get(
    store: &SharedSecrets,
    account: &Account,
    kind: SecretKind,
) -> Result<Option<String>, Error> {
    let (store, id) = (Arc::clone(store), account.id.clone());
    Ok(tokio::task::spawn_blocking(move || store.get(&id, kind)).await??)
}

async fn secret_set(
    store: &SharedSecrets,
    account: &Account,
    kind: SecretKind,
    value: String,
) -> Result<(), Error> {
    let (store, id) = (Arc::clone(store), account.id.clone());
    Ok(tokio::task::spawn_blocking(move || store.set(&id, kind, &value)).await??)
}

/// Best effort: removes every secret of the account.
async fn secrets_clear(store: &SharedSecrets, account: &Account) -> Result<(), Error> {
    let mut first_error = None;
    for kind in [
        SecretKind::Password,
        SecretKind::SessionId,
        SecretKind::MasterKey,
    ] {
        let (store, id) = (Arc::clone(store), account.id.clone());
        let result = tokio::task::spawn_blocking(move || store.delete(&id, kind)).await?;
        if let Err(e) = result {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), |e| Err(e.into()))
}

async fn create_remote(
    rcd: &Rcd,
    account: &Account,
    password: &str,
    two_fa: Option<&str>,
    session: Option<&Session>,
) -> Result<(), Error> {
    let mut params = Map::new();
    params.insert("user".into(), Value::from(account.email.as_str()));
    params.insert("pass".into(), Value::from(password));
    match session {
        Some(s) => {
            params.insert("session_id".into(), Value::from(s.id.as_str()));
            params.insert("master_key".into(), Value::from(s.master_key.as_str()));
        }
        None => {
            if let Some(code) = two_fa {
                params.insert("2fa".into(), Value::from(code));
            }
        }
    }
    rcd.config_create(&account.remote_name(), "mega", &params)
        .await?;
    Ok(())
}

/// Adds an account: defines the remote, proves the login with a real call,
/// and only then persists the metadata and the secrets. On any failure
/// nothing is left behind.
pub async fn add_account(
    rcd: &Rcd,
    store: &mut AccountStore,
    secrets: &SharedSecrets,
    label: &str,
    email: &str,
    password: &str,
    two_fa: Option<&str>,
) -> Result<Account, Error> {
    let account = store.prepare(label, email)?;
    let remote = account.remote_name();

    create_remote(rcd, &account, password, two_fa, None).await?;
    let result = persist_after_login(rcd, store, secrets, &account, password).await;
    if result.is_err() {
        // Best effort: the daemon's config is in memory only.
        let _ = rcd.config_delete(&remote).await;
    }
    result.map(|()| account)
}

async fn persist_after_login(
    rcd: &Rcd,
    store: &mut AccountStore,
    secrets: &SharedSecrets,
    account: &Account,
    password: &str,
) -> Result<(), Error> {
    rcd.about(&format!("{}:", account.remote_name()))
        .await
        .map_err(|source| Error::Login {
            email: account.email.clone(),
            source,
        })?;

    let outcome = async {
        secret_set(secrets, account, SecretKind::Password, password.to_owned()).await?;
        // If rclone recorded the session after logging in, keep it.
        let stored = rcd.config_get(&account.remote_name()).await?;
        if let (Some(id), Some(key)) = (stored.get("session_id"), stored.get("master_key")) {
            secret_set(secrets, account, SecretKind::SessionId, id.clone()).await?;
            secret_set(secrets, account, SecretKind::MasterKey, key.clone()).await?;
        }
        store.commit(account.clone())?;
        Ok::<(), Error>(())
    }
    .await;
    if outcome.is_err() {
        let _ = secrets_clear(secrets, account).await;
    }
    outcome
}

/// Signs an existing account in again with a (new) password, for example after
/// a restore or a password change. The account keeps its id, so the index
/// still refers to it. Nothing changes unless the login is proven.
pub async fn reauthorize(
    rcd: &Rcd,
    store: &AccountStore,
    secrets: &SharedSecrets,
    label: &str,
    password: &str,
    two_fa: Option<&str>,
) -> Result<Account, Error> {
    let account = store
        .find(label)
        .cloned()
        .ok_or_else(|| account::Error::NotFound(label.to_owned()))?;
    let remote = account.remote_name();

    create_remote(rcd, &account, password, two_fa, None).await?;
    let result = async {
        rcd.about(&format!("{remote}:"))
            .await
            .map_err(|source| Error::Login {
                email: account.email.clone(),
                source,
            })?;
        secret_set(secrets, &account, SecretKind::Password, password.to_owned()).await?;
        // The old session belongs to the old password: replace it or drop it.
        let stored = rcd.config_get(&remote).await?;
        match (stored.get("session_id"), stored.get("master_key")) {
            (Some(id), Some(key)) => {
                secret_set(secrets, &account, SecretKind::SessionId, id.clone()).await?;
                secret_set(secrets, &account, SecretKind::MasterKey, key.clone()).await?;
            }
            _ => {
                for kind in [SecretKind::SessionId, SecretKind::MasterKey] {
                    let (secrets, id) = (Arc::clone(secrets), account.id.clone());
                    tokio::task::spawn_blocking(move || secrets.delete(&id, kind)).await??;
                }
            }
        }
        Ok::<(), Error>(())
    }
    .await;
    if result.is_err() {
        let _ = rcd.config_delete(&remote).await;
    }
    result.map(|()| account)
}

/// Defines the remote of every stored account in the daemon, from the stored
/// password (and session, when there is one). Returns one result per account
/// so that a single bad account does not hide the others.
pub async fn connect_all(
    rcd: &Rcd,
    store: &AccountStore,
    secrets: &SharedSecrets,
) -> Vec<(Account, Result<(), Error>)> {
    let mut results = Vec::new();
    for account in store.list() {
        let result = connect_one(rcd, secrets, account).await;
        results.push((account.clone(), result));
    }
    results
}

async fn connect_one(rcd: &Rcd, secrets: &SharedSecrets, account: &Account) -> Result<(), Error> {
    let password = secret_get(secrets, account, SecretKind::Password)
        .await?
        .ok_or_else(|| Error::MissingPassword(account.label.clone()))?;
    let session = match (
        secret_get(secrets, account, SecretKind::SessionId).await?,
        secret_get(secrets, account, SecretKind::MasterKey).await?,
    ) {
        (Some(id), Some(master_key)) => Some(Session { id, master_key }),
        _ => None,
    };
    create_remote(rcd, account, &password, None, session.as_ref()).await
}

/// Removes an account: its secrets first, then its metadata.
pub async fn remove_account(
    store: &mut AccountStore,
    secrets: &SharedSecrets,
    label: &str,
) -> Result<Account, Error> {
    let account = store
        .find(label)
        .cloned()
        .ok_or_else(|| account::Error::NotFound(label.to_owned()))?;
    secrets_clear(secrets, &account).await?;
    Ok(store.remove(label)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    #[tokio::test]
    async fn remove_clears_secrets_and_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = AccountStore::load(dir.path().join("accounts.toml")).unwrap();
        let account = store.add("work", "a@example.com").unwrap();
        let secrets: SharedSecrets = Arc::new(MemoryStore::default());
        secret_set(&secrets, &account, SecretKind::Password, "pw".into())
            .await
            .unwrap();

        remove_account(&mut store, &secrets, "work").await.unwrap();

        assert!(store.list().is_empty());
        assert_eq!(
            secret_get(&secrets, &account, SecretKind::Password)
                .await
                .unwrap(),
            None
        );
        assert!(remove_account(&mut store, &secrets, "work").await.is_err());
    }

    #[tokio::test]
    async fn reauthorizing_an_unknown_label_fails_without_touching_anything() {
        let Ok(rcd) = Rcd::start("rclone").await else {
            if std::env::var_os("BUFONIDAE_REQUIRE_RCLONE").is_some() {
                panic!("rclone is required");
            }
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let store = AccountStore::load(dir.path().join("accounts.toml")).unwrap();
        let secrets: SharedSecrets = Arc::new(MemoryStore::default());
        let result = reauthorize(&rcd, &store, &secrets, "nobody", "pw", None).await;
        assert!(matches!(
            result,
            Err(Error::Account(account::Error::NotFound(_)))
        ));
        rcd.shutdown().await.unwrap();
    }
}
