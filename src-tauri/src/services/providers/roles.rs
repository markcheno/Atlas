//! Which saved AI service and model does each job (phase2-spec §7.1, ADR
//! 0012). A role is assigned only with a recent passing capability test of
//! that model; deleting the service clears its roles (foreign key).

use rusqlite::{OptionalExtension, Row, params};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::url_policy;
use crate::domain::dto::{AiRole, CapabilityOutcome, CapabilityResult, ModelRole, ModelRoles};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::now_rfc3339;
use crate::infrastructure::http_probe::valid_model_id;
use crate::services::catalog::codec::{from_db, to_db};
use crate::services::workspace::OpenRoot;

/// How long a passing capability test can justify assigning a role.
pub const TEST_VALID_FOR: time::Duration = time::Duration::hours(24);

fn from_row(r: &Row) -> rusqlite::Result<Option<ModelRole>> {
    let role: String = r.get(0)?;
    let base_url: String = r.get(3)?;
    let dimensions: Option<i64> = r.get(5)?;
    Ok(from_db::<AiRole>(&role).map(|role| ModelRole {
        role,
        profile_id: r.get(1).unwrap_or_default(),
        profile_label: r.get(2).unwrap_or_default(),
        location: url_policy::location_of(&base_url),
        model_id: r.get(4).unwrap_or_default(),
        dimensions: dimensions.and_then(|d| u32::try_from(d).ok()),
        tested_at: r.get(6).unwrap_or_default(),
    }))
}

pub fn get(root: &OpenRoot) -> AtlasResult<ModelRoles> {
    let rows = root
        .reader()
        .read(|c| -> rusqlite::Result<Vec<Option<ModelRole>>> {
            c.prepare(
                "SELECT r.role, r.provider_profile_id, p.label, p.base_url, r.model_id,
                        r.dimensions, r.tested_at
                 FROM model_roles r JOIN provider_profiles p ON p.id = r.provider_profile_id",
            )?
            .query_map([], from_row)?
            .collect()
        })?
        .map_err(AtlasError::db("read the model roles"))?;
    let mut roles = ModelRoles::default();
    for role in rows.into_iter().flatten() {
        match role.role {
            AiRole::Chat => roles.chat = Some(role),
            AiRole::Embedding => roles.embedding = Some(role),
        }
    }
    Ok(roles)
}

/// Whether `result` justifies giving `model` the `role` now: a passing test
/// of that model for that role, at most [`TEST_VALID_FOR`] old.
pub fn justifies(
    result: &CapabilityResult,
    role: AiRole,
    model: &str,
    now: OffsetDateTime,
) -> bool {
    let recent = OffsetDateTime::parse(&result.tested_at, &Rfc3339)
        .is_ok_and(|t| t <= now + time::Duration::minutes(5) && now - t <= TEST_VALID_FOR);
    result.outcome == CapabilityOutcome::Works
        && result.role == role
        && result.model_id == model
        && (role == AiRole::Chat || result.dimensions.is_some())
        && recent
}

/// Assigns `role` to `model` on the saved service `profile_id`.
pub fn set(
    root: &OpenRoot,
    role: AiRole,
    profile_id: &str,
    model_id: &str,
    result: &CapabilityResult,
) -> AtlasResult<ModelRoles> {
    let model = valid_model_id(model_id).ok_or(AtlasError::ModelNotTested)?;
    if !justifies(result, role, model, OffsetDateTime::now_utc()) {
        return Err(AtlasError::ModelNotTested);
    }
    let id = profile_id.to_owned();
    let exists = root
        .reader()
        .read(move |c| {
            c.query_row(
                "SELECT 1 FROM provider_profiles WHERE id = ?1",
                [id],
                |_| Ok(()),
            )
            .optional()
        })?
        .map_err(AtlasError::db("read AI services"))?
        .is_some();
    if !exists {
        return Err(AtlasError::ProfileNotFound);
    }
    let row = (
        to_db(&role),
        profile_id.to_owned(),
        model.to_owned(),
        result.dimensions.map(i64::from),
        result.tested_at.clone(),
        now_rfc3339(),
    );
    root.catalog()
        .call(move |c| {
            c.execute(
                "INSERT INTO model_roles (role, provider_profile_id, model_id, dimensions,
                     tested_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (role) DO UPDATE SET
                     provider_profile_id = excluded.provider_profile_id,
                     model_id = excluded.model_id, dimensions = excluded.dimensions,
                     tested_at = excluded.tested_at, updated_at = excluded.updated_at",
                params![row.0, row.1, row.2, row.3, row.4, row.5],
            )
        })?
        .map_err(AtlasError::db("save the model role"))?;
    get(root)
}

pub fn clear(root: &OpenRoot, role: AiRole) -> AtlasResult<ModelRoles> {
    let role = to_db(&role);
    root.catalog()
        .call(move |c| c.execute("DELETE FROM model_roles WHERE role = ?1", [role]))?
        .map_err(AtlasError::db("clear the model role"))?;
    get(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passed(role: AiRole, model: &str, at: OffsetDateTime) -> CapabilityResult {
        CapabilityResult {
            role,
            model_id: model.into(),
            outcome: CapabilityOutcome::Works,
            dimensions: (role == AiRole::Embedding).then_some(1024),
            message: String::new(),
            elapsed_ms: 10,
            tested_at: at.format(&Rfc3339).unwrap(),
        }
    }

    #[test]
    fn only_a_recent_matching_pass_justifies_a_role() {
        let now = OffsetDateTime::now_utc();
        let e = AiRole::Embedding;
        assert!(justifies(&passed(e, "m", now), e, "m", now));
        assert!(
            !justifies(&passed(e, "m", now), AiRole::Chat, "m", now),
            "other role"
        );
        assert!(
            !justifies(&passed(e, "m", now), e, "other", now),
            "other model"
        );
        let stale = passed(e, "m", now - time::Duration::hours(25));
        assert!(!justifies(&stale, e, "m", now));
        let future = passed(e, "m", now + time::Duration::hours(1));
        assert!(!justifies(&future, e, "m", now));
        let mut failed = passed(e, "m", now);
        failed.outcome = CapabilityOutcome::WrongRole;
        assert!(!justifies(&failed, e, "m", now));
        let mut no_dims = passed(e, "m", now);
        no_dims.dimensions = None;
        assert!(
            !justifies(&no_dims, e, "m", now),
            "an embedding pass reports dimensions"
        );
        assert!(justifies(
            &passed(AiRole::Chat, "c", now),
            AiRole::Chat,
            "c",
            now
        ));
    }
}
