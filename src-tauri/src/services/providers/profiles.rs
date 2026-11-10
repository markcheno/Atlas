//! Saved provider profiles in the catalog; keys in the [`SecretVault`].

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::privacy::{self, Use};
use super::secrets::{Secret, SecretVault, account_for};
use super::url_policy::{self, Endpoint, UrlProblem};
use crate::domain::dto::{
    ConnectionOutcome, ConnectionResult, ConnectionTestTarget, CredentialStorage, PrivacySettings,
    ProviderKind, ProviderProfile, ProviderProfileInput,
};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::now_rfc3339;
use crate::services::catalog::codec::{from_db, to_db};
use crate::services::workspace::OpenRoot;

struct StoredProfile {
    id: String,
    kind: ProviderKind,
    label: String,
    base_url: String,
    storage: CredentialStorage,
    insecure_http_acknowledged: bool,
    created_at: String,
    updated_at: String,
}

const COLUMNS: &str = "id, kind, label, base_url, credential_storage, insecure_http_acknowledged, created_at, updated_at";

fn from_row(r: &Row) -> rusqlite::Result<StoredProfile> {
    let kind: String = r.get(1)?;
    let storage: Option<String> = r.get(4)?;
    Ok(StoredProfile {
        id: r.get(0)?,
        kind: from_db(&kind).unwrap_or(ProviderKind::OpenaiCompatible),
        label: r.get(2)?,
        base_url: r.get(3)?,
        storage: storage
            .as_deref()
            .and_then(from_db)
            .unwrap_or(CredentialStorage::None),
        insecure_http_acknowledged: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

fn to_dto(p: StoredProfile, vault: &SecretVault) -> ProviderProfile {
    let available = vault.available(&account_for(&p.id), p.storage);
    ProviderProfile {
        location: url_policy::location_of(&p.base_url),
        has_credential: available,
        credential_storage: if available {
            p.storage
        } else {
            CredentialStorage::None
        },
        id: p.id,
        kind: p.kind,
        label: p.label,
        base_url: p.base_url,
        insecure_http_acknowledged: p.insecure_http_acknowledged,
        created_at: p.created_at,
        updated_at: p.updated_at,
    }
}

fn load(root: &OpenRoot, id: &str) -> AtlasResult<Option<StoredProfile>> {
    let id = id.to_owned();
    root.reader()
        .read(move |c| load_with(c, &id))?
        .map_err(AtlasError::db("read AI services"))
}

fn load_with(conn: &Connection, id: &str) -> rusqlite::Result<Option<StoredProfile>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM provider_profiles WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()
}

pub fn list(root: &OpenRoot, vault: &SecretVault) -> AtlasResult<Vec<ProviderProfile>> {
    let rows = root
        .reader()
        .read(|c| -> rusqlite::Result<Vec<StoredProfile>> {
            let mut stmt = c.prepare(&format!(
                "SELECT {COLUMNS} FROM provider_profiles ORDER BY label, created_at"
            ))?;
            stmt.query_map([], from_row)?.collect()
        })?
        .map_err(AtlasError::db("read AI services"))?;
    Ok(rows.into_iter().map(|p| to_dto(p, vault)).collect())
}

fn url_error(problem: UrlProblem) -> AtlasError {
    match problem {
        UrlProblem::Invalid => AtlasError::InvalidUrl,
        UrlProblem::HasCredentials => AtlasError::UrlHasCredentials,
        UrlProblem::NeedsAcknowledgement => AtlasError::InsecureHttpNotAcknowledged,
    }
}

fn default_label(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Ollama => "Ollama",
        ProviderKind::OpenaiCompatible => "OpenAI-compatible service",
    }
}

/// Creates or updates a profile. The key, if given, goes to the vault.
pub fn save(
    root: &OpenRoot,
    vault: &SecretVault,
    input: ProviderProfileInput,
) -> AtlasResult<ProviderProfile> {
    let endpoint =
        url_policy::check(&input.base_url, input.insecure_http_acknowledged).map_err(url_error)?;
    let now = now_rfc3339();
    let existing = match &input.id {
        Some(id) => Some(load(root, id)?.ok_or(AtlasError::ProfileNotFound)?),
        None => None,
    };
    let id = existing
        .as_ref()
        .map(|p| p.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let account = account_for(&id);

    let mut storage = existing
        .as_ref()
        .map_or(CredentialStorage::None, |p| p.storage);
    if input.clear_api_key {
        vault.remove(&account)?;
        storage = CredentialStorage::None;
    }
    if let Some(key) = input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        storage = vault.put(&account, Secret::new(key));
    }

    let label = match input.label.trim() {
        "" => default_label(input.kind).to_owned(),
        l => l.to_owned(),
    };
    let row = (
        id.clone(),
        to_db(&input.kind),
        label,
        endpoint.base.clone(),
        match storage {
            CredentialStorage::None => None,
            other => Some(to_db(&other)),
        },
        input.insecure_http_acknowledged,
        existing
            .as_ref()
            .map_or(now.clone(), |p| p.created_at.clone()),
        now,
    );
    root.catalog()
        .call(move |c| {
            c.execute(
                "INSERT INTO provider_profiles (id, kind, label, base_url, credential_storage,
                     insecure_http_acknowledged, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (id) DO UPDATE SET kind = excluded.kind, label = excluded.label,
                     base_url = excluded.base_url, credential_storage = excluded.credential_storage,
                     insecure_http_acknowledged = excluded.insecure_http_acknowledged,
                     updated_at = excluded.updated_at",
                params![row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7],
            )
        })?
        .map_err(AtlasError::db("save the AI service"))?;
    let saved = load(root, &id)?.ok_or(AtlasError::ProfileNotFound)?;
    Ok(to_dto(saved, vault))
}

/// Removes the profile and its key.
pub fn delete(root: &OpenRoot, vault: &SecretVault, id: &str) -> AtlasResult<()> {
    if load(root, id)?.is_none() {
        return Err(AtlasError::ProfileNotFound);
    }
    vault.remove(&account_for(id))?;
    let id = id.to_owned();
    root.catalog()
        .call(move |c| c.execute("DELETE FROM provider_profiles WHERE id = ?1", [id]))?
        .map_err(AtlasError::db("remove the AI service"))?;
    Ok(())
}

/// Why a request to an internet service wasn't sent.
pub const EXTERNAL_AI_OFF: &str = "All external AI is turned off in Settings › Privacy.";

/// What a test should contact, or why it must not.
pub enum Prepared {
    Ready {
        kind: ProviderKind,
        endpoint: Endpoint,
        key: Option<Secret>,
    },
    /// The address breaks the URL rules; the message says how.
    Blocked(&'static str),
}

impl Prepared {
    /// Whether "Disable all external AI" stops this test: a ready
    /// internet service while the switch is on (ADR 0015).
    pub fn disabled_by(&self, privacy: &PrivacySettings) -> bool {
        match self {
            Prepared::Ready { endpoint, .. } => {
                !privacy::allows(privacy, url_policy::location_of(&endpoint.base), Use::Test)
            }
            Prepared::Blocked(_) => false,
        }
    }

    /// A refused connection test, for [`Prepared::Blocked`].
    pub fn blocked_connection(message: &str) -> ConnectionResult {
        ConnectionResult {
            outcome: ConnectionOutcome::InsecureUrlRejected,
            model_ids: Vec::new(),
            message: message.to_owned(),
            server_version: None,
            tested_at: now_rfc3339(),
        }
    }
}

fn blocked(problem: UrlProblem) -> Prepared {
    Prepared::Blocked(match problem {
        UrlProblem::Invalid => "This isn't a web address Atlas can use.",
        UrlProblem::HasCredentials => "The address contains a user name or password.",
        UrlProblem::NeedsAcknowledgement => {
            "Unencrypted HTTP to another computer needs your confirmation first."
        }
    })
}

/// Resolves a saved profile on any catalog connection (a background job
/// has no open-root handle): its endpoint under ADR 0007's rules and its
/// key. `ProfileNotFound` if the profile is gone.
pub fn resolve_saved(
    conn: &Connection,
    vault: &SecretVault,
    profile_id: &str,
) -> AtlasResult<Prepared> {
    let p = load_with(conn, profile_id)
        .map_err(AtlasError::db("read AI services"))?
        .ok_or(AtlasError::ProfileNotFound)?;
    Ok(prepare_saved(&p, vault))
}

fn prepare_saved(p: &StoredProfile, vault: &SecretVault) -> Prepared {
    match url_policy::check(&p.base_url, p.insecure_http_acknowledged) {
        Ok(endpoint) => Prepared::Ready {
            kind: p.kind,
            endpoint,
            key: vault.get(&account_for(&p.id), p.storage),
        },
        Err(problem) => blocked(problem),
    }
}

/// Resolves a saved or candidate profile to an endpoint and key without
/// any network I/O, for connection and capability tests alike.
pub fn resolve(
    root: &OpenRoot,
    vault: &SecretVault,
    target: &ConnectionTestTarget,
) -> AtlasResult<Prepared> {
    match target {
        ConnectionTestTarget::SavedProfile { profile_id } => {
            let p = load(root, profile_id)?.ok_or(AtlasError::ProfileNotFound)?;
            Ok(prepare_saved(&p, vault))
        }
        ConnectionTestTarget::Candidate { profile } => {
            let endpoint =
                match url_policy::check(&profile.base_url, profile.insecure_http_acknowledged) {
                    Ok(e) => e,
                    Err(problem) => return Ok(blocked(problem)),
                };
            let typed = profile
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(Secret::new);
            // Editing a saved profile without retyping its key: use the saved one.
            let key = match (typed, &profile.id) {
                (Some(k), _) => Some(k),
                (None, Some(id)) if !profile.clear_api_key => {
                    load(root, id)?.and_then(|p| vault.get(&account_for(&p.id), p.storage))
                }
                _ => None,
            };
            Ok(Prepared::Ready {
                kind: profile.kind,
                endpoint,
                key,
            })
        }
    }
}
