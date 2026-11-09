//! AI service connections (phase0-spec §5.7, §10; phase2-spec §7.1; ADR
//! 0007, ADR 0012): saved profiles, URL rules, API keys in the keychain,
//! connection and capability tests, model roles, recommendations, and
//! finding services on this computer on request. A test lists models or
//! sends one fixed request; nothing from the library.

pub mod detect;
pub mod profiles;
pub mod recommend;
pub mod roles;
pub mod secrets;
pub mod url_policy;
