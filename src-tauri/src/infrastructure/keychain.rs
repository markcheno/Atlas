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
