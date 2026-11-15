//! Accounts and the on-disk store of their non-secret metadata.
//!
//! Secrets (passwords, sessions) never live here: see [`crate::secrets`].

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const MAX_LABEL_LEN: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "invalid label {0:?}: use 1-{MAX_LABEL_LEN} characters from a-z, A-Z, 0-9, '.', '_' or '-'"
    )]
    InvalidLabel(String),
    #[error("an account labelled {0:?} already exists")]
    DuplicateLabel(String),
    #[error(
        "{email} is already connected as {label:?}; connecting it twice would make two copies \
         look like they are on different accounts"
    )]
    DuplicateAccount { email: String, label: String },
    #[error("the account list is not empty: restoring would mix two sets of accounts")]
    NotEmpty,
    #[error("invalid or repeated account id {0:?}")]
    InvalidId(String),
    #[error("no account labelled {0:?}")]
    NotFound(String),
    #[error("could not determine the configuration directory")]
    NoConfigDir,
    #[error("accounts file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("accounts file {path} is corrupt: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("could not serialize accounts: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// A connected Mega account. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Stable identifier, independent of the label.
    pub id: String,
    /// Human-friendly unique name.
    pub label: String,
    pub email: String,
}

impl Account {
    /// Name of this account's remote inside the rclone daemon. Derived from
    /// the id so that no user-controlled text reaches rclone's remote syntax.
    #[must_use]
    pub fn remote_name(&self) -> String {
        format!("acct-{}", self.id)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default, rename = "account")]
    accounts: Vec<Account>,
}

/// TOML file with the list of accounts.
#[derive(Debug)]
pub struct AccountStore {
    path: PathBuf,
    file: File,
}

impl AccountStore {
    /// `$XDG_CONFIG_HOME/bufonidae/accounts.toml`.
    pub fn default_path() -> Result<PathBuf, Error> {
        let dirs = directories::ProjectDirs::from("", "", "bufonidae").ok_or(Error::NoConfigDir)?;
        Ok(dirs.config_dir().join("accounts.toml"))
    }

    /// Loads the store, or starts an empty one when the file does not exist.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, Error> {
        let path = path.into();
        let file = match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|source| Error::Parse {
                path: path.clone(),
                source,
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => File::default(),
            Err(source) => {
                return Err(Error::Io { path, source });
            }
        };
        Ok(Self { path, file })
    }

    #[must_use]
    pub fn list(&self) -> &[Account] {
        &self.file.accounts
    }

    #[must_use]
    pub fn find(&self, label: &str) -> Option<&Account> {
        self.file.accounts.iter().find(|a| a.label == label)
    }

    /// Validates `label`, checks it is free and returns a new account with a
    /// fresh id, without storing it: callers verify the login first, then
    /// [`commit`](Self::commit).
    pub fn prepare(&self, label: &str, email: &str) -> Result<Account, Error> {
        validate_label(label)?;
        if self.find(label).is_some() {
            return Err(Error::DuplicateLabel(label.to_owned()));
        }
        self.check_new_identity(email)?;
        Ok(Account {
            id: uuid::Uuid::new_v4().simple().to_string(),
            label: label.to_owned(),
            email: email.to_owned(),
        })
    }

    /// The same Mega account under two labels is refused: mirroring counts
    /// copies on distinct accounts, so a second label for one account would
    /// make a single point of failure look redundant. E-mails are compared
    /// ignoring case and surrounding spaces.
    fn check_new_identity(&self, email: &str) -> Result<(), Error> {
        let wanted = email.trim().to_lowercase();
        match self
            .file
            .accounts
            .iter()
            .find(|a| a.email.trim().to_lowercase() == wanted)
        {
            Some(existing) => Err(Error::DuplicateAccount {
                email: email.trim().to_owned(),
                label: existing.label.clone(),
            }),
            None => Ok(()),
        }
    }

    /// Stores an account returned by [`prepare`](Self::prepare) and saves.
    pub fn commit(&mut self, account: Account) -> Result<(), Error> {
        if self.find(&account.label).is_some() {
            return Err(Error::DuplicateLabel(account.label));
        }
        self.check_new_identity(&account.email)?;
        self.file.accounts.push(account);
        if let Err(e) = self.save() {
            self.file.accounts.pop();
            return Err(e);
        }
        Ok(())
    }

    /// `prepare` + `commit`.
    pub fn add(&mut self, label: &str, email: &str) -> Result<Account, Error> {
        let account = self.prepare(label, email)?;
        self.commit(account.clone())?;
        Ok(account)
    }

    /// Loads exported accounts into an **empty** store, keeping their ids (the
    /// index refers to them). Validates every entry like a new account.
    pub fn restore(&mut self, accounts: Vec<Account>) -> Result<(), Error> {
        if !self.file.accounts.is_empty() {
            return Err(Error::NotEmpty);
        }
        let mut staged = AccountStore {
            path: self.path.clone(),
            file: File::default(),
        };
        for account in accounts {
            let id_ok = !account.id.is_empty()
                && account.id.len() <= 64
                && account
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-');
            if !id_ok {
                return Err(Error::InvalidId(account.id));
            }
            if staged.file.accounts.iter().any(|a| a.id == account.id) {
                return Err(Error::InvalidId(account.id));
            }
            validate_label(&account.label)?;
            if staged.find(&account.label).is_some() {
                return Err(Error::DuplicateLabel(account.label));
            }
            staged.check_new_identity(&account.email)?;
            staged.file.accounts.push(account);
        }
        staged.save()?;
        self.file = staged.file;
        Ok(())
    }

    /// Removes the account labelled `label` and saves the store.
    pub fn remove(&mut self, label: &str) -> Result<Account, Error> {
        let index = self
            .file
            .accounts
            .iter()
            .position(|a| a.label == label)
            .ok_or_else(|| Error::NotFound(label.to_owned()))?;
        let account = self.file.accounts.remove(index);
        self.save()?;
        Ok(account)
    }

    /// Writes atomically (temp file + rename) with mode 0600: the file reveals
    /// which accounts the user owns.
    fn save(&self) -> Result<(), Error> {
        let io = |source| Error::Io {
            path: self.path.clone(),
            source,
        };
        let text = toml::to_string_pretty(&self.file)?;
        crate::fsutil::write_private(&self.path, &text).map_err(io)
    }
}

fn validate_label(label: &str) -> Result<(), Error> {
    let ok = !label.is_empty()
        && label.len() <= MAX_LABEL_LEN
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidLabel(label.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn store(dir: &tempfile::TempDir) -> AccountStore {
        AccountStore::load(dir.path().join("sub/accounts.toml")).unwrap()
    }

    #[test]
    fn add_persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        let a = s.add("work", "a@example.com").unwrap();
        let reloaded = store(&dir);
        assert_eq!(reloaded.list(), std::slice::from_ref(&a));
        assert_eq!(reloaded.find("work"), Some(&a));
        assert_eq!(a.remote_name(), format!("acct-{}", a.id));
    }

    #[test]
    fn file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        s.add("work", "a@example.com").unwrap();
        let mode = fs::metadata(dir.path().join("sub/accounts.toml"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn rejects_duplicate_and_invalid_labels() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        s.add("work", "a@example.com").unwrap();
        assert!(matches!(
            s.add("work", "b@example.com"),
            Err(Error::DuplicateLabel(_))
        ));
        for bad in ["", "has space", "a/b", "x:", &"y".repeat(33)] {
            assert!(matches!(
                s.add(bad, "c@example.com"),
                Err(Error::InvalidLabel(_))
            ));
        }
    }

    #[test]
    fn remove_deletes_and_reports_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        s.add("work", "a@example.com").unwrap();
        s.remove("work").unwrap();
        assert!(store(&dir).list().is_empty());
        assert!(matches!(s.remove("work"), Err(Error::NotFound(_))));
    }

    #[test]
    fn corrupt_file_is_an_error_not_an_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.toml");
        fs::write(&path, "not [valid").unwrap();
        assert!(matches!(
            AccountStore::load(&path),
            Err(Error::Parse { .. })
        ));
    }

    #[test]
    fn the_same_mega_account_cannot_be_connected_twice() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir);
        s.add("work", "Ana@Example.com").unwrap();
        for dup in ["ana@example.com", " ANA@EXAMPLE.COM ", "Ana@Example.com"] {
            assert!(
                matches!(s.add("other", dup), Err(Error::DuplicateAccount { .. })),
                "{dup}"
            );
        }
        assert!(s.list().len() == 1);
        s.add("other", "ana2@example.com").unwrap();
    }
}
