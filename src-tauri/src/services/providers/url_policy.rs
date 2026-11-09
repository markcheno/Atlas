//! Which AI-service addresses Atlas will contact (phase0-spec §5.7, §10).
//!
//! - Only `http` and `https`. No user name or password in the address, and
//!   no query or fragment.
//! - HTTPS is always allowed.
//! - Plain HTTP is allowed without asking only for this computer
//!   (`localhost`, `127.0.0.0/8`, `::1`). The name `localhost` is matched
//!   literally, never resolved through DNS.
//! - Any other plain-HTTP address needs the profile's explicit
//!   acknowledgement. The rule does not depend on telling LAN from
//!   internet: names, VPN ranges and mDNS make that unreliable.
//!
//! [`location`] gives the privacy label shown beside a service (Slice 2.1,
//! ADR 0012): this computer, another computer nearby, or an internet
//! service. It reads the address only, never DNS, so it is a label for the
//! user, not a security boundary.

use std::net::{Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

use crate::domain::dto::ProviderLocation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Normalized, without a trailing slash.
    pub base: String,
    pub on_this_computer: bool,
    pub plain_http: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlProblem {
    Invalid,
    HasCredentials,
    /// Plain HTTP to another computer without acknowledgement.
    NeedsAcknowledgement,
}

pub fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(d)) => {
            d.eq_ignore_ascii_case("localhost") || d.eq_ignore_ascii_case("localhost.")
        }
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn private_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_private() || ip.is_link_local() || (a == 100 && (64..128).contains(&b))
}

fn private_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
}

/// Where a service runs, judged from its address alone (no DNS).
///
/// - This computer: loopback.
/// - Another computer: private, link-local and carrier-grade NAT addresses;
///   `.local`, `.lan`, `.home.arpa` and `.internal` names, and single-label
///   host names, which only resolve on a local network.
/// - Internet: everything else.
pub fn location(url: &Url) -> ProviderLocation {
    if is_loopback(url) {
        return ProviderLocation::ThisComputer;
    }
    let nearby = match url.host() {
        Some(Host::Ipv4(ip)) => private_v4(ip),
        Some(Host::Ipv6(ip)) => private_v6(ip),
        Some(Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            !d.contains('.')
                || [".local", ".lan", ".home.arpa", ".internal"]
                    .iter()
                    .any(|suffix| d.ends_with(suffix))
        }
        None => false,
    };
    if nearby {
        ProviderLocation::AnotherComputer
    } else {
        ProviderLocation::Internet
    }
}

/// [`location`] of a stored address; an unparsable one counts as the
/// internet, the most cautious label.
pub fn location_of(raw: &str) -> ProviderLocation {
    Url::parse(raw).map_or(ProviderLocation::Internet, |u| location(&u))
}

pub fn check(raw: &str, acknowledged: bool) -> Result<Endpoint, UrlProblem> {
    let url = Url::parse(raw.trim()).map_err(|_| UrlProblem::Invalid)?;
    let plain_http = match url.scheme() {
        "https" => false,
        "http" => true,
        _ => return Err(UrlProblem::Invalid),
    };
    if url.host().is_none() || url.cannot_be_a_base() {
        return Err(UrlProblem::Invalid);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(UrlProblem::HasCredentials);
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(UrlProblem::Invalid);
    }
    let on_this_computer = is_loopback(&url);
    if plain_http && !on_this_computer && !acknowledged {
        return Err(UrlProblem::NeedsAcknowledgement);
    }
    let base = url.as_str().trim_end_matches('/').to_owned();
    Ok(Endpoint {
        base,
        on_this_computer,
        plain_http,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        let ok = |s: &str| check(s, false).map(|e| e.base);
        assert_eq!(
            ok("http://localhost:11434/"),
            Ok("http://localhost:11434".into())
        );
        assert_eq!(
            ok("http://127.0.0.1:8000/v1/"),
            Ok("http://127.0.0.1:8000/v1".into())
        );
        assert_eq!(
            ok("http://127.3.4.5:8000"),
            Ok("http://127.3.4.5:8000".into())
        );
        assert_eq!(
            ok("http://[::1]:1234/v1"),
            Ok("http://[::1]:1234/v1".into())
        );
        assert_eq!(
            ok("https://api.openai.com/v1"),
            Ok("https://api.openai.com/v1".into())
        );
        assert_eq!(
            ok("  http://LOCALHOST:11434  "),
            Ok("http://localhost:11434".into())
        );

        assert_eq!(
            check("http://192.168.1.20:11434", false),
            Err(UrlProblem::NeedsAcknowledgement)
        );
        assert_eq!(
            check("http://studio.local:11434", false),
            Err(UrlProblem::NeedsAcknowledgement)
        );
        assert_eq!(
            check("http://localhost.example.com", false),
            Err(UrlProblem::NeedsAcknowledgement)
        );
        assert!(check("http://192.168.1.20:11434", true).is_ok());

        assert_eq!(
            check("https://user:pass@api.example.com/v1", false),
            Err(UrlProblem::HasCredentials)
        );
        assert_eq!(
            check("https://key@api.example.com", false),
            Err(UrlProblem::HasCredentials)
        );
        assert_eq!(check("ftp://localhost", false), Err(UrlProblem::Invalid));
        assert_eq!(check("file:///etc/passwd", false), Err(UrlProblem::Invalid));
        assert_eq!(check("localhost:11434", false), Err(UrlProblem::Invalid));
        assert_eq!(
            check("http://localhost:11434/?x=1", false),
            Err(UrlProblem::Invalid)
        );
        assert_eq!(check("not a url", false), Err(UrlProblem::Invalid));
    }

    #[test]
    fn locations() {
        use ProviderLocation::*;
        for (url, want) in [
            ("http://localhost:8000/v1", ThisComputer),
            ("http://127.0.0.1:11434", ThisComputer),
            ("http://[::1]:1234/v1", ThisComputer),
            ("http://192.168.1.20:11434", AnotherComputer),
            ("http://10.0.0.5:8000/v1", AnotherComputer),
            ("http://172.16.4.2:8000", AnotherComputer),
            ("http://172.32.0.1:8000", Internet),
            ("http://169.254.10.10:8000", AnotherComputer),
            ("http://100.101.102.103:11434", AnotherComputer),
            ("http://100.128.0.1:11434", Internet),
            ("http://[fd12:3456::1]:8000", AnotherComputer),
            ("http://[fe80::1]:8000", AnotherComputer),
            ("http://studio.local:11434", AnotherComputer),
            ("http://nas.lan:8000/v1", AnotherComputer),
            ("http://gpu.home.arpa:8000", AnotherComputer),
            ("http://mac-studio:11434", AnotherComputer),
            ("https://api.openai.com/v1", Internet),
            ("https://openrouter.ai/api/v1", Internet),
            ("http://8.8.8.8:8000", Internet),
            ("http://localhost.example.com", Internet),
        ] {
            assert_eq!(location_of(url), want, "{url}");
        }
        assert_eq!(location_of("not a url"), Internet);
    }

    #[test]
    fn loopback_flag() {
        assert!(check("http://localhost:1", false).unwrap().on_this_computer);
        assert!(
            !check("https://api.openai.com", false)
                .unwrap()
                .on_this_computer
        );
    }
}
