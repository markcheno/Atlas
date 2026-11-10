//! What the embedding job's status line, the per-folder lines and Home
//! show (phase2-spec §5.2): files with text, by where they stand for the
//! active model.

use rusqlite::{Connection, OptionalExtension};

use super::models;
use crate::domain::dto::{
    EmbeddingCounts, EmbeddingModelInfo, EmbeddingOverview, EmbeddingStatus, JobState,
    ProviderLocation, RootEmbeddingSummary,
};
use crate::services::catalog::codec::from_db;
use crate::services::providers::url_policy;

/// Counts per folder for `model_id` (or every file waiting, with none).
fn root_counts(
    conn: &Connection,
    model_id: Option<&str>,
) -> rusqlite::Result<Vec<(String, EmbeddingCounts)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT f.source_root_id,
                SUM(CASE WHEN de.status IN ('done', 'empty') THEN 1 ELSE 0 END),
                SUM(CASE WHEN de.status = 'failed' AND de.attempts >= 3 THEN 1 ELSE 0 END),
                COUNT(*)
         FROM file_instances f
         JOIN documents d ON d.id = f.document_id AND d.state = 'done'
         LEFT JOIN document_embeddings de ON de.document_id = d.id AND de.model_id = ?1
         WHERE f.availability = 'available'
         GROUP BY f.source_root_id",
    )?;
    stmt.query_map([model_id.unwrap_or("")], |r| {
        let (indexed, failed, total): (i64, i64, i64) = (r.get(1)?, r.get(2)?, r.get(3)?);
        Ok((
            r.get::<_, String>(0)?,
            EmbeddingCounts {
                indexed: indexed as u32,
                failed: failed as u32,
                waiting: (total - indexed - failed).max(0) as u32,
                total: total as u32,
            },
        ))
    })?
    .collect()
}

fn add(a: &mut EmbeddingCounts, b: &EmbeddingCounts) {
    a.indexed += b.indexed;
    a.waiting += b.waiting;
    a.failed += b.failed;
    a.total += b.total;
}

/// The embedding role's service: its label and place.
fn role_service(conn: &Connection) -> rusqlite::Result<Option<(String, ProviderLocation)>> {
    conn.query_row(
        "SELECT p.label, p.base_url FROM model_roles r
         JOIN provider_profiles p ON p.id = r.provider_profile_id
         WHERE r.role = 'embedding'",
        [],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                url_policy::location_of(&r.get::<_, String>(1)?),
            ))
        },
    )
    .optional()
}

/// Whether embedding may go through the role's service: always for this
/// computer or another one; for an internet service, only when allowed.
pub fn permitted(conn: &Connection, allow_remote: bool) -> rusqlite::Result<bool> {
    Ok(match role_service(conn)? {
        Some((_, ProviderLocation::Internet)) => allow_remote,
        _ => true,
    })
}

fn active_job(conn: &Connection) -> rusqlite::Result<Option<EmbeddingStatus>> {
    conn.query_row(
        "SELECT j.id, j.source_root_id, r.display_path, j.state, j.pause_reason,
                j.throttle_reason, j.last_error, j.embedded_chunks, j.sent_chars, j.service_ms
         FROM jobs j JOIN source_roots r ON r.id = j.source_root_id
         WHERE j.kind = 'embed'
           AND j.state IN ('running', 'paused_policy', 'paused_user', 'failed_retryable', 'queued')
         ORDER BY CASE j.state WHEN 'running' THEN 0 WHEN 'paused_policy' THEN 1
                  WHEN 'paused_user' THEN 2 WHEN 'failed_retryable' THEN 3 ELSE 4 END,
                  j.created_at, j.rowid
         LIMIT 1",
        [],
        |r| {
            let text = |i: usize| r.get::<_, Option<String>>(i);
            Ok(EmbeddingStatus {
                job_id: r.get(0)?,
                source_root_id: r.get(1)?,
                source_root_display_path: r.get(2)?,
                state: from_db(&r.get::<_, String>(3)?).unwrap_or(JobState::Queued),
                pause_reason: text(4)?.and_then(|s| from_db(&s)),
                throttle_reason: text(5)?.and_then(|s| from_db(&s)),
                last_error: text(6)?,
                embedded_chunks: r.get::<_, i64>(7)? as u64,
                sent_chars: r.get::<_, i64>(8)? as u64,
                service_ms: r.get::<_, i64>(9)? as u64,
            })
        },
    )
    .optional()
}

/// Everything the status lines show. `allow_remote` is the library's
/// privacy setting.
pub fn overview(conn: &Connection, allow_remote: bool) -> rusqlite::Result<EmbeddingOverview> {
    let role = models::role(conn)?;
    let active = models::active(conn)?;
    let rebuilding = match &active {
        Some(m) if m.status == "building" => models::predecessor(conn, &m.id)?.is_some(),
        _ => false,
    };
    let service = role_service(conn)?;
    let model = active.as_ref().map(|m| EmbeddingModelInfo {
        name: m.model_name.clone(),
        dims: m.dims as u32,
        profile_label: service.as_ref().map(|(l, _)| l.clone()),
        location: service.as_ref().map(|(_, loc)| *loc),
    });
    let mut counts = EmbeddingCounts::default();
    let jobs: Vec<(String, String)> = conn
        .prepare_cached(
            "SELECT source_root_id, id FROM jobs WHERE kind = 'embed'
             AND state IN ('running', 'paused_policy', 'paused_user', 'failed_retryable', 'queued')",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let roots = root_counts(conn, active.as_ref().map(|m| m.id.as_str()))?
        .into_iter()
        .map(|(root, c)| {
            add(&mut counts, &c);
            RootEmbeddingSummary {
                active_job_id: jobs
                    .iter()
                    .find(|(r, _)| *r == root)
                    .map(|(_, j)| j.clone()),
                source_root_id: root,
                counts: c,
            }
        })
        .collect();
    let queued_jobs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM jobs WHERE kind = 'embed' AND state = 'queued'",
        [],
        |r| r.get(0),
    )?;
    let reading: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM jobs WHERE kind = 'extract'
                        AND state IN ('running', 'paused_policy', 'paused_user',
                                      'failed_retryable', 'queued'))",
        [],
        |r| r.get(0),
    )?;
    Ok(EmbeddingOverview {
        active: active_job(conn)?,
        queued_jobs: queued_jobs as u32,
        counts,
        model,
        rebuilding,
        needs_model: role.is_none(),
        has_ai_service: conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM provider_profiles)",
            [],
            |r| r.get(0),
        )?,
        needs_permission: !permitted(conn, allow_remote)?,
        roots,
        reading,
    })
}
