//! What may go to an AI service (phase2-spec §5.7, §7.6, §8; ADR 0015).
//!
//! Services on this computer or another one are never restricted here (ADR
//! 0007's address rules still apply). For an internet service:
//!
//! - "Disable all external AI" blocks every request: Ask Atlas, embedding
//!   the library, embedding a search, and the connection and model tests.
//! - Embedding the library (and so a search's question) needs the opt-in.
//! - Ask Atlas needs the disclosure agreed to, per profile and address.

use crate::domain::dto::{Acknowledgement, PrivacySettings, ProviderLocation};

/// What a request would be for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    /// Ask Atlas: the question and excerpts.
    Ask,
    /// The embedding job, or a search's question: library text.
    Embed,
    /// A connection or capability test: fixed text, nothing from the library.
    Test,
}

/// Whether a request for `what` may go to a service at `location`.
/// The disclosure is asked separately ([`acknowledged`]): it is a step,
/// not a refusal.
pub fn allows(p: &PrivacySettings, location: ProviderLocation, what: Use) -> bool {
    if location != ProviderLocation::Internet {
        return true;
    }
    if p.disable_external_ai {
        return false;
    }
    match what {
        Use::Embed => p.allow_remote_embedding,
        Use::Ask | Use::Test => true,
    }
}

/// Whether embedding may use an internet service, as the embedding job,
/// its status and search ask it (a service elsewhere needs nothing).
pub fn remote_embedding(p: &PrivacySettings) -> bool {
    allows(p, ProviderLocation::Internet, Use::Embed)
}

/// Whether the user agreed to the disclosure for this profile at this
/// address.
pub fn acknowledged(p: &PrivacySettings, profile_id: &str, base_url: &str) -> bool {
    p.acknowledged
        .iter()
        .any(|a| a.profile_id == profile_id && a.base_url == base_url)
}

/// Records an agreement, replacing an older one for the same profile.
pub fn acknowledge(p: &mut PrivacySettings, entry: Acknowledgement) {
    p.acknowledged.retain(|a| a.profile_id != entry.profile_id);
    p.acknowledged.push(entry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProviderLocation::{AnotherComputer, Internet, ThisComputer};

    fn settings(remote: bool, disabled: bool) -> PrivacySettings {
        PrivacySettings {
            allow_remote_embedding: remote,
            disable_external_ai: disabled,
            acknowledged: Vec::new(),
        }
    }

    #[test]
    fn nearby_services_are_never_restricted() {
        for p in [settings(false, true), settings(false, false)] {
            for what in [Use::Ask, Use::Embed, Use::Test] {
                assert!(allows(&p, ThisComputer, what));
                assert!(allows(&p, AnotherComputer, what));
            }
        }
    }

    #[test]
    fn internet_services_follow_the_two_choices() {
        let p = settings(false, false);
        assert!(allows(&p, Internet, Use::Ask));
        assert!(allows(&p, Internet, Use::Test));
        assert!(!allows(&p, Internet, Use::Embed));
        assert!(allows(&settings(true, false), Internet, Use::Embed));
        // Disable all external AI wins over the opt-in.
        let off = settings(true, true);
        for what in [Use::Ask, Use::Embed, Use::Test] {
            assert!(!allows(&off, Internet, what));
        }
        assert!(!remote_embedding(&off));
        assert!(remote_embedding(&settings(true, false)));
    }

    #[test]
    fn an_agreement_is_for_one_profile_at_one_address() {
        let mut p = settings(false, false);
        let entry = |url: &str| Acknowledgement {
            profile_id: "p".into(),
            base_url: url.into(),
            label: "Service".into(),
            at: "2026-11-10T00:00:00Z".into(),
        };
        acknowledge(&mut p, entry("https://a.example/v1"));
        assert!(acknowledged(&p, "p", "https://a.example/v1"));
        assert!(!acknowledged(&p, "p", "https://b.example/v1"));
        assert!(!acknowledged(&p, "q", "https://a.example/v1"));
        acknowledge(&mut p, entry("https://b.example/v1"));
        assert_eq!(p.acknowledged.len(), 1, "replaced, not added");
        assert!(acknowledged(&p, "p", "https://b.example/v1"));
    }
}
