//! Metadata quality (phase1-spec P1-18): what a document is called and who
//! wrote it, cleaned when the catalog is read so the stored values stay as
//! the file had them. Titles come from the file's metadata when usable,
//! then from its first page (PDFs), then from its file name.

use crate::domain::dto::TitleSource;

/// Longest title shown, in characters; longer ones end with "…".
const MAX_TITLE_CHARS: usize = 200;

/// Titles that say nothing about the document.
const PLACEHOLDER_TITLES: &[&str] = &[
    "untitled",
    "untitled document",
    "no title",
    "title",
    "document",
    "presentation",
    "slide 1",
    "new document",
    "microsoft word document",
    "powerpoint presentation",
];

/// Authors that name an account or a template, not a person.
const PLACEHOLDER_AUTHORS: &[&str] = &[
    "unknown",
    "author",
    "user",
    "admin",
    "administrator",
    "owner",
    "microsoft office user",
    "windows user",
    "default",
];

/// Prefixes office software puts before a file name ("Microsoft Word -
/// draft3.docx").
const APP_PREFIXES: &[&str] = &[
    "microsoft word - ",
    "microsoft powerpoint - ",
    "microsoft excel - ",
    "microsoft word – ",
];

/// Extensions that mark a title as a file name.
const FILE_EXTENSIONS: &[&str] = &[
    "pdf", "doc", "docx", "dot", "rtf", "odt", "txt", "tex", "dvi", "ps", "eps", "indd", "qxd",
    "qxp", "pages", "ppt", "pptx", "key", "xls", "xlsx", "html", "htm", "epub", "md",
];

/// Collapses whitespace and drops control characters.
fn tidy(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_letters(s: &str) -> bool {
    s.chars().any(char::is_alphabetic)
}

/// `Some(stem)` when `s` ends in a known document extension.
fn strip_extension(s: &str) -> Option<&str> {
    let (stem, ext) = s.rsplit_once('.')?;
    let ext = ext.trim().to_lowercase();
    (FILE_EXTENSIONS.contains(&ext.as_str()) && !stem.trim().is_empty()).then_some(stem.trim())
}

fn looks_like_path(s: &str) -> bool {
    let b = s.as_bytes();
    s.contains('\\')
        || s.starts_with('/')
        || s.starts_with("file:")
        || (b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && (b[2] == b'\\' || b[2] == b'/'))
}

fn truncate(s: String) -> String {
    if s.chars().count() <= MAX_TITLE_CHARS {
        return s;
    }
    let cut: String = s.chars().take(MAX_TITLE_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// "Untitled", "Document3", "Presentation 1", "Slide 2": a placeholder,
/// with or without a number.
fn is_placeholder(title: &str) -> bool {
    let lower = title.to_lowercase();
    let base =
        lower.trim_end_matches(|c: char| c.is_ascii_digit() || c.is_whitespace() || c == '-');
    PLACEHOLDER_TITLES.contains(&lower.as_str())
        || PLACEHOLDER_TITLES.contains(&base)
        || base == "slide"
}

/// A title worth showing, or `None` for an empty, placeholder, file-name or
/// path-like one. "Microsoft Word - Notes on gears.docx" becomes "Notes on
/// gears".
pub fn clean_title(raw: &str) -> Option<String> {
    let mut title = tidy(raw);
    let lower = title.to_lowercase();
    if let Some(prefix) = APP_PREFIXES.iter().find(|p| lower.starts_with(*p)) {
        let rest = title[prefix.len()..].to_owned();
        title = strip_extension(&rest).map_or(rest.clone(), str::to_owned);
    }
    if looks_like_path(&title) || strip_extension(&title).is_some() {
        return None;
    }
    let title = title.trim().to_owned();
    if !has_letters(&title) || is_placeholder(&title) {
        return None;
    }
    Some(truncate(title))
}

/// Authors worth showing: each entry tidied, split on `;` (commas belong to
/// "Lin, Ada"), placeholders and repeats dropped.
pub fn clean_authors(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in raw {
        for name in entry.split(';') {
            let name = tidy(name);
            if name.is_empty()
                || !has_letters(&name)
                || PLACEHOLDER_AUTHORS.contains(&name.to_lowercase().as_str())
                || out.iter().any(|a| a.eq_ignore_ascii_case(&name))
            {
                continue;
            }
            out.push(name);
        }
    }
    out
}

/// A readable title from a file name: no extension, `_` and runs of `.` or
/// `-` between words become spaces. Case and accents are kept.
/// "harmonic_drive-notes_v2.pdf" → "harmonic drive notes v2".
pub fn tidy_file_name(name: &str) -> String {
    let stem = name
        .rsplit_once('.')
        .filter(|(stem, ext)| !stem.is_empty() && !ext.contains(' ') && ext.len() <= 5)
        .map_or(name, |(stem, _)| stem);
    let spaced: String = stem
        .chars()
        .map(|c| if c == '_' || c == '.' { ' ' } else { c })
        .collect();
    // A hyphen joining two words stays ("Jean-Paul"); a run of hyphens or a
    // hyphen beside a space is a separator.
    let spaced = spaced
        .replace("--", " ")
        .replace(" -", " ")
        .replace("- ", " ");
    let tidied = tidy(&spaced);
    let tidied = tidied.trim_matches('-').trim().to_owned();
    if tidied.is_empty() {
        name.to_owned()
    } else {
        truncate(tidied)
    }
}

/// What to call a document, and where that came from.
pub fn display_title(
    metadata: Option<&str>,
    content: Option<&str>,
    file_name: &str,
) -> (String, TitleSource) {
    if let Some(t) = metadata.and_then(clean_title) {
        return (t, TitleSource::Metadata);
    }
    if let Some(t) = content.and_then(clean_title) {
        return (t, TitleSource::Contents);
    }
    (tidy_file_name(file_name), TitleSource::FileName)
}

/// Authors stored as a JSON array, cleaned.
pub fn authors_from_json(json: Option<&str>) -> Vec<String> {
    let raw: Vec<String> = json
        .and_then(|a| serde_json::from_str(a).ok())
        .unwrap_or_default();
    clean_authors(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn good_titles_are_tidied_not_changed() {
        assert_eq!(clean_title("  The  Raven\n"), Some("The Raven".into()));
        assert_eq!(
            clean_title("Candide, ou l'Optimisme"),
            Some("Candide, ou l'Optimisme".into())
        );
        assert_eq!(clean_title("1984"), None, "no letters");
        assert_eq!(clean_title("Report 2024"), Some("Report 2024".into()));
        assert_eq!(
            clean_title("Version 2.0 notes"),
            Some("Version 2.0 notes".into())
        );
    }

    #[test]
    fn placeholder_and_empty_titles_are_dropped() {
        for t in [
            "",
            "   ",
            "Untitled",
            "untitled document",
            "TITLE",
            "Slide 1",
            "--",
            "\u{0}\u{1}",
        ] {
            assert_eq!(clean_title(t), None, "{t:?}");
        }
    }

    #[test]
    fn office_prefixes_and_file_names_are_handled() {
        assert_eq!(
            clean_title("Microsoft Word - Notes on gears.docx"),
            Some("Notes on gears".into())
        );
        assert_eq!(
            clean_title("Microsoft Word - draft3.doc"),
            Some("draft3".into())
        );
        assert_eq!(clean_title("Microsoft PowerPoint - Presentation1"), None);
        assert_eq!(clean_title("thesis_final_v2.pdf"), None);
        assert_eq!(clean_title("paper.tex"), None);
        assert_eq!(clean_title("C:\\Users\\me\\report.doc"), None);
        assert_eq!(clean_title("/home/me/notes"), None);
    }

    #[test]
    fn long_titles_are_cut_with_an_ellipsis() {
        let long = "word ".repeat(100);
        let t = clean_title(&long).unwrap();
        assert_eq!(t.chars().count(), MAX_TITLE_CHARS);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn authors_split_on_semicolons_and_drop_placeholders() {
        let raw = vec![
            "Lin, Ada; Okafor, Bo".to_owned(),
            "Administrator".to_owned(),
        ];
        assert_eq!(clean_authors(&raw), vec!["Lin, Ada", "Okafor, Bo"]);
        let raw = vec!["Ada Lin".to_owned(), " ada lin ".to_owned(), "".to_owned()];
        assert_eq!(clean_authors(&raw), vec!["Ada Lin"]);
        assert!(clean_authors(&["Microsoft Office User".to_owned()]).is_empty());
    }

    #[test]
    fn file_names_become_readable_titles() {
        assert_eq!(
            tidy_file_name("harmonic_drive-notes_v2.pdf"),
            "harmonic drive-notes v2"
        );
        assert_eq!(
            tidy_file_name("Café crème book 000000.epub"),
            "Café crème book 000000"
        );
        assert_eq!(tidy_file_name("smith.2019.gears.pdf"), "smith 2019 gears");
        assert_eq!(tidy_file_name("Jean-Paul -- notes.pdf"), "Jean-Paul notes");
        assert_eq!(
            tidy_file_name("___.pdf"),
            "___.pdf",
            "nothing left: keep the name"
        );
        assert_eq!(tidy_file_name("no extension"), "no extension");
    }

    #[test]
    fn the_title_comes_from_metadata_then_contents_then_the_file_name() {
        assert_eq!(
            display_title(Some("Gears"), Some("Other"), "g.pdf"),
            ("Gears".into(), TitleSource::Metadata)
        );
        assert_eq!(
            display_title(Some("Untitled"), Some("Harmonic Drives"), "g.pdf"),
            ("Harmonic Drives".into(), TitleSource::Contents)
        );
        assert_eq!(
            display_title(None, None, "harmonic_drives.pdf"),
            ("harmonic drives".into(), TitleSource::FileName)
        );
    }
}
