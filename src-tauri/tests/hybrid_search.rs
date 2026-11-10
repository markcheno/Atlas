//! Slice 2.4: hybrid search (phase2-spec §7.4).
//!
//! - P2-10: a question that shares no words with a document still finds it
//!   by meaning; a quoted phrase finds by text; a document found both ways
//!   says so and ranks first; folder and type filters apply to meaning
//!   matches; nonsense finds nothing.
//! - P2-11: without a model, with the service down, slow or not allowed,
//!   or with the index still loading, the page is exactly Phase 1's plus a
//!   status, never an error.

use std::time::Duration;

use atlas_lib::domain::dto::{
    FileType, MatchReason, SearchPage, SemanticStatus, SemanticUnavailable,
};
use atlas_lib::services::embedding::models;
use atlas_lib::services::search;

mod common;
use common::meaning::{LONG, Lib};
use common::wait_until;

fn ids(page: &SearchPage) -> Vec<(Option<String>, MatchReason)> {
    page.hits
        .iter()
        .map(|h| (h.document_id.clone(), h.why))
        .collect()
}

#[test]
fn p2_10_meaning_finds_what_the_words_miss_and_fusion_ranks_both_first() {
    let lib = Lib::new();

    // Shares no word with the document: found by meaning only.
    lib.alias_to_sky("interstellar odyssey");
    let page = lib.hybrid(&Lib::query("interstellar odyssey"));
    assert!(matches!(
        page.semantic,
        SemanticStatus::Ready {
            coverage_percent: 100
        }
    ));
    assert!(
        lib.text_only(&Lib::query("interstellar odyssey"))
            .hits
            .is_empty()
    );
    let first = &page.hits[0];
    assert_eq!(first.document_id.as_deref(), Some(lib.sky.as_str()));
    assert_eq!(first.why, MatchReason::Semantic);
    assert!(first.all_words, "never 'some of your words'");
    let snippet: String = first.snippet.iter().map(|s| s.text.as_str()).collect();
    assert!(snippet.contains("starship"), "{snippet}");
    assert!(page.hits.len() <= search::hybrid::MAX_MEANING_ONLY);

    // Found both ways: says so, and comes first.
    lib.alias_to_sky("distant comets");
    let page = lib.hybrid(&Lib::query("distant comets"));
    assert_eq!(page.hits[0].document_id.as_deref(), Some(lib.sky.as_str()));
    assert_eq!(page.hits[0].why, MatchReason::TextSemantic);

    // A quoted phrase finds its document by text.
    let page = lib.hybrid(&Lib::query("\"sourdough bread\""));
    assert!(matches!(
        page.hits[0].why,
        MatchReason::Text | MatchReason::TextSemantic
    ));
    let bread: String = page.hits[0]
        .snippet
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert!(bread.to_lowercase().contains("sourdough"));

    // Filters apply to meaning matches: the other file type excludes it.
    let mut other = Lib::query("interstellar odyssey");
    other.file_type = Some(match lib.sky_type {
        FileType::Pdf => FileType::Epub,
        FileType::Epub => FileType::Pdf,
    });
    let page = lib.hybrid(&other);
    assert!(
        page.hits
            .iter()
            .all(|h| h.document_id.as_deref() != Some(lib.sky.as_str())),
        "{:?}",
        ids(&page)
    );
    let mut same = Lib::query("interstellar odyssey");
    same.file_type = Some(lib.sky_type);
    assert_eq!(
        lib.hybrid(&same).hits[0].document_id.as_deref(),
        Some(lib.sky.as_str())
    );
    let mut nowhere = Lib::query("interstellar odyssey");
    nowhere.source_root_id = Some("no-such-folder".into());
    assert!(lib.hybrid(&nowhere).hits.is_empty());

    // Copies are one result.
    let page = lib.hybrid(&Lib::query("book"));
    let docs: Vec<_> = page
        .hits
        .iter()
        .filter_map(|h| h.document_id.clone())
        .collect();
    let unique: std::collections::BTreeSet<_> = docs.iter().collect();
    assert_eq!(docs.len(), unique.len());

    // Nonsense still finds nothing.
    let page = lib.hybrid(&Lib::query("zqxjv plorbix"));
    assert!(page.hits.is_empty(), "{:?}", ids(&page));

    // A repeated question is not sent again.
    let calls = lib.h.script.calls();
    lib.hybrid(&Lib::query("interstellar odyssey"));
    assert_eq!(lib.h.script.calls(), calls, "answered from the cache");
}

#[test]
fn p2_11_without_meaning_the_page_is_phase_1s_plus_a_status() {
    let lib = Lib::new();
    let q = Lib::query("comets bread");
    let phase1 = ids(&lib.text_only(&q));
    assert!(!phase1.is_empty());

    // The service is down.
    lib.h
        .script
        .with(|s| s.fail_always = Some(atlas_lib::domain::traits::EmbedError::Network));
    let page = lib.hybrid(&Lib::query("comets bread"));
    assert_eq!(
        page.semantic,
        SemanticStatus::Unavailable {
            reason: SemanticUnavailable::Unreachable
        }
    );
    assert_eq!(ids(&page), phase1);

    // Too slow: Phase 1's page now, and the answer fills the cache.
    lib.h.script.with(|s| {
        s.fail_always = None;
        s.delay = Duration::from_millis(2_500);
    });
    let page = lib.hybrid(&Lib::query("comets bread"));
    assert_eq!(
        page.semantic,
        SemanticStatus::Unavailable {
            reason: SemanticUnavailable::Timeout
        }
    );
    assert_eq!(ids(&page), phase1);
    std::thread::sleep(Duration::from_millis(1_500));
    let page = lib.hybrid(&Lib::query("comets bread"));
    assert!(
        matches!(page.semantic, SemanticStatus::Ready { .. }),
        "{:?}",
        page.semantic
    );
    lib.h.script.with(|s| s.delay = Duration::ZERO);

    // The index isn't open yet: it opens in the background.
    let model = models::serving(&lib.catalog()).unwrap().unwrap();
    lib.root().vectors().forget(&model.id);
    let page = lib.hybrid(&Lib::query("comets bread"));
    assert_eq!(
        page.semantic,
        SemanticStatus::Unavailable {
            reason: SemanticUnavailable::Loading
        }
    );
    assert_eq!(ids(&page), phase1);
    assert!(wait_until(LONG, || lib
        .root()
        .vectors()
        .loaded(&model.id)
        .is_some()));
    assert!(matches!(
        lib.hybrid(&Lib::query("comets bread")).semantic,
        SemanticStatus::Ready { .. }
    ));

    // An internet service without permission: nothing is sent.
    lib.set_role("https://embeddings.invalid/v1");
    let calls = lib.h.script.calls();
    let page = lib.hybrid(&Lib::query("a fresh question"));
    assert_eq!(
        page.semantic,
        SemanticStatus::Unavailable {
            reason: SemanticUnavailable::NotPermitted
        }
    );
    assert_eq!(lib.h.script.calls(), calls);

    // No embedding model at all: Off.
    lib.root()
        .catalog()
        .call(|c| c.execute_batch("DELETE FROM model_roles"))
        .unwrap()
        .unwrap();
    let page = lib.hybrid(&Lib::query("comets bread"));
    assert_eq!(page.semantic, SemanticStatus::Off);
    assert_eq!(ids(&page), phase1);
}
