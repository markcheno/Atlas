//! Model recommendations (phase2-spec §7.1, ADR 0012): the catalog bundled
//! with this release (`recommendations.json`) crossed with the device check
//! and the model ids the user's services already listed. Pure: no network,
//! no downloads.

use serde::Deserialize;

use crate::domain::dto::{
    AiRole, DeviceSnapshot, ModelRecommendation, RecommendationKind, Recommendations,
    RoleRecommendations, RuntimeSteps,
};

const CATALOG_JSON: &str = include_str!("recommendations.json");
const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub version: u32,
    pub models: Vec<CatalogModel>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    pub id: String,
    pub label: String,
    pub role: AiRole,
    pub kind: RecommendationKind,
    pub size_bytes: u64,
    pub min_memory_gb: u32,
    pub dimensions: Option<u32>,
    pub why: String,
    #[serde(rename = "match")]
    pub matches: Vec<String>,
    pub runtimes: Vec<RuntimeSteps>,
}

/// The catalog bundled with this build.
pub fn bundled() -> Catalog {
    serde_json::from_str(CATALOG_JSON).expect("the bundled recommendation catalog is valid")
}

/// Lowercase letters and digits only, so `Qwen3-Embedding-0.6B-8bit`,
/// `qwen3-embedding:0.6b` and `text-embedding-qwen3-embedding-0.6b` compare
/// alike.
fn fold(id: &str) -> String {
    id.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The first listed id that is an installed copy of `model`.
fn installed_as(model: &CatalogModel, listed: &[String]) -> Option<String> {
    let wanted: Vec<String> = model.matches.iter().map(|m| fold(m)).collect();
    listed
        .iter()
        .find(|id| {
            let id = fold(id);
            wanted
                .iter()
                .any(|w| !w.is_empty() && id.contains(w.as_str()))
        })
        .cloned()
}

fn recommendation(model: &CatalogModel, listed: &[String]) -> ModelRecommendation {
    ModelRecommendation {
        id: model.id.clone(),
        label: model.label.clone(),
        kind: model.kind,
        size_bytes: model.size_bytes,
        min_memory_gb: model.min_memory_gb,
        dimensions: model.dimensions,
        why: model.why.clone(),
        runtimes: model.runtimes.clone(),
        installed_as: installed_as(model, listed),
    }
}

fn for_role(
    catalog: &Catalog,
    role: AiRole,
    memory_gb: Option<u32>,
    listed: &[String],
) -> RoleRecommendations {
    let of_role: Vec<&CatalogModel> = catalog.models.iter().filter(|m| m.role == role).collect();
    let fits = |m: &&CatalogModel| memory_gb.is_some_and(|gb| gb >= m.min_memory_gb);
    // The recommended entry when it fits; otherwise (or with memory
    // unknown) the smallest one for the role.
    let primary = of_role
        .iter()
        .find(|m| m.kind == RecommendationKind::Recommended && fits(m))
        .or_else(|| of_role.iter().min_by_key(|m| m.min_memory_gb))
        .copied();
    let mut models = Vec::new();
    if let Some(primary) = primary {
        models.push(recommendation(primary, listed));
        for kind in [
            RecommendationKind::Faster,
            RecommendationKind::HigherQuality,
        ] {
            if let Some(alt) = of_role
                .iter()
                .find(|m| m.kind == kind && m.id != primary.id && fits(m))
            {
                models.push(recommendation(alt, listed));
            }
        }
    }
    RoleRecommendations { role, models }
}

/// Recommendations for this computer. `listed` are model ids the user's
/// services listed in this session's tests.
pub fn recommend(catalog: &Catalog, device: &DeviceSnapshot, listed: &[String]) -> Recommendations {
    // Rounded to the nearest GiB, so a "16 GB" Mac reporting slightly less
    // still counts as 16.
    let memory_gb = device
        .total_memory_bytes
        .map(|b| u32::try_from((b + GIB / 2) / GIB).unwrap_or(u32::MAX));
    Recommendations {
        catalog_version: catalog.version,
        total_memory_gb: memory_gb,
        embedding: for_role(catalog, AiRole::Embedding, memory_gb, listed),
        chat: for_role(catalog, AiRole::Chat, memory_gb, listed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::dto::{AiRuntime, OsKind, PowerSource, PressureLevel, ThermalLevel};

    fn device(gb: Option<u64>) -> DeviceSnapshot {
        DeviceSnapshot {
            schema_version: 1,
            os: OsKind::Macos,
            arch: "aarch64".into(),
            logical_cores: Some(8),
            total_memory_bytes: gb.map(|g| g * GIB),
            available_memory_bytes: None,
            data_volume_reachable: None,
            data_volume_free_bytes: None,
            data_volume_total_bytes: None,
            data_volume_kind: None,
            required_free_bytes: None,
            power: PowerSource::Ac,
            battery_present: None,
            battery_percent: None,
            charging: None,
            memory_pressure: PressureLevel::Normal,
            thermal: ThermalLevel::Nominal,
            sampled_at: "t".into(),
        }
    }

    fn ids(r: &RoleRecommendations) -> Vec<&str> {
        r.models.iter().map(|m| m.id.as_str()).collect()
    }

    /// P2-03: the bundled catalog crossed with memory sizes.
    #[test]
    fn p2_03_recommendations_follow_memory() {
        let c = bundled();
        let r = recommend(&c, &device(Some(8)), &[]);
        assert_eq!(ids(&r.embedding), ["qwen3-embedding-0.6b"]);
        assert_eq!(ids(&r.chat), ["qwen3.5-9b"], "the smallest when none fits");
        let r = recommend(&c, &device(Some(16)), &[]);
        assert_eq!(
            ids(&r.embedding),
            ["qwen3-embedding-0.6b", "qwen3-embedding-4b"]
        );
        assert_eq!(ids(&r.chat), ["qwen3.5-9b"]);
        let r = recommend(&c, &device(Some(64)), &[]);
        assert_eq!(ids(&r.chat), ["qwen3.5-9b", "qwen3.5-27b"]);
        assert_eq!(r.total_memory_gb, Some(64));
        let r = recommend(&c, &device(None), &[]);
        assert_eq!(ids(&r.embedding), ["qwen3-embedding-0.6b"]);
        assert_eq!(ids(&r.chat), ["qwen3.5-9b"]);
        assert_eq!(r.total_memory_gb, None);
    }

    #[test]
    fn installed_copies_are_found_whatever_the_spelling() {
        let c = bundled();
        let listed: Vec<String> = [
            "Qwen3.8-27B-MLX-4bit",
            "mlx-community/Qwen3-Embedding-0.6B-8bit",
            "qwen3.5:9b",
        ]
        .map(String::from)
        .to_vec();
        let r = recommend(&c, &device(Some(64)), &listed);
        assert_eq!(
            r.embedding.models[0].installed_as.as_deref(),
            Some("mlx-community/Qwen3-Embedding-0.6B-8bit")
        );
        assert_eq!(
            r.embedding.models[1].installed_as, None,
            "4B isn't installed"
        );
        assert_eq!(r.chat.models[0].installed_as.as_deref(), Some("qwen3.5:9b"));
        assert_eq!(
            r.chat.models[1].installed_as, None,
            "Qwen3.8 27B is not Qwen3.5 27B"
        );
        let lmstudio = ["text-embedding-qwen3-embedding-0.6b".to_owned()];
        let r = recommend(&c, &device(Some(16)), &lmstudio);
        assert!(r.embedding.models[0].installed_as.is_some());
    }

    #[test]
    fn every_entry_has_steps_for_every_runtime() {
        for m in bundled().models {
            let runtimes: Vec<AiRuntime> = m.runtimes.iter().map(|r| r.runtime).collect();
            assert_eq!(
                runtimes,
                [AiRuntime::Omlx, AiRuntime::Ollama, AiRuntime::Lmstudio],
                "{}",
                m.id
            );
            for r in &m.runtimes {
                assert!(!r.steps.is_empty() && !r.model_id.is_empty(), "{}", m.id);
                assert!(
                    r.steps
                        .iter()
                        .all(|s| !s.to_lowercase().contains("atlas will download"))
                );
            }
            assert!(!m.matches.is_empty(), "{}", m.id);
            assert_eq!(
                m.dimensions.is_some(),
                m.role == AiRole::Embedding,
                "{}",
                m.id
            );
        }
    }
}
