//! Enum ⇄ TEXT for catalog columns, using the same snake_case names as the
//! IPC contract, so a state stored as `paused_user` reads back exactly.

use serde::Serialize;
use serde::de::DeserializeOwned;

pub fn to_db<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

pub fn from_db<T: DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(text.to_owned())).ok()
}

pub fn opt_from_db<T: DeserializeOwned>(text: Option<String>) -> Option<T> {
    text.as_deref().and_then(from_db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::dto::{JobKind, JobState, PauseReason};

    #[test]
    fn enum_text_matches_the_ipc_names() {
        assert_eq!(to_db(&JobState::PausedPolicy), "paused_policy");
        assert_eq!(
            from_db::<JobState>("paused_user"),
            Some(JobState::PausedUser)
        );
        assert_eq!(to_db(&PauseReason::BatteryLow), "battery_low");
        assert_eq!(from_db::<JobState>("nonsense"), None);
        assert_eq!(to_db(&JobKind::Scan), "scan");
        assert_eq!(from_db::<JobKind>("extract"), Some(JobKind::Extract));
        assert_eq!(from_db::<JobKind>("ocr"), None);
    }
}
