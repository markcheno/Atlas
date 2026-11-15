//! Slice 4.6.1: Explore in the app (ADR 0024; P4-31 to P4-34). Book
//! vectors kept as books are embedded; the map drawn in the background
//! from them, kept stable across redraws; what the window reads; the
//! Library's area filter; the owner's names.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use atlas_lib::domain::dto::{
    BackgroundMode, DocumentQuery, DocumentSort, ExploreState, PauseReason, PowerSource,
};
use atlas_lib::services::catalog::reader::CatalogReader;
use atlas_lib::services::documents::related;
use atlas_lib::services::embedding::db as edb;
use atlas_lib::services::embedding::fake::HashedBagOfWordsBackend;
use atlas_lib::services::embedding::triggers;
use atlas_lib::services::embedding::vector::from_f16_blob;
use atlas_lib::services::explore::app;
use atlas_lib::services::explore::build::{self, Outcome};
use atlas_lib::services::workspace::{OpenRoot, Workspace};
use atlas_lib::services::{inventory, library, sources};
use rusqlite::{Connection, params};

mod common;
use common::embedding::{EmbedHarness, fast_embedding};
use common::vectors::Lib;
use common::{Env, wait_until};

const DIMS: usize = 64;
const LONG: Duration = Duration::from_secs(180);

const TOPICS: [[&str; 6]; 3] = [
    ["orbit", "telescope", "comet", "galaxy", "nebula", "planet"],
    ["bread", "yeast", "flour", "oven", "crust", "dough"],
    ["violin", "sonata", "tempo", "chord", "melody", "rhythm"],
];
const FILLER: [&str; 6] = ["notes", "study", "history", "method", "theory", "practice"];

/// A page of a book on `topic`, as the spike's tests write them.
fn page(topic: usize, book: usize, page: usize) -> String {
    let mut x = (topic * 1_000 + book * 10 + page) as u64 * 2_654_435_761 + 1;
    let mut out = String::new();
    for i in 0..160 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let word = match x % 10 {
            0..=6 => TOPICS[topic][(x >> 8) as usize % 6],
            7..=8 => FILLER[(x >> 8) as usize % 6],
            _ => &format!("own{topic}x{book}"),
        };
        out.push_str(word);
        out.push_str(if i % 12 == 11 { ". " } else { " " });
    }
    out
}

/// A catalog of books on three topics, embedded with the fake embedder.
struct Planted {
    lib: Lib,
    backend: HashedBagOfWordsBackend,
    model: String,
    /// Each book's topic, by document id.
    topic: HashMap<String, usize>,
    reader: CatalogReader,
}

impl Planted {
    fn new(per_topic: usize) -> Self {
        let lib = Lib::new();
        let model = lib.create_model(DIMS);
        let reader = CatalogReader::open(&lib.paths.catalog).unwrap();
        let mut planted = Self {
            lib,
            backend: HashedBagOfWordsBackend::new(DIMS),
            model,
            topic: HashMap::new(),
            reader,
        };
        for topic in 0..3 {
            planted.add(topic, 0..per_topic);
        }
        planted
    }

    fn add(&mut self, topic: usize, books: std::ops::Range<usize>) {
        for book in books {
            let pages: Vec<String> = (0..3).map(|p| page(topic, book, p)).collect();
            let pages: Vec<&str> = pages.iter().map(String::as_str).collect();
            let name = format!("{} volume {book}.pdf", TOPICS[topic][0]);
            let doc = self
                .lib
                .read(&name, format!("{topic}-{book}").as_bytes(), &pages);
            self.lib.embed(&self.backend, &self.model, &doc);
            self.topic.insert(doc, topic);
        }
    }

    fn build(&self, fresh: bool) -> Outcome {
        build::build(
            &self.lib.writer.handle(),
            &self.reader,
            "t",
            fresh,
            2,
            &|| false,
        )
        .unwrap()
    }

    fn conn(&self) -> Connection {
        self.lib.conn()
    }

    /// Each book's area (number, hue, name) and place.
    fn points(&self) -> BTreeMap<String, (i64, i64, Option<String>, f64, f64)> {
        let c = self.conn();
        let mut stmt = c
            .prepare(
                "SELECT p.document_id, a.number, a.hue, a.name, p.x, p.y
                 FROM explore_points p JOIN explore_areas a ON a.id = p.area_id",
            )
            .unwrap();
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?),
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// P4-31: a book's vector is stored as it is embedded, matches Related's,
/// and is filled in for books embedded before 4.6.1; copies kept out of a
/// book in two formats aren't on the map.
#[test]
fn p4_31_book_vectors_are_kept_with_the_embeddings() {
    let planted = Planted::new(4);
    let c = planted.conn();
    assert_eq!(count(&c, "SELECT COUNT(*) FROM book_vectors"), 12);
    for doc in planted.topic.keys() {
        let (blob, seq): (Vec<u8>, i64) = c
            .query_row(
                "SELECT vector, updated_seq FROM book_vectors WHERE document_id = ?1",
                [doc],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let stored = from_f16_blob(&blob, DIMS).unwrap();
        let chunks: Vec<Vec<f32>> = edb::document_vectors(&c, &planted.model, DIMS, doc)
            .unwrap()
            .into_iter()
            .map(|(_, v)| v)
            .collect();
        let want = related::mean_vector(&chunks).unwrap();
        let worst = stored
            .iter()
            .zip(&want)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(worst < 1e-3, "{doc}: {worst}");
        let de_seq: i64 = c
            .query_row(
                "SELECT seq FROM document_embeddings WHERE document_id = ?1",
                [doc],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(seq, de_seq);
    }

    // A library embedded before 4.6.1 has none; drawing fills them in.
    let mut planted = Planted::new(11);
    planted
        .lib
        .writer
        .call(|c| c.execute("DELETE FROM book_vectors", []).unwrap())
        .unwrap();
    assert!(matches!(planted.build(false), Outcome::Built(b) if b.books == 33));
    assert_eq!(
        count(&planted.conn(), "SELECT COUNT(*) FROM book_vectors"),
        33
    );

    // A copy kept out of a book in two formats leaves the map.
    let hidden = planted.topic.keys().next().unwrap().clone();
    let shown = planted.topic.keys().nth(1).unwrap().clone();
    planted
        .lib
        .writer
        .call(move |c| {
            c.execute(
                "INSERT INTO book_pairs (pdf_document_id, epub_document_id, verdict, basis,
                     index_format, check_version, pdf_checked_at, epub_checked_at,
                     index_document_id, hidden_document_id, created_at, updated_at)
                 VALUES (?1, ?2, 'same', 'text', 'epub', 1, 't', 't', ?2, ?1, 't', 't')",
                params![hidden, shown],
            )
            .unwrap();
        })
        .unwrap();
    planted.add(0, 100..101);
    assert!(matches!(planted.build(false), Outcome::Built(b) if b.books == 33));
    let id = planted.topic.keys().next().unwrap().clone();
    assert!(!planted.points().contains_key(&id));
}

/// P4-32: the planted topics become areas, each of one topic; too few
/// books make no map.
#[test]
fn p4_32_the_map_is_drawn_from_the_stored_vectors() {
    let few = Planted::new(9);
    assert!(matches!(few.build(false), Outcome::TooFew(27)));

    let planted = Planted::new(12);
    let Outcome::Built(built) = planted.build(false) else {
        panic!("no map")
    };
    assert_eq!((built.books, built.kept), (36, 0));
    let points = planted.points();
    assert_eq!(points.len(), 36);
    // Every area holds one topic.
    let mut topics_of: BTreeMap<i64, std::collections::BTreeSet<usize>> = BTreeMap::new();
    for (doc, (number, ..)) in &points {
        topics_of
            .entry(*number)
            .or_default()
            .insert(planted.topic[doc]);
    }
    assert!(topics_of.values().all(|t| t.len() == 1), "{topics_of:?}");
    let c = planted.conn();
    // Numbers run from 1; colours are palette indexes; names come later.
    let numbers: Vec<i64> = c
        .prepare("SELECT number FROM explore_areas WHERE parent_id IS NULL ORDER BY number")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(numbers, (1..=numbers.len() as i64).collect::<Vec<_>>());
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM explore_areas WHERE name IS NOT NULL"
        ),
        0
    );
    let s = build::staleness(&c).unwrap();
    assert_eq!((s.books, s.changed, s.due()), (36, 0, false));
}

/// P4-33: a redraw after new books keeps each surviving area's number,
/// colour and the owner's name, and old books near their places; a few
/// new books don't make the map due, enough do.
#[test]
fn p4_33_a_redraw_keeps_the_map_stable() {
    let mut planted = Planted::new(40);
    assert!(matches!(planted.build(false), Outcome::Built(_)));
    let before = planted.points();
    // The owner names the area of the first topic's books.
    let astronomy = before
        .iter()
        .find(|(d, _)| planted.topic[*d] == 0)
        .map(|(_, p)| p.0)
        .unwrap();
    let id: i64 = planted
        .conn()
        .query_row(
            "SELECT id FROM explore_areas WHERE number = ?1 AND parent_id IS NULL",
            [astronomy],
            |r| r.get(0),
        )
        .unwrap();
    planted
        .lib
        .writer
        .call(move |c| app::rename(c, id, Some("  Astronomy ")).unwrap().unwrap())
        .unwrap();

    // Three new books: under ten, not due.
    planted.add(0, 40..41);
    planted.add(1, 40..41);
    planted.add(2, 40..41);
    let s = build::staleness(&planted.conn()).unwrap();
    assert_eq!((s.changed, s.due()), (3, false));
    // Twelve (10%): due.
    for t in 0..3 {
        planted.add(t, 41..44);
    }
    let s = build::staleness(&planted.conn()).unwrap();
    assert_eq!((s.changed, s.due()), (12, true));

    let Outcome::Built(built) = planted.build(false) else {
        panic!("no map")
    };
    assert_eq!(built.books, 132);
    assert!(built.kept >= 3, "{built:?}");
    let after = planted.points();
    // Nearly every old book's area kept its number and colour, and the
    // owner's name went with the books it was given to.
    let mut kept = 0;
    let mut named_before = 0;
    let mut named_after = 0;
    for (doc, (number, hue, ..)) in &before {
        let (n2, h2, name2, ..) = &after[doc];
        if (number, hue) == (n2, h2) {
            kept += 1;
        }
        if *number == astronomy {
            named_before += 1;
            if name2.as_deref() == Some("Astronomy") {
                named_after += 1;
            }
        }
    }
    assert!(
        kept * 10 >= before.len() * 9,
        "{kept} of {} kept",
        before.len()
    );
    assert!(
        named_after * 10 >= named_before * 9,
        "{named_after} of {named_before}"
    );
    // The view fits the map to the window, so only its shape shows: the
    // old books' places, centred and scaled alike, hardly move.
    let shape = |points: &BTreeMap<String, (i64, i64, Option<String>, f64, f64)>| {
        let docs: Vec<&String> = before.keys().collect();
        let n = docs.len() as f64;
        let (mx, my) = docs.iter().fold((0.0, 0.0), |(x, y), d| {
            (x + points[*d].3 / n, y + points[*d].4 / n)
        });
        let rms = (docs
            .iter()
            .map(|d| (points[*d].3 - mx).powi(2) + (points[*d].4 - my).powi(2))
            .sum::<f64>()
            / n)
            .sqrt();
        docs.iter()
            .map(|d| ((points[*d].3 - mx) / rms, (points[*d].4 - my) / rms))
            .collect::<Vec<_>>()
    };
    let (old_shape, new_shape) = (shape(&before), shape(&after));
    let (min, max) = old_shape.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
        (lo.min(p.0).min(p.1), hi.max(p.0).max(p.1))
    });
    let span = max - min;
    // Each area's middle stays put…
    type Middle = ((f64, f64), (f64, f64), f64);
    let mut middles: BTreeMap<i64, Middle> = BTreeMap::new();
    for ((doc, a), b) in before.keys().zip(&old_shape).zip(&new_shape) {
        let m = middles.entry(before[doc].0).or_default();
        m.0 = (m.0.0 + a.0, m.0.1 + a.1);
        m.1 = (m.1.0 + b.0, m.1.1 + b.1);
        m.2 += 1.0;
    }
    for (area, (a, b, n)) in &middles {
        let shift = ((a.0 - b.0) / n).hypot((a.1 - b.1) / n);
        assert!(shift < 0.05 * span, "area {area} moved {shift} of {span}");
    }
    // …and books stay near their places.
    let mut shifts: Vec<f64> = old_shape
        .iter()
        .zip(&new_shape)
        .map(|(a, b)| (a.0 - b.0).hypot(a.1 - b.1))
        .collect();
    shifts.sort_by(f64::total_cmp);
    let median = shifts[shifts.len() / 2];
    assert!(median < 0.1 * span, "median shift {median} of {span}");
    let generation: i64 = count(&planted.conn(), "SELECT generation FROM explore_maps");
    assert_eq!(generation, 2);

    // From scratch: still the owner's name where the area survives.
    assert!(matches!(planted.build(true), Outcome::Built(_)));
    let names = count(
        &planted.conn(),
        "SELECT COUNT(*) FROM explore_areas WHERE name = 'Astronomy'",
    );
    assert_eq!(names, 1);

    // A name too long, or for an area gone, is refused; blank goes back to
    // the words.
    let c = planted.conn();
    assert_eq!(
        app::rename(&c, 999_999, Some("x")).unwrap(),
        Err(app::RenameError::NotFound)
    );
    let any: i64 = count(&c, "SELECT MIN(id) FROM explore_areas");
    assert_eq!(
        app::rename(&c, any, Some(&"x".repeat(61))).unwrap(),
        Err(app::RenameError::TooLong)
    );
    app::rename(&c, any, Some("   ")).unwrap().unwrap();
    let name: Option<String> = c
        .query_row("SELECT name FROM explore_areas WHERE id = ?1", [any], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(name, None);
}

// --------------------------------------------------- in a running library

struct Running {
    env: Env,
    h: EmbedHarness,
    ws: Workspace,
}

impl Running {
    /// A library of generated books, read and embedded with the fake
    /// service, whose Explore watch looks every `every`.
    fn new(files: u32, every: Duration) -> Self {
        let mut h = EmbedHarness::new(fast_embedding());
        h.ex.inv.deps.config.explore_every = every;
        let env = Env::new();
        let mut ws = env.workspace_with_embedding(&h);
        ws.choose(&env.base.join("AtlasData")).unwrap();
        let books = env.base.join("Books");
        let out = Command::new("node")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
            .arg(&books)
            .args(["--files", &files.to_string(), "--text-kb", "2"])
            .output()
            .unwrap();
        assert!(out.status.success());
        let lib = Self { env, h, ws };
        let root = lib.root();
        let added = sources::add(root, &books).unwrap();
        inventory::control::enqueue(root, &added.id).unwrap();
        lib
    }

    fn root(&self) -> &OpenRoot {
        self.ws.require_open().unwrap()
    }

    fn set_role(&self) {
        self.root()
            .catalog()
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO provider_profiles (id, kind, label, base_url,
                         insecure_http_acknowledged, created_at, updated_at)
                     VALUES ('svc', 'openai_compatible', 'Test', 'http://localhost:9/v1', 1,
                             't', 't');
                     INSERT INTO model_roles (role, provider_profile_id, model_id,
                         dimensions, tested_at, updated_at)
                     VALUES ('embedding', 'svc', 'test-embed', 64, 't', 't');",
                )
                .unwrap();
            })
            .unwrap();
        triggers::changed(self.root()).unwrap();
    }

    fn status(&self) -> atlas_lib::domain::dto::ExploreStatus {
        let shared = self.root().explore().clone();
        self.root()
            .reader()
            .read(move |c| app::status(c, &shared))
            .unwrap()
            .unwrap()
    }

    fn wait_ready(&self) {
        assert!(
            wait_until(LONG, || self.status().state == ExploreState::Ready),
            "{:?}",
            self.status()
        );
    }

    fn mode(&mut self, mode: BackgroundMode) {
        let mut s = self.ws.settings().unwrap();
        s.background.mode = mode;
        self.ws.update_settings(s).unwrap();
    }
}

/// P4-32: the watch draws the map by itself once enough books have meaning,
/// and not while the computer is on battery; a redraw asked for under
/// Paused waits and says why.
#[test]
fn p4_32_the_watch_draws_when_due_and_the_computer_allows() {
    let mut lib = Running::new(34, Duration::from_millis(50));
    // While work is slowed (here the power source can't be told; meaning
    // is still indexed), a due map waits.
    lib.h.ex.inv.monitor.set(|s| {
        s.power = PowerSource::Unknown;
        s.battery_present = None;
    });
    lib.set_role();
    assert!(wait_until(LONG, || lib.status().library_books >= 30));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(lib.status().state, ExploreState::NotDrawn);
    // Plugged in, it's drawn.
    lib.h.ex.inv.monitor.set(|s| {
        s.power = PowerSource::Ac;
        s.battery_present = Some(false);
    });
    lib.wait_ready();
    let status = lib.status();
    assert!(status.books >= 30 && status.generation == Some(1));
    assert!(!status.building && status.new_since == 0);

    // Asked for under Paused: held, then drawn once Automatic.
    lib.mode(BackgroundMode::Paused);
    lib.root().explore().request(false);
    assert!(wait_until(LONG, || lib.status().held
        == Some(PauseReason::PausedByUser)));
    lib.mode(BackgroundMode::Automatic);
    assert!(wait_until(LONG, || lib.status().generation == Some(2)));
    assert_eq!(lib.status().held, None);

    // Closing while the watch runs doesn't wait for long.
    lib.root().explore().request(true);
    let started = Instant::now();
    lib.ws.shutdown();
    assert!(started.elapsed() < Duration::from_secs(10));
    let _ = &lib.env;
}

/// P4-34: what the window reads; the Library's area filter.
#[test]
fn p4_34_the_map_opens_files_and_the_library_lists_an_area() {
    let lib = Running::new(34, Duration::from_secs(3600));
    lib.set_role();
    assert!(wait_until(LONG, || lib.status().library_books >= 30
        && lib.status().state == ExploreState::NotDrawn));
    lib.root().explore().request(false);
    lib.wait_ready();
    let shared = lib.root().explore().clone();
    let map = lib
        .root()
        .reader()
        .read(move |c| app::map(c, &shared))
        .unwrap()
        .unwrap();
    assert_eq!(map.points.len() as u32, map.status.books);
    assert!(!map.areas.is_empty());
    assert_eq!(
        map.areas.iter().map(|a| a.size).sum::<u32>(),
        map.status.books
    );
    // Every point opens a file of its book.
    let c = Connection::open(lib.env.base.join("AtlasData/library/catalog.sqlite")).unwrap();
    for p in &map.points {
        let doc: Option<String> = c
            .query_row(
                "SELECT document_id FROM file_instances WHERE id = ?1",
                [&p.file_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(doc.as_deref(), Some(p.document_id.as_str()));
        assert!(!p.title.is_empty());
    }
    // The Library lists an area's books, and a sub-area's.
    let area = &map.areas[0];
    let listed = |area_id: i64| {
        library::list(
            lib.root().reader(),
            lib.root().root_status(),
            &DocumentQuery {
                text: None,
                source_root_id: None,
                file_type: None,
                sort: DocumentSort::NameAsc,
                page: 0,
                page_size: 500,
                include_missing: false,
                text_status: None,
                area_id: Some(area_id),
            },
        )
        .unwrap()
        .items
        .len() as u32
    };
    assert_eq!(listed(area.id), area.size);
    if let Some(sub) = area.subs.first() {
        assert_eq!(listed(sub.id), sub.size);
    }
    // Central titles name each area.
    assert!(map.areas.iter().all(|a| !a.central.is_empty()));
    let _ = Arc::new(());
}

/// What drawing costs in the app (`just bench-explore-app`): a first map
/// and a redraw after 5% new books, at 5,000 and 30,000 synthetic books of
/// 512 dimensions, from stored vectors; then reading the map as the window
/// does, and its size.
#[test]
#[ignore = "a measurement: just bench-explore-app"]
fn measure_the_build_at_5k_and_30k() {
    use atlas_lib::services::embedding::vector::to_f16_blob;
    use atlas_lib::services::explore::synthetic::planted;
    use atlas_lib::services::explore::watch::ExploreShared;

    for n in [5_000usize, 30_000] {
        let lib = Lib::new();
        let model = lib.create_model(512);
        let points = planted(40, n.div_ceil(40), 512, 0.6, 7);
        let insert = |from: usize, to: usize| {
            let rows: Vec<Vec<u8>> = (from..to).map(|i| to_f16_blob(points.row(i))).collect();
            let model = model.clone();
            lib.writer
                .call(move |c| {
                    let tx = c.transaction().unwrap();
                    for (k, blob) in rows.iter().enumerate() {
                        let i = from + k;
                        let doc = format!("doc-{i:06}");
                        tx.execute(
                            "INSERT INTO documents (id, content_hash, state, title, created_at,
                                 updated_at)
                             VALUES (?1, ?2, 'done', ?3, 't', 't')",
                            params![doc, (i as u64).to_le_bytes().to_vec(), format!("Book {i}")],
                        )
                        .unwrap();
                        tx.execute(
                            "INSERT INTO file_instances (id, source_root_id, rel_path, display_path,
                                 display_name, extension, file_type, availability, document_id,
                                 created_at, updated_at)
                             VALUES (?1, 'root-1', CAST(?1 AS BLOB), ?1, ?1, 'pdf', 'pdf',
                                     'available', ?2, 't', 't')",
                            params![format!("file-{i:06}"), doc],
                        )
                        .unwrap();
                        tx.execute(
                            "INSERT INTO document_embeddings (document_id, model_id, seq, status,
                                 chunk_count, updated_at)
                             VALUES (?1, ?2, ?3, 'done', 1, 't')",
                            params![doc, model, i as i64 + 1],
                        )
                        .unwrap();
                        tx.execute(
                            "INSERT INTO book_vectors VALUES (?1, ?2, ?3, 1, ?4)",
                            params![model, doc, blob, i as i64 + 1],
                        )
                        .unwrap();
                    }
                    tx.commit().unwrap();
                })
                .unwrap();
        };
        let first = n * 95 / 100;
        insert(0, first);
        let reader = CatalogReader::open(&lib.paths.catalog).unwrap();
        let build = || {
            let started = Instant::now();
            let out =
                build::build(&lib.writer.handle(), &reader, "t", false, 8, &|| false).unwrap();
            (out, started.elapsed().as_millis())
        };
        let (out, first_ms) = build();
        assert!(matches!(out, Outcome::Built(_)));
        insert(first, n);
        let (out, again_ms) = build();
        let Outcome::Built(b) = out else { panic!() };
        let shared = ExploreShared::default();
        let started = Instant::now();
        let map = reader.read(|c| app::map(c, &shared)).unwrap().unwrap();
        let read_ms = started.elapsed().as_millis();
        let json = serde_json::to_vec(&map).unwrap().len();
        println!(
            "explore-app books={n} first_ms={first_ms} redraw_ms={again_ms} areas={} kept={} \
             map_read_ms={read_ms} map_json_kib={}",
            b.areas,
            b.kept,
            json / 1024
        );
    }
}
