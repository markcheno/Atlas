//! AI service connections (phase0-spec §5.7, §10; ADR 0007): saved
//! profiles, URL rules, API keys in the keychain, connection tests, and
//! finding services on this computer on request. No inference: a test only
//! asks the service for its list of models.

pub mod detect;
pub mod profiles;
pub mod secrets;
pub mod url_policy;
