//! Slice 0.3: library folders (phase0-spec §5.3, §6; tests P0-05, P0-06,
//! P0-13, P0-20, P0-24).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use atlas_lib::domain::dto::SourceRootStatus;
use atlas_lib::domain::traits::{CatalogRepository, SourceRootRecord};
use atlas_lib::error::ErrorCode;
use atlas_lib::infrastructure::native_path;
use atlas_lib::services::catalog::repository::SqliteCatalog;
use atlas_lib::services::sources;
use atlas_lib::services::workspace::{Workspace, manifest};

mod common;
use common::{Env, code};

/// An environment with an open library at `<base>/AtlasData`.
struct Lib {
    env: Env,
    ws: Workspace,
}

impl Lib {
    fn new() -> Self {
        let env = Env::new();
        let mut ws = env.workspace();
        ws.choose(&env.base.join("AtlasData")).unwrap();
        Self { env, ws }
    }

    fn folder(&self, rel: &str) -> PathBuf {
        let path = self.env.base.join(rel);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn add(
        &self,
        path: &Path,
    ) -> atlas_lib::error::AtlasResult<atlas_lib::domain::dto::SourceRoot> {
        sources::add(self.ws.require_open().unwrap(), path)
    }

    fn list(&self) -> Vec<atlas_lib::domain::dto::SourceRoot> {
        sources::list(self.ws.require_open().unwrap()).unwrap()
    }

    fn reopen(&mut self) {
        let path = self.env.base.join("AtlasData");
        // Drop the open root first so its lock is released.
        self.ws = self.env.other_workspace();
        let mut ws = self.env.workspace();
        ws.open(&path).unwrap();
        self.ws = ws;
    }
}

type Snapshot = BTreeMap<PathBuf, (bool, u64, Option<SystemTime>)>;

/// Names, sizes, and modification times of everything under `root`.
fn snapshot(root: &Path) -> Snapshot {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let meta = fs::symlink_metadata(entry.path()).unwrap();
            let rel = entry.path().strip_prefix(root).unwrap().to_path_buf();
            out.insert(rel, (meta.is_dir(), meta.len(), meta.modified().ok()));
            if meta.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    out
}

fn populate(dir: &Path) {
    fs::create_dir_all(dir.join("Astronomy/Meteors")).unwrap();
    fs::write(dir.join("Astronomy/Meteors/detection.pdf"), b"%PDF-1.7").unwrap();
    fs::write(dir.join("handbook.epub"), b"PK\x03\x04").unwrap();
    fs::create_dir_all(dir.join("empty")).unwrap();
}

#[test]
fn added_folders_persist_across_reopen() {
    let mut lib = Lib::new();
    let books = lib.folder("Books");
    let added = lib.add(&books).unwrap();
    assert_eq!(added.status, SourceRootStatus::Available);
    assert_eq!(
        PathBuf::from(&added.display_path),
        fs::canonicalize(&books).unwrap()
    );
    assert!(!added.contains_data_root);
    assert_eq!(added.nested_in_root_id, None);

    lib.reopen();
    let listed = lib.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, added.id);
    assert_eq!(listed[0].display_path, added.display_path);
}

/// P0-05: adding, listing, and removing never changes the source tree.
#[test]
fn p0_05_sources_are_never_modified() {
    let lib = Lib::new();
    let books = lib.folder("Books");
    populate(&books);
    let before = snapshot(&books);

    let added = lib.add(&books).unwrap();
    lib.list();
    sources::remove(lib.ws.require_open().unwrap(), &added.id).unwrap();

    assert_eq!(snapshot(&books), before);
    assert!(lib.list().is_empty());
}

#[test]
fn the_same_folder_cannot_be_added_twice() {
    let lib = Lib::new();
    let books = lib.folder("Books");
    let first = lib.add(&books).unwrap();

    let err = lib.add(&books).unwrap_err();
    assert_eq!(err.code(), ErrorCode::SourceAlreadyAdded);
    assert_eq!(err.conflicting_root_id(), Some(first.id.clone()));

    let mut trailing = books.clone().into_os_string();
    trailing.push("/");
    assert_eq!(
        code(lib.add(Path::new(&trailing))),
        ErrorCode::SourceAlreadyAdded
    );

    #[cfg(unix)]
    {
        let link = lib.env.base.join("BooksLink");
        std::os::unix::fs::symlink(&books, &link).unwrap();
        assert_eq!(code(lib.add(&link)), ErrorCode::SourceAlreadyAdded);
    }
    assert_eq!(lib.list().len(), 1);
}

/// On case-insensitive volumes (the macOS default), a differently
/// capitalized spelling is the same folder.
#[test]
fn case_insensitive_aliases_are_the_same_folder() {
    let lib = Lib::new();
    let books = lib.folder("Books");
    if !lib.env.base.join("books").exists() {
        eprintln!("case-sensitive volume; skipping");
        return;
    }
    lib.add(&books).unwrap();
    assert_eq!(
        code(lib.add(&lib.env.base.join("books"))),
        ErrorCode::SourceAlreadyAdded
    );
}

/// P0-06: overlapping folders are refused when added.
#[test]
fn p0_06_nested_and_containing_folders_are_refused() {
    let lib = Lib::new();
    let books = lib.folder("Library/Books");
    let first = lib.add(&books).unwrap();

    let inner = lib.folder("Library/Books/Astronomy");
    let err = lib.add(&inner).unwrap_err();
    assert_eq!(err.code(), ErrorCode::SourceOverlaps);
    assert_eq!(err.conflicting_root_id(), Some(first.id.clone()));

    let outer = lib.env.base.join("Library");
    let err = lib.add(&outer).unwrap_err();
    assert_eq!(err.code(), ErrorCode::SourceOverlaps);
    assert_eq!(err.conflicting_root_id(), Some(first.id));

    // A sibling whose name shares a prefix is unrelated.
    let sibling = lib.folder("Library/Books2");
    lib.add(&sibling).unwrap();
    assert_eq!(lib.list().len(), 2);
}

/// P0-06: overlaps already present in a catalog resolve deterministically
/// to the outermost folder.
#[test]
fn p0_06_existing_overlaps_resolve_to_the_outermost_root() {
    let lib = Lib::new();
    let outer = fs::canonicalize(lib.folder("A")).unwrap();
    let middle = fs::canonicalize(lib.folder("A/B")).unwrap();
    let inner = fs::canonicalize(lib.folder("A/B/C")).unwrap();
    let root = lib.ws.require_open().unwrap();
    let catalog = SqliteCatalog::new(root.catalog());
    for (id, path) in [("inner", &inner), ("outer", &outer), ("middle", &middle)] {
        catalog
            .insert_source_root(SourceRootRecord {
                id: id.into(),
                workspace_id: root.manifest().root_id.clone(),
                path_bytes: native_path::encode(path),
                display_path: native_path::display(path),
                created_at: "t".into(),
                last_scan_at: None,
            })
            .unwrap();
    }

    for _ in 0..3 {
        let listed = lib.list();
        let by_id: BTreeMap<_, _> = listed
            .iter()
            .map(|r| (r.id.as_str(), r.nested_in_root_id.as_deref()))
            .collect();
        assert_eq!(by_id["outer"], None);
        assert_eq!(by_id["middle"], Some("outer"));
        assert_eq!(by_id["inner"], Some("outer"));
        let order: Vec<_> = listed.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(order, ["outer", "middle", "inner"]);
    }
}

/// P0-20: the data root is never a source; a source may contain it.
#[test]
fn p0_20_data_root_rules() {
    let lib = Lib::new();
    let data_root = lib.env.base.join("AtlasData");
    assert_eq!(code(lib.add(&data_root)), ErrorCode::SourceIsDataRoot);
    assert_eq!(
        code(lib.add(&data_root.join("exports"))),
        ErrorCode::SourceIsDataRoot
    );

    let parent = lib.add(&lib.env.base).unwrap();
    assert!(parent.contains_data_root);
    assert!(lib.list()[0].contains_data_root);
}

/// P0-13: an unplugged folder stays listed as unavailable.
#[test]
fn p0_13_unplugged_folders_are_kept() {
    let lib = Lib::new();
    let drive = lib.folder("Drive/Books");
    let added = lib.add(&drive).unwrap();

    let parked = lib.env.base.join("parked");
    fs::rename(lib.env.base.join("Drive"), &parked).unwrap();
    let listed = lib.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, added.id);
    assert_eq!(listed[0].status, SourceRootStatus::Unavailable);

    // While it is away, an overlapping folder is still refused by path.
    fs::create_dir_all(lib.env.base.join("Drive")).unwrap();
    assert_eq!(
        code(lib.add(&lib.env.base.join("Drive"))),
        ErrorCode::SourceOverlaps
    );
    fs::remove_dir(lib.env.base.join("Drive")).unwrap();

    fs::rename(&parked, lib.env.base.join("Drive")).unwrap();
    assert_eq!(lib.list()[0].status, SourceRootStatus::Available);
}

#[test]
fn unusable_selections_are_refused() {
    let lib = Lib::new();
    assert_eq!(
        code(lib.add(&lib.env.base.join("nope"))),
        ErrorCode::SourceMissing
    );
    let file = lib.env.base.join("file.pdf");
    fs::write(&file, b"%PDF").unwrap();
    assert_eq!(code(lib.add(&file)), ErrorCode::SourceNotAFolder);
    assert_eq!(code(lib.add(Path::new("relative"))), ErrorCode::InvalidPath);
    assert!(lib.list().is_empty());
}

#[cfg(unix)]
#[test]
fn permission_problems_are_reported() {
    use std::os::unix::fs::PermissionsExt;
    let lib = Lib::new();
    let locked = lib.folder("Locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let refused = lib.add(&locked);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(code(refused), ErrorCode::SourceUnreadable);

    lib.add(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let status = lib.list()[0].status;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(status, SourceRootStatus::NoAccess);
}

/// P0-24: folder names that are not valid UTF-8 are stored losslessly.
/// APFS rejects such names, so this runs on Linux only.
#[cfg(target_os = "linux")]
#[test]
fn p0_24_non_utf8_folder_names() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let lib = Lib::new();
    let odd = lib.env.base.join(OsStr::from_bytes(b"caf\xe9-books"));
    fs::create_dir_all(&odd).unwrap();
    let added = lib.add(&odd).unwrap();
    assert!(added.display_path.contains('\u{FFFD}'));
    let root = lib.ws.require_open().unwrap();
    let stored = SqliteCatalog::new(root.catalog())
        .list_source_root_records()
        .unwrap();
    assert_eq!(
        native_path::decode(&stored[0].path_bytes).unwrap(),
        fs::canonicalize(&odd).unwrap()
    );
    assert_eq!(lib.list()[0].status, SourceRootStatus::Available);
}

#[test]
fn a_path_from_another_platform_is_unavailable_not_fatal() {
    let lib = Lib::new();
    let root = lib.ws.require_open().unwrap();
    SqliteCatalog::new(root.catalog())
        .insert_source_root(SourceRootRecord {
            id: "foreign".into(),
            workspace_id: root.manifest().root_id.clone(),
            path_bytes: vec![native_path::foreign_tag(), b'C', 0, b':', 0],
            display_path: "C:".into(),
            created_at: "t".into(),
            last_scan_at: None,
        })
        .unwrap();
    let listed = lib.list();
    assert_eq!(listed[0].status, SourceRootStatus::Unavailable);
    // Its presence does not block adding unrelated folders.
    lib.add(&lib.folder("Books")).unwrap();
}

#[test]
fn add_many_reports_each_folder() {
    let lib = Lib::new();
    let books = lib.folder("Books");
    let papers = lib.folder("Papers");
    let inner = lib.folder("Books/Inner");
    let result = sources::add_many(
        lib.ws.require_open().unwrap(),
        &[books, papers, inner.clone(), lib.env.base.join("AtlasData")],
    );
    assert_eq!(result.added.len(), 2);
    assert_eq!(result.rejected.len(), 2);
    assert_eq!(result.rejected[0].error.code, ErrorCode::SourceOverlaps);
    assert_eq!(
        result.rejected[0].conflicting_root_id,
        Some(result.added[0].id.clone())
    );
    assert_eq!(PathBuf::from(&result.rejected[0].display_path), inner);
    assert_eq!(result.rejected[1].error.code, ErrorCode::SourceIsDataRoot);
}

#[test]
fn removing_an_unknown_folder_is_reported() {
    let lib = Lib::new();
    assert_eq!(
        code(sources::remove(lib.ws.require_open().unwrap(), "nope")),
        ErrorCode::SourceNotFound
    );
}

#[test]
fn source_commands_need_an_open_library() {
    let env = Env::new();
    let ws = env.workspace();
    assert_eq!(code(ws.require_open().map(|_| ())), ErrorCode::NoDataRoot);
}

/// A library created by Slice 0.2 (schema 1) migrates to the current schema.
#[test]
fn slice_02_libraries_migrate_forward() {
    let env = Env::new();
    let root_path = env.base.join("AtlasData");
    env.workspace().choose(&root_path).unwrap();

    // Turn it back into a schema-1 library.
    {
        let conn = rusqlite::Connection::open(root_path.join("library/catalog.sqlite")).unwrap();
        // Drop everything newer than schema 1, whatever later slices added.
        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table'
                 AND name NOT IN ('schema_migrations', 'workspace') AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        for table in tables {
            conn.execute_batch(&format!("DROP TABLE \"{table}\""))
                .unwrap();
        }
        conn.execute("DELETE FROM schema_migrations WHERE version >= 2", [])
            .unwrap();
    }
    let mut m = manifest::read(&root_path).unwrap();
    m.catalog_schema_version = 1;
    manifest::write(&root_path, &m).unwrap();

    let mut ws = env.workspace();
    ws.open(&root_path).unwrap();
    assert_eq!(
        manifest::read(&root_path).unwrap().catalog_schema_version,
        atlas_lib::services::catalog::SUPPORTED_SCHEMA_VERSION
    );
    let books = env.base.join("Books");
    fs::create_dir_all(&books).unwrap();
    sources::add(ws.require_open().unwrap(), &books).unwrap();
}
