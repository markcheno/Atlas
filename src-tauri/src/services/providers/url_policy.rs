//! Which AI-service addresses Atlas will contact (phase0-spec §5.7, §10).
//!
//! - Only `http` and `https`. No user name or password in the address, and
//!   no query or fragment.
//! - HTTPS is always allowed.
//! - Plain HTTP is allowed without asking only for this computer
//!   (`localhost`, `127.0.0.0/8`, `::1`). The name `localhost` is matched
//!   literally, never resolved through DNS.
//! - Any other plain-HTTP address needs the profile's explicit
//!   acknowledgement. Atlas does not try to tell LAN from internet: names,
//!   VPN ranges and mDNS make that unreliable.

use url::{Host, Url};

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
    fn loopback_flag() {
        assert!(check("http://localhost:1", false).unwrap().on_this_computer);
        assert!(
            !check("https://api.openai.com", false)
                .unwrap()
                .on_this_computer
        );
    }
}
