//! Markdown notes (Slice 3.8, phase3-spec §7.7): inside `Atlas/`,
//! `Trails/<date> <title>.md`, `Findings/<date> <first words>.md`,
//! `Bookmarks/<document title>.md`, `Interests/<name>.md` and
//! `Projects/<name>.md`, each starting with front matter that names it
//! (`atlas_id`), and linked to each other in Obsidian's style or as
//! relative Markdown links.
//!
//! Every name is taken before anything is written, so links point at the
//! names the notes really get. Recorded text (excerpts, notes) is quoted
//! line by line and headings are kept to one line, so it can't break a
//! note's structure. An answer is written as it was received.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::dest::{Claim, ExportDest, safe_name};
use super::json::{BookmarkRow, CollectionRow, FindingRow, ItemKind, TrailRow};
use super::{ExportSet, Exporter};
use crate::domain::dto::{
    Anchor, AskEnding, CollectionKind, CollectionStatus, ExportFormat, ExportReport, FileType,
    FindingKind, LinkStyle, OpenAction, Reached, StepPayload,
};
use crate::error::AtlasResult;

pub const TRAILS: &str = "Trails";
pub const FINDINGS: &str = "Findings";
pub const BOOKMARKS: &str = "Bookmarks";
pub const INTERESTS: &str = "Interests";
pub const PROJECTS: &str = "Projects";
/// Words of a finding used for its name.
const NAME_WORDS: usize = 8;

enum Body<'a> {
    Trail(&'a TrailRow),
    Finding(&'a FindingRow),
    Bookmarks {
        document_id: &'a str,
        rows: Vec<&'a BookmarkRow>,
    },
    Collection(&'a CollectionRow),
}

struct Note<'a> {
    /// Its id within the export, and its `atlas_id`.
    atlas_id: String,
    kind: &'static str,
    folder: &'static str,
    base: String,
    created: String,
    updated: String,
    body: Body<'a>,
}

/// `text` collapsed to one line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A YAML double-quoted string.
pub fn yaml(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Each line of `text` as a Markdown quote.
fn quote(text: &str) -> String {
    text.trim_end()
        .lines()
        .map(|l| {
            if l.trim().is_empty() {
                ">".to_owned()
            } else {
                format!("> {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Text in a code span, so a path's characters stay as they are.
fn code(text: &str) -> String {
    if text.contains('`') {
        format!("`` {text} ``")
    } else {
        format!("`{text}`")
    }
}

/// A link's label, without what would end it.
fn label(text: &str) -> String {
    one_line(text).replace(['[', ']'], "")
}

/// Percent-encodes a path segment for a relative Markdown link.
pub fn encode(segment: &str) -> String {
    let mut out = String::new();
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// A link from a note in one of Atlas's folders to `to`.
pub fn link(style: LinkStyle, to: &Claim, text: &str) -> String {
    match style {
        LinkStyle::Obsidian => format!("[[{}/{}]]", to.folder, to.name),
        LinkStyle::Markdown => format!(
            "[{}](../{}/{}.md)",
            label(text),
            encode(to.folder),
            encode(&to.name)
        ),
    }
}

fn date(at: &str) -> &str {
    at.get(..10).unwrap_or(at)
}

fn time(at: &str) -> &str {
    at.get(11..16).unwrap_or("")
}

fn first_words(text: &str, n: usize) -> String {
    text.split_whitespace()
        .take(n)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A trail's label, as the list shows it: its title, or its first step.
fn trail_label(t: &TrailRow) -> String {
    if let Some(title) = &t.title {
        return title.clone();
    }
    let first = t.steps.first().map(|s| match &s.payload {
        StepPayload::Search(x) => x.words.clone(),
        StepPayload::Ask(x) => x.question.clone(),
        StepPayload::Open(x) => x.anchor.title.clone(),
        StepPayload::Note(x) => x.text.clone(),
    });
    first.unwrap_or_else(|| "Trail".to_owned())
}

fn finding_label(f: &FindingRow) -> String {
    let text = match (&f.kind, &f.question) {
        (FindingKind::Answer, Some(q)) => q.as_str(),
        _ => f.text.as_str(),
    };
    let words = first_words(text, NAME_WORDS);
    if words.is_empty() {
        "Finding".to_owned()
    } else {
        words
    }
}

fn bookmarks_title(rows: &[&BookmarkRow], set: &ExportSet) -> String {
    let first = rows[0];
    set.places
        .get(&first.anchor.document_id)
        .and_then(|p| p.title.clone())
        .unwrap_or_else(|| first.anchor.title.clone())
}

fn plan(set: &ExportSet) -> Vec<Note<'_>> {
    let file = &set.file;
    let mut notes = Vec::new();
    for t in &file.trails {
        notes.push(Note {
            atlas_id: t.id.clone(),
            kind: "trail",
            folder: TRAILS,
            base: safe_name(&format!(
                "{} {}",
                date(&t.started_at),
                one_line(&trail_label(t))
            )),
            created: t.started_at.clone(),
            updated: t.updated_at.clone(),
            body: Body::Trail(t),
        });
    }
    for f in &file.findings {
        notes.push(Note {
            atlas_id: f.id.clone(),
            kind: "finding",
            folder: FINDINGS,
            base: safe_name(&format!("{} {}", date(&f.created_at), finding_label(f))),
            created: f.created_at.clone(),
            updated: f.updated_at.clone(),
            body: Body::Finding(f),
        });
    }
    // Bookmarks, one note per document, in the order first saved.
    let mut order: Vec<&str> = Vec::new();
    let mut by_doc: HashMap<&str, Vec<&BookmarkRow>> = HashMap::new();
    for b in &file.bookmarks {
        let doc = b.anchor.document_id.as_str();
        if !by_doc.contains_key(doc) {
            order.push(doc);
        }
        by_doc.entry(doc).or_default().push(b);
    }
    for doc in order {
        let rows = by_doc.remove(doc).unwrap_or_default();
        let created = rows
            .iter()
            .map(|b| b.created_at.clone())
            .min()
            .unwrap_or_default();
        let updated = rows
            .iter()
            .map(|b| b.updated_at.clone())
            .max()
            .unwrap_or_default();
        notes.push(Note {
            atlas_id: format!("bookmarks:{doc}"),
            kind: "bookmarks",
            folder: BOOKMARKS,
            base: safe_name(&bookmarks_title(&rows, set)),
            created,
            updated,
            body: Body::Bookmarks {
                document_id: doc,
                rows,
            },
        });
    }
    for c in &file.collections {
        let (kind, folder) = match c.kind {
            CollectionKind::Interest => ("interest", INTERESTS),
            CollectionKind::Project => ("project", PROJECTS),
        };
        notes.push(Note {
            atlas_id: c.id.clone(),
            kind,
            folder,
            base: safe_name(&c.name),
            created: c.created_at.clone(),
            updated: c.updated_at.clone(),
            body: Body::Collection(c),
        });
    }
    notes
}

/// Writes the set as notes.
pub struct MarkdownExporter;

impl Exporter for MarkdownExporter {
    fn id(&self) -> &'static str {
        "markdown"
    }

    fn export(&self, set: &ExportSet, dest: &mut ExportDest) -> AtlasResult<ExportReport> {
        let notes = plan(set);
        let mut claims: HashMap<String, Claim> = HashMap::new();
        for n in &notes {
            let claim = dest.claim(n.folder, &n.base, "md", Some(&n.atlas_id))?;
            claims.insert(n.atlas_id.clone(), claim);
        }
        let ctx = Ctx::new(set, &claims);
        for n in &notes {
            let text = ctx.render(n);
            dest.write(&claims[&n.atlas_id], text.as_bytes())?;
        }
        Ok(dest.report(ExportFormat::Markdown))
    }
}

struct Ctx<'a> {
    set: &'a ExportSet,
    claims: &'a HashMap<String, Claim>,
    /// The trail each exported step belongs to, for a finding's link.
    step_trail: HashMap<&'a str, &'a str>,
}

impl<'a> Ctx<'a> {
    fn new(set: &'a ExportSet, claims: &'a HashMap<String, Claim>) -> Self {
        let mut step_trail = HashMap::new();
        for t in &set.file.trails {
            for s in &t.steps {
                step_trail.insert(s.id.as_str(), t.id.as_str());
            }
        }
        Self {
            set,
            claims,
            step_trail,
        }
    }

    fn link_to(&self, atlas_id: &str, text: &str) -> Option<String> {
        self.claims
            .get(atlas_id)
            .map(|c| link(self.set.link_style, c, text))
    }

    /// A source: its title, its page or place, and its file as text.
    fn source(&self, a: &Anchor) -> String {
        let mut out = format!("**{}**", one_line(&a.title));
        if let Some(p) = a.page_no {
            let _ = write!(out, ", p. {p}");
        } else if let Some(l) = &a.location {
            let _ = write!(out, ", {}", one_line(l));
        }
        match self
            .set
            .places
            .get(&a.document_id)
            .and_then(|p| p.path.as_deref())
        {
            Some(path) => {
                let _ = write!(out, " — {}", code(path));
            }
            None => out.push_str(" — no file now"),
        }
        out
    }

    fn render(&self, n: &Note<'_>) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "---\natlas_id: {}\natlas_kind: {}\ncreated: {}\nupdated: {}\nexported: {}\n---\n\n",
            yaml(&n.atlas_id),
            yaml(n.kind),
            yaml(&n.created),
            yaml(&n.updated),
            yaml(&self.set.file.exported),
        );
        match &n.body {
            Body::Trail(t) => self.trail(&mut out, t),
            Body::Finding(f) => self.finding(&mut out, f),
            Body::Bookmarks { document_id, rows } => self.bookmarks(&mut out, document_id, rows),
            Body::Collection(c) => self.collection(&mut out, c),
        }
        let mut text = out.trim_end().to_owned();
        text.push('\n');
        text
    }

    fn trail(&self, out: &mut String, t: &TrailRow) {
        let _ = writeln!(out, "# {}\n", one_line(&trail_label(t)));
        let steps = t.steps.len();
        let _ = writeln!(
            out,
            "Started {} at {} UTC. {} step{}.{}\n",
            date(&t.started_at),
            time(&t.started_at),
            steps,
            if steps == 1 { "" } else { "s" },
            if t.pinned { " Pinned." } else { "" }
        );
        for s in &t.steps {
            let at = time(&s.at);
            match &s.payload {
                StepPayload::Search(x) => {
                    let _ = writeln!(out, "## {at} · Search: {}\n", one_line(&x.words));
                    let mut filters = Vec::new();
                    if let Some(f) = &x.filters.folder_name {
                        filters.push(format!("in {}", one_line(f)));
                    }
                    if let Some(ft) = x.filters.file_type {
                        filters.push(
                            match ft {
                                FileType::Pdf => "PDFs only",
                                FileType::Epub => "EPUBs only",
                            }
                            .to_owned(),
                        );
                    }
                    if !filters.is_empty() {
                        let _ = writeln!(out, "Filters: {}.\n", filters.join(", "));
                    }
                    let _ = writeln!(
                        out,
                        "{} matched{}\n",
                        x.matched,
                        if x.results.is_empty() {
                            "."
                        } else {
                            ". The top results then:"
                        }
                    );
                    for (i, r) in x.results.iter().enumerate() {
                        let _ = writeln!(out, "{}. {}", i + 1, self.source(&r.anchor));
                        if let Some(e) = &r.anchor.excerpt {
                            let _ = writeln!(out, "{}", indent(&quote(e)));
                        }
                    }
                    out.push('\n');
                }
                StepPayload::Ask(x) => {
                    let _ = writeln!(out, "## {at} · Question: {}\n", one_line(&x.question));
                    match x.ending {
                        AskEnding::Done => {}
                        AskEnding::Stopped => out.push_str("_Stopped before the end._\n\n"),
                        AskEnding::Failed { .. } => out.push_str("_The answer failed._\n\n"),
                        AskEnding::NothingFound => {
                            out.push_str("_Nothing in the library to answer from._\n\n");
                        }
                    }
                    if !x.answer.trim().is_empty() {
                        let _ = writeln!(out, "{}\n", x.answer.trim_end());
                    }
                    if let Some(model) = &x.model {
                        let _ = writeln!(out, "_Answered by {}._\n", one_line(model));
                    }
                    if !x.sources.is_empty() {
                        out.push_str("Sources:\n\n");
                        for src in &x.sources {
                            let _ = writeln!(
                                out,
                                "- [{}] {}{}",
                                src.n,
                                self.source(&src.anchor),
                                if src.cited { " (cited)" } else { "" }
                            );
                            if let Some(e) = &src.anchor.excerpt {
                                let _ = writeln!(out, "{}", indent(&quote(e)));
                            }
                        }
                        out.push('\n');
                    }
                }
                StepPayload::Open(x) => {
                    let _ = writeln!(out, "## {at} · Opened {}\n", one_line(&x.anchor.title));
                    let actions: Vec<&str> = x
                        .actions
                        .iter()
                        .map(|a| match a {
                            OpenAction::Details => "the details",
                            OpenAction::Page => "the page",
                            OpenAction::Open => "the file",
                            OpenAction::Reveal => "its folder",
                        })
                        .collect();
                    let via = match x.via {
                        Reached::Result => "a search result",
                        Reached::Citation => "a citation",
                        Reached::Related => "a related document",
                        Reached::Suggestion => "a suggestion",
                        Reached::Bookmark => "a bookmark",
                        Reached::Trail => "a trail",
                    };
                    let _ = writeln!(
                        out,
                        "{}. From {via}{}.\n",
                        self.source(&x.anchor),
                        if actions.is_empty() {
                            String::new()
                        } else {
                            format!(", to see {}", actions.join(", then "))
                        }
                    );
                }
                StepPayload::Note(x) => {
                    let _ = writeln!(out, "## {at} · Note\n\n{}\n", quote(&x.text));
                }
            }
            if let Some(note) = &s.note {
                let _ = writeln!(out, "> **Note:**\n{}\n", quote(note));
            }
        }
    }

    fn finding(&self, out: &mut String, f: &FindingRow) {
        let _ = writeln!(out, "# {}\n", one_line(&finding_label(f)));
        let _ = writeln!(out, "Saved {}.\n", date(&f.created_at));
        match f.kind {
            FindingKind::Answer => {
                if let Some(q) = &f.question {
                    let _ = writeln!(out, "**Question:** {}\n", one_line(q));
                }
                let _ = writeln!(out, "{}\n", f.text.trim_end());
            }
            FindingKind::Passage => {
                let _ = writeln!(out, "{}\n", quote(&f.text));
            }
        }
        if !f.sources.is_empty() {
            out.push_str("Sources:\n\n");
            for s in &f.sources {
                let _ = writeln!(out, "- [{}] {}", s.n, self.source(&s.anchor));
            }
            out.push('\n');
        }
        if let Some(note) = &f.note {
            let _ = writeln!(out, "**Note:**\n\n{}\n", quote(note));
        }
        if let Some(trail) = f
            .from_step_id
            .as_deref()
            .and_then(|s| self.step_trail.get(s))
        {
            let name = self
                .claims
                .get(*trail)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            if let Some(l) = self.link_to(trail, &name) {
                let _ = writeln!(out, "From the trail {l}.");
            }
        }
    }

    fn bookmarks(&self, out: &mut String, document_id: &str, rows: &[&BookmarkRow]) {
        let title = bookmarks_title(rows, self.set);
        let _ = writeln!(out, "# {}\n", one_line(&title));
        match self
            .set
            .places
            .get(document_id)
            .and_then(|p| p.path.as_deref())
        {
            Some(path) => {
                let _ = writeln!(out, "{}\n", code(path));
            }
            None => out.push_str("No file has this document now.\n\n"),
        }
        for b in rows {
            let place = match (b.anchor.page_no, &b.anchor.location, &b.anchor.excerpt) {
                (Some(p), _, _) => format!("Page {p}"),
                (None, Some(l), _) => one_line(l),
                (None, None, Some(_)) => "A passage".to_owned(),
                (None, None, None) => "The whole document".to_owned(),
            };
            let _ = writeln!(out, "- **{place}**, saved {}", date(&b.created_at));
            if let Some(e) = &b.anchor.excerpt {
                let _ = writeln!(out, "{}", indent(&quote(e)));
            }
            if let Some(note) = &b.note {
                let _ = writeln!(out, "  Note: {}", one_line(note));
            }
        }
    }

    fn collection(&self, out: &mut String, c: &CollectionRow) {
        let _ = writeln!(out, "# {}\n", one_line(&c.name));
        let kind = match c.kind {
            CollectionKind::Interest => "Interest",
            CollectionKind::Project => "Project",
        };
        let status = match c.status {
            CollectionStatus::Active => "active",
            CollectionStatus::Finished => "finished",
            CollectionStatus::Archived => "archived",
        };
        let _ = writeln!(out, "{kind}, {status}.\n");
        if let Some(d) = &c.description {
            let _ = writeln!(out, "{}\n", d.trim_end());
        }
        if let Some(g) = &c.goal {
            let _ = writeln!(out, "**Goal:** {}\n", one_line(g));
        }
        let mut section = |heading: &str, lines: Vec<String>| {
            if !lines.is_empty() {
                let _ = writeln!(out, "## {heading}\n");
                for l in lines {
                    let _ = writeln!(out, "- {l}");
                }
                out.push('\n');
            }
        };
        let linked = |kind: ItemKind| -> Vec<String> {
            c.items
                .iter()
                .filter(|i| i.kind == kind)
                .filter_map(|i| {
                    let id = match kind {
                        ItemKind::Bookmark => {
                            let b = self.set.file.bookmarks.iter().find(|b| b.id == i.id)?;
                            format!("bookmarks:{}", b.anchor.document_id)
                        }
                        _ => i.id.clone(),
                    };
                    let name = self.claims.get(&id)?.name.clone();
                    self.link_to(&id, &name)
                })
                .collect()
        };
        section("Trails", linked(ItemKind::Trail));
        section("Findings", linked(ItemKind::Finding));
        let mut bookmarks = linked(ItemKind::Bookmark);
        bookmarks.dedup();
        section("Bookmarks", bookmarks);
        let documents = c
            .items
            .iter()
            .filter(|i| i.kind == ItemKind::Document)
            .map(|i| {
                let place = self.set.places.get(&i.id);
                let title = place
                    .and_then(|p| p.title.clone())
                    .unwrap_or_else(|| "A document Atlas hasn't read yet".to_owned());
                match place.and_then(|p| p.path.as_deref()) {
                    Some(path) => format!("**{}** — {}", one_line(&title), code(path)),
                    None => format!("**{}** — no file now", one_line(&title)),
                }
            })
            .collect();
        section("Documents", documents);
    }
}

/// Indents a block under a list item.
fn indent(text: &str) -> String {
    text.lines()
        .map(|l| format!("   {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn yaml_strings_are_quoted_and_escaped() {
        assert_eq!(yaml("a\"b\\c\nd\u{7}"), "\"a\\\"b\\\\c\\nd\"");
    }

    #[test]
    fn links_in_both_styles() {
        let claim = Claim {
            path: PathBuf::from("x"),
            folder: TRAILS,
            name: "2026-11-11 Stoic virtue (1)".into(),
            replaces: false,
            beside: false,
        };
        assert_eq!(
            link(LinkStyle::Obsidian, &claim, "x"),
            "[[Trails/2026-11-11 Stoic virtue (1)]]"
        );
        assert_eq!(
            link(LinkStyle::Markdown, &claim, "Stoic [virtue]"),
            "[Stoic virtue](../Trails/2026-11-11%20Stoic%20virtue%20%281%29.md)"
        );
        assert_eq!(encode("é"), "%C3%A9");
    }

    #[test]
    fn recorded_text_is_quoted_and_headings_stay_on_one_line() {
        assert_eq!(quote("a\n\n---\nb"), "> a\n>\n> ---\n> b");
        assert_eq!(one_line(" a\n#  b "), "a # b");
        assert_eq!(code("a`b"), "`` a`b ``");
    }
}
