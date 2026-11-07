//! API keys: kept in the OS keychain, or for one session only when the
//! keychain is unavailable. Never written to the catalog, logs, or IPC.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::domain::dto::CredentialStorage;
use crate::error::AtlasResult;

/// A secret string whose `Debug` output is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// A persistent secret store (the OS keychain in the app; fakes in tests).
pub trait SecretStore: Send + Sync {
    fn set(&self, account: &str, secret: &Secret) -> AtlasResult<()>;
    fn get(&self, account: &str) -> AtlasResult<Option<Secret>>;
    fn delete(&self, account: &str) -> AtlasResult<()>;
}

pub fn account_for(profile_id: &str) -> String {
    format!("provider:{profile_id}")
}

/// The keychain plus an in-memory fallback for this session.
pub struct SecretVault {
    store: Option<Arc<dyn SecretStore>>,
    session: Mutex<HashMap<String, Secret>>,
}

impl SecretVault {
    pub fn new(store: Option<Arc<dyn SecretStore>>) -> Self {
        Self {
            store,
            session: Mutex::new(HashMap::new()),
        }
    }

    fn session(&self) -> std::sync::MutexGuard<'_, HashMap<String, Secret>> {
        self.session.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Stores a key, preferring the keychain. Returns where it went.
    pub fn put(&self, account: &str, secret: Secret) -> CredentialStorage {
        if let Some(store) = &self.store
            && store.set(account, &secret).is_ok()
        {
            self.session().remove(account);
            return CredentialStorage::Keychain;
        }
        self.session().insert(account.to_owned(), secret);
        CredentialStorage::SessionOnly
    }

    pub fn get(&self, account: &str, storage: CredentialStorage) -> Option<Secret> {
        match storage {
            CredentialStorage::None => None,
            CredentialStorage::SessionOnly => self.session().get(account).cloned(),
            CredentialStorage::Keychain => self
                .store
                .as_ref()
                .and_then(|s| s.get(account).ok().flatten()),
        }
    }

    /// Whether a key recorded as `storage` is actually available now (a
    /// session-only key is gone after a restart).
    pub fn available(&self, account: &str, storage: CredentialStorage) -> bool {
        match storage {
            CredentialStorage::None => false,
            CredentialStorage::SessionOnly => self.session().contains_key(account),
            CredentialStorage::Keychain => true,
        }
    }

    pub fn remove(&self, account: &str) -> AtlasResult<()> {
        self.session().remove(account);
        match &self.store {
            Some(store) => store.delete(account),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_secret() {
        let s = Secret::new("sk-very-secret");
        assert!(!format!("{s:?}").contains("very-secret"));
    }

    #[test]
    fn falls_back_to_the_session_without_a_store() {
        let vault = SecretVault::new(None);
        let storage = vault.put("provider:x", Secret::new("k"));
        assert_eq!(storage, CredentialStorage::SessionOnly);
        assert_eq!(vault.get("provider:x", storage).unwrap().expose(), "k");
        assert!(vault.available("provider:x", storage));
        let fresh = SecretVault::new(None);
        assert!(
            !fresh.available("provider:x", storage),
            "gone after restart"
        );
    }
}
