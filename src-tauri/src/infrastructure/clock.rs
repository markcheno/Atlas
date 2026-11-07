//! Wall-clock timestamps in the catalog's RFC 3339 UTC format.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned())
}
