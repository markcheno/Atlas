//! Slice 1.6: Settings › Storage (phase1-spec §5.5, §8). Usage by category
//! from the files and `dbstat`; the indexing estimate only after 50
//! documents and always as a range; Clean caches empties `cache/` and
//! `tmp/` only, never following links. Runs the real worker.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use atlas_lib::domain::dto::IndexingEstimate;
use atlas_lib::services::extraction::db;
use atlas_lib::services::workspace::Workspace;
use atlas_lib::services::{inventory, sources, storage};

mod common;
use common::{Env, ExtractionHarness, fast_extraction, local, wait_until, worker};

const LONG: Duration = Duration::from_secs(180);

fn generate(dir: &Path, files: &str) {
    let out = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
        .arg(dir)
        .args(["--files", files, "--text-kb", "4"])
        .output()
        .expect("node runs the fixture generator");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn settled(ws: &Workspace) -> bool {
    let root = ws.require_open().unwrap();
    let inv = inventory::control::overview(root).unwrap();
    let ov = root
        .reader()
        .read(|c| db::overview(c, None))
        .unwrap()
        .unwrap();
    inv.active.is_none() && ov.active.is_none() && ov.queued_jobs == 0 && ov.counts.pending == 0
}

fn data(env: &Env) -> PathBuf {
    env.base.join("AtlasData")
}

/// Reads a generated library of `files` text files.
fn read_library(env: &Env, files: &str) -> (Workspace, ExtractionHarness) {
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_extraction(&h);
    ws.choose(&data(env)).unwrap();
    let books = env.base.join("Books");
    generate(&books, files);
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    assert!(wait_until(LONG, || settled(&ws)), "never settled");
    (ws, h)
}

#[test]
fn usage_is_measured_by_category() {
    let env = Env::new();
    let (ws, _h) = read_library(&env, "6");
    let usage = storage::usage(&data(&env), local().as_ref()).unwrap();
    assert!(usage.text_bytes > 0 && usage.index_bytes > 0, "{usage:?}");
    assert!(usage.other_catalog_bytes > 0, "{usage:?}");
    assert_eq!(
        usage.catalog_bytes,
        usage.text_bytes + usage.index_bytes + usage.other_catalog_bytes
    );
    let on_disk: u64 = ["", "-wal", "-shm"]
        .iter()
        .map(|s| {
            fs::metadata(data(&env).join(format!("library/catalog.sqlite{s}")))
                .map(|m| m.len())
                .unwrap_or(0)
        })
        .sum();
    assert_eq!(usage.catalog_bytes, on_disk);
    assert!(usage.data_volume_free_bytes.is_some());
    assert_eq!(usage.estimate, IndexingEstimate::AllRead);
    drop(ws);
}

#[test]
fn the_estimate_waits_for_fifty_documents_then_gives_a_range() {
    let env = Env::new();
    // Cataloged, nothing read: not enough to estimate.
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_inventory(h.inv.deps.clone());
    ws.choose(&data(&env)).unwrap();
    let first = env.base.join("First");
    generate(&first, "6");
    let root = ws.require_open().unwrap();
    let id = sources::add(root, &first).unwrap().id;
    inventory::control::enqueue(root, &id).unwrap();
    assert!(wait_until(LONG, || inventory::control::overview(
        ws.require_open().unwrap()
    )
    .unwrap()
    .active
    .is_none()));
    let usage = storage::usage(&data(&env), local().as_ref()).unwrap();
    assert_eq!(
        usage.estimate,
        IndexingEstimate::NotEnoughRead {
            done: 0,
            needed: 50
        }
    );
    drop(ws);

    // 66 files read (61 documents: the six Introduction copies are one),
    // then a new folder cataloged but not read yet.
    let env2 = Env::new();
    let (ws, _h) = read_library(&env2, "60");
    drop(ws);
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env2.workspace_with_inventory(h.inv.deps.clone());
    ws.startup();
    let more = env2.base.join("More");
    generate(&more, "6");
    let root = ws.require_open().unwrap();
    let id = sources::add(root, &more).unwrap().id;
    inventory::control::enqueue(root, &id).unwrap();
    assert!(wait_until(LONG, || inventory::control::overview(
        ws.require_open().unwrap()
    )
    .unwrap()
    .active
    .is_none()));
    let usage = storage::usage(&data(&env2), local().as_ref()).unwrap();
    let IndexingEstimate::Range {
        remaining_files,
        low_bytes,
        high_bytes,
    } = usage.estimate
    else {
        panic!("{:?}", usage.estimate)
    };
    assert_eq!(remaining_files, 12, "the new folder's files");
    let per_document = (usage.text_bytes + usage.index_bytes) as f64 / 61.0;
    let expected = per_document * 12.0;
    assert!(
        (low_bytes as f64 - expected * 0.6).abs() < 2.0
            && (high_bytes as f64 - expected * 1.4).abs() < 2.0,
        "{low_bytes}–{high_bytes} around {expected}"
    );
}

#[test]
fn clean_caches_empties_only_the_caches() {
    let env = Env::new();
    let (ws, _h) = read_library(&env, "6");
    let root = data(&env);
    fs::write(root.join("cache/previews/a.bin"), vec![1u8; 1_000]).unwrap();
    fs::create_dir_all(root.join("cache/old/deeper")).unwrap();
    fs::write(root.join("cache/old/deeper/b.bin"), vec![2u8; 200]).unwrap();
    fs::write(root.join("tmp/stale.part"), vec![3u8; 500]).unwrap();
    let outside = env.base.join("precious.txt");
    fs::write(&outside, b"never removed").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("cache/link.txt")).unwrap();
    // A link counts as its own few bytes, never as what it points to.
    let link_bytes = fs::symlink_metadata(root.join("cache/link.txt"))
        .map(|m| m.len())
        .unwrap_or(0);
    let keep = |p: &str| fs::read(root.join(p)).unwrap();
    let (manifest, settings) = (keep("atlas.json"), keep("config/settings.json"));
    let before = storage::usage(&root, local().as_ref()).unwrap();
    assert_eq!(before.cache_bytes, 1_700 + link_bytes);

    let freed = storage::clean_caches(&root).unwrap();
    assert_eq!(freed, 1_700 + link_bytes);
    let after = storage::usage(&root, local().as_ref()).unwrap();
    assert_eq!(after.cache_bytes, 0);
    assert_eq!(after.text_bytes, before.text_bytes, "the text is kept");
    assert!(
        root.join("cache/previews").is_dir(),
        "the library's own folders stay"
    );
    assert!(!root.join("cache/old").exists());
    assert!(root.join("tmp").is_dir());
    assert_eq!(fs::read(&outside).unwrap(), b"never removed");
    assert_eq!(
        (keep("atlas.json"), keep("config/settings.json")),
        (manifest, settings)
    );
    assert_eq!(
        storage::clean_caches(&root).unwrap(),
        0,
        "nothing left to clean"
    );
    drop(ws);
}
