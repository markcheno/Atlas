//! OS keychain adapter for [`SecretStore`] via `keyring-core` and the
//! platform store for each OS: macOS Keychain, Windows Credential Manager,
//! Linux Secret Service (pure-Rust D-Bus client).
//!
//! Debug builds use a separate service name so development never touches
//! the keys of an installed Atlas.

use std::sync::Arc;

use keyring_core::{CredentialStore, Error as KeyringError};

use crate::error::{AtlasError, AtlasResult};
use crate::services::providers::secrets::{Secret, SecretStore};

pub const SERVICE: &str = if cfg!(debug_assertions) {
    "com.altazlabs.atlas.debug"
} else {
    "com.altazlabs.atlas"
};

pub struct KeychainStore {
    store: Arc<CredentialStore>,
}

impl KeychainStore {
    /// `None` when this platform's credential store can't be reached (for
    /// example no Secret Service running on Linux).
    pub fn open() -> Option<Self> {
        platform_store().ok().map(|store| Self { store })
    }

    fn entry(&self, account: &str) -> AtlasResult<keyring_core::Entry> {
        self.store
            .build(SERVICE, account, None)
            .map_err(|_| AtlasError::SecretStoreFailed)
    }
}

/// Opens the keychain on first use instead of at startup, waiting at most
/// [`OPEN_TIMEOUT`]. Connecting can block for a long time (on Linux, D-Bus
/// tries to start a Secret Service that may not exist, about 25 s), and
/// most sessions never touch a key, so startup must not wait for it. If it
/// can't be opened, every call fails and keys are kept for the session
/// only, as when no keychain exists.
#[derive(Default)]
pub struct LazyKeychain {
    opened: std::sync::OnceLock<Option<Arc<KeychainStore>>>,
}

pub const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl LazyKeychain {
    fn store(&self) -> AtlasResult<&KeychainStore> {
        self.opened
            .get_or_init(|| {
                let (tx, rx) = std::sync::mpsc::channel();
                let spawned = std::thread::Builder::new()
                    .name("atlas-keychain-open".into())
                    .spawn(move || {
                        let _ = tx.send(KeychainStore::open().map(Arc::new));
                    });
                // A thread still stuck after the timeout is left to finish
                // on its own; its result is ignored.
                spawned
                    .ok()
                    .and_then(|_| rx.recv_timeout(OPEN_TIMEOUT).ok())
                    .flatten()
            })
            .as_deref()
            .ok_or(AtlasError::SecretStoreFailed)
    }
}

impl SecretStore for LazyKeychain {
    fn set(&self, account: &str, secret: &Secret) -> AtlasResult<()> {
        self.store()?.set(account, secret)
    }

    fn get(&self, account: &str) -> AtlasResult<Option<Secret>> {
        self.store()?.get(account)
    }

    fn delete(&self, account: &str) -> AtlasResult<()> {
        self.store()?.delete(account)
    }
}

#[cfg(target_os = "macos")]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

#[cfg(windows)]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(windows_native_keyring_store::Store::new()?)
}

#[cfg(target_os = "linux")]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(zbus_secret_service_keyring_store::Store::new()?)
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Err(KeyringError::NoDefaultStore)
}

impl SecretStore for KeychainStore {
    fn set(&self, account: &str, secret: &Secret) -> AtlasResult<()> {
        self.entry(account)?
            .set_password(secret.expose())
            .map_err(|_| AtlasError::SecretStoreFailed)
    }

    fn get(&self, account: &str) -> AtlasResult<Option<Secret>> {
        match self.entry(account)?.get_password() {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(_) => Err(AtlasError::SecretStoreFailed),
        }
    }

    fn delete(&self, account: &str) -> AtlasResult<()> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(_) => Err(AtlasError::SecretStoreFailed),
        }
    }
}
