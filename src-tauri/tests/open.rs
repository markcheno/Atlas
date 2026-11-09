//! Slice 0.6: opening and revealing originals (phase0-spec §9; P0-16).

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use atlas_lib::domain::dto::DocumentQuery;
use atlas_lib::domain::dto::DocumentSort;
use atlas_lib::domain::traits::SourceOpener;
use atlas_lib::error::{AtlasResult, ErrorCode};
use atlas_lib::services::inventory::control;
use atlas_lib::services::workspace::Workspace;
use atlas_lib::services::{library, source_open, sources};

mod common;
use common::{Env, InventoryHarness, code, fast_config, wait_until};

#[derive(Default)]
struct FakeOpener {
    opened: Mutex<Vec<PathBuf>>,
    revealed: Mutex<Vec<PathBuf>>,
}

impl SourceOpener for FakeOpener {
    fn open_original(&self, path: &Path) -> AtlasResult<()> {
        self.opened.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }
    fn reveal_original(&self, path: &Path) -> AtlasResult<()> {
        self.revealed.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }
}

struct Lib {
    env: Env,
    _h: InventoryHarness,
    ws: Workspace,
    opener: FakeOpener,
    books: PathBuf,
}

impl Lib {
    /// A scanned library over `<base>/Books` containing `files`.
    fn with_files(files: &[&str]) -> Self {
        Self::with_paths(&files.iter().map(OsString::from).collect::<Vec<_>>())
    }

    /// The same, with native file names (which needn't be valid Unicode).
    fn with_paths(files: &[OsString]) -> Self {
        let env = Env::new();
        let h = InventoryHarness::new(fast_config());
        let mut ws = env.workspace_with_inventory(h.deps.clone());
        ws.choose(&env.base.join("AtlasData")).unwrap();
        let books = env.base.join("Books");
        fs::create_dir_all(&books).unwrap();
        for f in files {
            let p = books.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"%PDF").unwrap();
        }
        let root = ws.require_open().unwrap();
        let added = sources::add(root, &books).unwrap();
        control::enqueue(root, &added.id).unwrap();
        assert!(wait_until(Duration::from_secs(20), || {
            control::overview(ws.require_open().unwrap())
                .is_ok_and(|o| o.active.is_none() && o.cataloged_files as usize == files.len())
        }));
        let books = dunce::canonicalize(&books).unwrap();
        Self {
            env,
            _h: h,
            ws,
            opener: FakeOpener::default(),
            books,
        }
    }

    fn id_of(&self, name: &str) -> String {
        let root = self.ws.require_open().unwrap();
        let page = library::list(
            root.reader(),
            root.root_status(),
            &DocumentQuery {
                text: None,
                source_root_id: None,
                file_type: None,
                sort: DocumentSort::NameAsc,
                page: 0,
                page_size: 200,
                include_missing: true,
                text_status: None,
            },
        )
        .unwrap();
        page.items
            .into_iter()
            .find(|i| i.display_name == name)
            .unwrap_or_else(|| panic!("{name} not cataloged"))
            .id
    }

    fn open(&self, id: &str) -> AtlasResult<()> {
        source_open::open(self.ws.require_open().unwrap(), &self.opener, id, false)
    }

    fn reveal(&self, id: &str) -> AtlasResult<()> {
        source_open::reveal(self.ws.require_open().unwrap(), &self.opener, id)
    }

    fn opened(&self) -> Vec<PathBuf> {
        self.opener.opened.lock().unwrap().clone()
    }
}

/// P0-16: hostile file names reach the OS opener as exact paths; nothing
/// is interpreted by a shell.
#[test]
fn p0_16_hostile_names_are_passed_through_exactly() {
    // Windows forbids `"`, `|`, `<`, `>`, `*`, `?` and `:` in names; its set
    // uses the characters cmd.exe treats specially instead.
    #[cfg(not(windows))]
    let names = [
        "; rm -rf ~.pdf",
        "$(touch pwned).pdf",
        "quote ' \" `whoami`.pdf",
        "-n leading dash.pdf",
        "pipe | and & ampersand.pdf",
    ];
    #[cfg(windows)]
    let names = [
        "; rm -rf ~.pdf",
        "$(touch pwned).pdf",
        "quote ' `whoami`.pdf",
        "-n leading dash.pdf",
        "and & ampersand %PATH% ^caret.pdf",
    ];
    let lib = Lib::with_files(&names);
    for name in names {
        lib.open(&lib.id_of(name)).unwrap();
    }
    let expected: Vec<PathBuf> = names.iter().map(|n| lib.books.join(n)).collect();
    assert_eq!(lib.opened(), expected);
    assert!(!lib.env.base.join("pwned").exists());
    assert!(!lib.books.join("pwned").exists());
}

#[test]
fn reveal_selects_the_file() {
    let lib = Lib::with_files(&["Astronomy/meteors.pdf"]);
    lib.reveal(&lib.id_of("meteors.pdf")).unwrap();
    assert_eq!(
        *lib.opener.revealed.lock().unwrap(),
        [lib.books.join("Astronomy/meteors.pdf")]
    );
}

/// P0-16: a file deleted since the scan is reported, not opened.
#[test]
fn p0_16_missing_file_is_reported_without_opening() {
    let lib = Lib::with_files(&["gone.pdf"]);
    let id = lib.id_of("gone.pdf");
    fs::remove_file(lib.books.join("gone.pdf")).unwrap();
    assert_eq!(code(lib.open(&id)), ErrorCode::FileMissing);
    assert_eq!(code(lib.reveal(&id)), ErrorCode::FileMissing);
    assert!(lib.opened().is_empty());
}

#[test]
fn unplugged_folder_is_reported() {
    let lib = Lib::with_files(&["book.pdf"]);
    let id = lib.id_of("book.pdf");
    fs::rename(&lib.books, lib.env.base.join("unplugged")).unwrap();
    assert_eq!(code(lib.open(&id)), ErrorCode::SourceUnavailable);
    assert!(lib.opened().is_empty());
}

#[cfg(unix)]
#[test]
fn a_file_replaced_by_a_link_is_refused() {
    let lib = Lib::with_files(&["book.pdf"]);
    let id = lib.id_of("book.pdf");
    let outside = lib.env.base.join("outside.pdf");
    fs::write(&outside, b"%PDF").unwrap();
    fs::remove_file(lib.books.join("book.pdf")).unwrap();
    std::os::unix::fs::symlink(&outside, lib.books.join("book.pdf")).unwrap();
    assert_eq!(code(lib.open(&id)), ErrorCode::FileNotRegular);
    assert!(lib.opened().is_empty());
}

#[cfg(unix)]
#[test]
fn a_folder_replaced_by_a_link_outside_is_refused() {
    let lib = Lib::with_files(&["Shelf/book.pdf"]);
    let id = lib.id_of("book.pdf");
    let elsewhere = lib.env.base.join("Elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(elsewhere.join("book.pdf"), b"%PDF").unwrap();
    fs::remove_dir_all(lib.books.join("Shelf")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, lib.books.join("Shelf")).unwrap();
    assert_eq!(code(lib.open(&id)), ErrorCode::FileOutsideFolder);
    assert!(lib.opened().is_empty());
}

#[test]
fn unknown_ids_are_reported() {
    let lib = Lib::with_files(&["book.pdf"]);
    assert_eq!(code(lib.open("no-such-id")), ErrorCode::FileNotFound);
}

/// A legacy iCloud placeholder can't be opened by its real name; reveal
/// shows the placeholder so the user can download it in Finder.
#[cfg(target_os = "macos")]
#[test]
fn legacy_icloud_placeholder_is_revealed_not_opened() {
    let env = Env::new();
    let h = InventoryHarness::new(fast_config());
    let mut ws = env.workspace_with_inventory(h.deps.clone());
    ws.choose(&env.base.join("AtlasData")).unwrap();
    let books = env.base.join("iCloud Books");
    fs::create_dir_all(&books).unwrap();
    let mut dict = plist::Dictionary::new();
    dict.insert(
        "NSURLFileSizeKey".into(),
        plist::Value::Integer(1234u64.into()),
    );
    plist::Value::Dictionary(dict)
        .to_file_binary(books.join(".Cosmos.pdf.icloud"))
        .unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    control::enqueue(root, &added.id).unwrap();
    assert!(wait_until(Duration::from_secs(20), || {
        control::overview(ws.require_open().unwrap())
            .is_ok_and(|o| o.active.is_none() && o.cataloged_files == 1)
    }));
    let lib = Lib {
        books: dunce::canonicalize(&books).unwrap(),
        env,
        _h: h,
        ws,
        opener: FakeOpener::default(),
    };
    let id = lib.id_of("Cosmos.pdf");
    assert_eq!(code(lib.open(&id)), ErrorCode::FileCloudPlaceholder);
    lib.reveal(&id).unwrap();
    assert_eq!(
        *lib.opener.revealed.lock().unwrap(),
        [lib.books.join(".Cosmos.pdf.icloud")]
    );
    assert!(lib.opened().is_empty());
}

/// A file name that isn't valid Unicode: invalid UTF-8 on Linux, an
/// unpaired surrogate on Windows. macOS file systems refuse both.
#[cfg(target_os = "linux")]
fn odd_name() -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(b"caf\xe9 book.pdf".to_vec())
}

#[cfg(windows)]
fn odd_name() -> OsString {
    use std::os::windows::ffi::OsStringExt;
    let mut wide: Vec<u16> = "caf".encode_utf16().collect();
    wide.push(0xD800);
    wide.extend(" book.pdf".encode_utf16());
    OsString::from_wide(&wide)
}

/// P0-24: such a file is listed with a replacement character and opened
/// from its exact native name.
#[cfg(any(target_os = "linux", windows))]
#[test]
fn p0_24_odd_names_open_from_native_bytes() {
    let name = odd_name();
    let lib = Lib::with_paths(std::slice::from_ref(&name));
    let display = name.to_string_lossy().into_owned();
    assert!(display.contains('\u{FFFD}'));
    lib.open(&lib.id_of(&display)).unwrap();
    assert_eq!(lib.opened(), [lib.books.join(&name)]);
}
