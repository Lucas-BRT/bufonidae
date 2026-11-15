//! Storage of account secrets, kept out of every file Bufonidae writes.
//!
//! Calls are blocking (the Secret Service goes over D-Bus); async callers
//! should wrap them in `spawn_blocking`.

use std::collections::HashMap;
use std::sync::Mutex;

const SERVICE: &str = "bufonidae";

/// What is stored for an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretKind {
    Password,
    /// Mega session, reused so that 2FA codes (single use) are needed only
    /// at the first login.
    SessionId,
    MasterKey,
}

impl SecretKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::SessionId => "session_id",
            Self::MasterKey => "master_key",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("secret store: {0}")]
    Backend(String),
}

/// A place to keep secrets, addressed by account id and kind.
pub trait SecretStore: Send + Sync {
    fn get(&self, account_id: &str, kind: SecretKind) -> Result<Option<String>, Error>;
    fn set(&self, account_id: &str, kind: SecretKind, value: &str) -> Result<(), Error>;
    /// Deleting something that is not there is not an error.
    fn delete(&self, account_id: &str, kind: SecretKind) -> Result<(), Error>;
}

/// The desktop's secret store (GNOME Keyring, KWallet, ...) through the
/// Secret Service.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

impl KeyringStore {
    fn entry(account_id: &str, kind: SecretKind) -> Result<keyring::Entry, Error> {
        keyring::Entry::new(SERVICE, &format!("{account_id}/{}", kind.as_str()))
            .map_err(|e| Error::Backend(e.to_string()))
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, account_id: &str, kind: SecretKind) -> Result<Option<String>, Error> {
        match Self::entry(account_id, kind)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Backend(e.to_string())),
        }
    }

    fn set(&self, account_id: &str, kind: SecretKind, value: &str) -> Result<(), Error> {
        Self::entry(account_id, kind)?
            .set_password(value)
            .map_err(|e| Error::Backend(e.to_string()))
    }

    fn delete(&self, account_id: &str, kind: SecretKind) -> Result<(), Error> {
        match Self::entry(account_id, kind)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Backend(e.to_string())),
        }
    }
}

/// In-memory store for tests and for environments without a Secret Service.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<(String, SecretKind), String>>);

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemoryStore(..)")
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, account_id: &str, kind: SecretKind) -> Result<Option<String>, Error> {
        let map = self.0.lock().map_err(|e| Error::Backend(e.to_string()))?;
        Ok(map.get(&(account_id.to_owned(), kind)).cloned())
    }

    fn set(&self, account_id: &str, kind: SecretKind, value: &str) -> Result<(), Error> {
        let mut map = self.0.lock().map_err(|e| Error::Backend(e.to_string()))?;
        map.insert((account_id.to_owned(), kind), value.to_owned());
        Ok(())
    }

    fn delete(&self, account_id: &str, kind: SecretKind) -> Result<(), Error> {
        let mut map = self.0.lock().map_err(|e| Error::Backend(e.to_string()))?;
        map.remove(&(account_id.to_owned(), kind));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trip() {
        let s = MemoryStore::default();
        assert_eq!(s.get("a", SecretKind::Password).unwrap(), None);
        s.set("a", SecretKind::Password, "pw").unwrap();
        s.set("a", SecretKind::SessionId, "sid").unwrap();
        assert_eq!(
            s.get("a", SecretKind::Password).unwrap().as_deref(),
            Some("pw")
        );
        s.delete("a", SecretKind::Password).unwrap();
        s.delete("a", SecretKind::Password).unwrap();
        assert_eq!(s.get("a", SecretKind::Password).unwrap(), None);
        assert_eq!(
            s.get("a", SecretKind::SessionId).unwrap().as_deref(),
            Some("sid")
        );
    }

    /// Needs a running Secret Service (a desktop session), so it is not part
    /// of CI: `cargo test -p bufonidae-core -- --ignored`.
    #[test]
    #[ignore = "needs a desktop Secret Service"]
    fn keyring_store_round_trip() {
        let s = KeyringStore;
        let id = "test-only-account";
        s.set(id, SecretKind::Password, "pw").unwrap();
        assert_eq!(
            s.get(id, SecretKind::Password).unwrap().as_deref(),
            Some("pw")
        );
        s.delete(id, SecretKind::Password).unwrap();
        s.delete(id, SecretKind::Password).unwrap();
        assert_eq!(s.get(id, SecretKind::Password).unwrap(), None);
    }
}
