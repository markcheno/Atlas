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
    /// Embedding models: the instruction put before a search query (never
    /// before document text), per the model's own instructions.
    #[serde(default, rename = "queryPrefix")]
    pub query_prefix: Option<String>,
    /// Embedding models: cosine similarity below which a chunk is not a
    /// meaning match (calibrated with the evaluation set, Slice 2.4).
    #[serde(default, rename = "minSimilarity")]
    pub min_similarity: Option<f32>,
    /// Embedding models that may be cut short (Matryoshka): the number of
    /// dimensions Atlas keeps, decided by the evaluation (Slice 2.4).
    #[serde(default, rename = "storedDimensions")]
    pub stored_dimensions: Option<usize>,
    /// Embedding models: how far above `min_similarity` the best meaning
    /// match must be for Ask Atlas's retrieval not to count as weak
    /// (Slice 3.1; tuned for Qwen3-Embedding-0.6B in Slice 2.6.3).
    #[serde(default, rename = "weakMargin")]
    pub weak_margin: Option<f32>,
}

/// How search treats queries for an embedding model.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryProfile {
    pub prefix: Option<String>,
    pub min_similarity: f32,
    /// How many of the model's dimensions Atlas keeps, if fewer than all.
    pub stored_dimensions: Option<usize>,
    /// How far above `min_similarity` the best match must be for Ask
    /// Atlas's retrieval not to count as weak.
    pub weak_margin: f32,
}

/// The floor for embedding models the catalog doesn't know.
pub const DEFAULT_MIN_SIMILARITY: f32 = 0.25;
/// The weak margin for embedding models the catalog doesn't give one
/// (Qwen3-Embedding-0.6B's, Slice 2.6.3).
pub const DEFAULT_WEAK_MARGIN: f32 = 0.20;

/// The query profile for `model_name`, from the bundled catalog when the
/// name matches one of its embedding models (as installed copies match).
pub fn query_profile(model_name: &str) -> QueryProfile {
    let catalog = bundled();
    let listed = [model_name.to_owned()];
    catalog
        .models
        .iter()
        .find(|m| m.role == AiRole::Embedding && installed_as(m, &listed).is_some())
        .map(|m| QueryProfile {
            prefix: m.query_prefix.clone(),
            min_similarity: m.min_similarity.unwrap_or(DEFAULT_MIN_SIMILARITY),
            stored_dimensions: m.stored_dimensions,
            weak_margin: m.weak_margin.unwrap_or(DEFAULT_WEAK_MARGIN),
        })
        .unwrap_or(QueryProfile {
            prefix: None,
            min_similarity: DEFAULT_MIN_SIMILARITY,
            stored_dimensions: None,
            weak_margin: DEFAULT_WEAK_MARGIN,
        })
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

    #[test]
    fn qwen3_embedding_queries_get_their_instruction() {
        for name in [
            "Qwen3-Embedding-0.6B-8bit",
            "qwen3-embedding:0.6b",
            "mlx-community/Qwen3-Embedding-4B",
        ] {
            let p = query_profile(name);
            assert!(
                p.prefix
                    .as_deref()
                    .is_some_and(|x| x.starts_with("Instruct: ")),
                "{name}"
            );
            assert!(p.prefix.unwrap().ends_with("\nQuery: "));
        }
        assert_eq!(
            query_profile("Qwen3-Embedding-0.6B-8bit").stored_dimensions,
            Some(512)
        );
        let other = query_profile("nomic-embed-text");
        assert_eq!(other.prefix, None);
        assert_eq!(other.stored_dimensions, None);
        assert_eq!(other.min_similarity, DEFAULT_MIN_SIMILARITY);
    }

    /// P3-02: the weak margin is the model's, from the catalog; a model the
    /// catalog doesn't know gets the default.
    #[test]
    fn p3_02_the_weak_margin_comes_from_the_model() {
        for name in ["Qwen3-Embedding-0.6B-8bit", "qwen3-embedding:4b"] {
            assert_eq!(query_profile(name).weak_margin, 0.20, "{name}");
        }
        assert_eq!(
            query_profile("nomic-embed-text").weak_margin,
            DEFAULT_WEAK_MARGIN
        );
        // Every embedding entry of the catalog gives its own.
        for m in bundled()
            .models
            .iter()
            .filter(|m| m.role == AiRole::Embedding)
        {
            let margin = m
                .weak_margin
                .unwrap_or_else(|| panic!("{} has no weakMargin", m.id));
            assert!((0.0..0.5).contains(&margin), "{}: {margin}", m.id);
        }
    }
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
