//! Project Gutenberg's header and licence are not the book (Slice 3.1,
//! phase3-spec §7.6). Its EPUBs put a line such as
//! `*** START OF THE PROJECT GUTENBERG EBOOK ALICE'S ADVENTURES … ***`
//! after the header, and `*** END OF THE PROJECT GUTENBERG EBOOK … ***`
//! before the licence. The cutter drops what comes before START and
//! everything from END on, across the spine items in order.
//!
//! An unmarked book reads exactly as before:
//! - Items before a START are held back, not dropped, until START is seen.
//! - The hold is bounded (`HOLD_ITEMS`, `HOLD_CHARS`). Past it, or at the end
//!   of the book, the held items go out unchanged and the cutter stops
//!   looking.
//! - An END before any START is ignored.
//!
//! These are one publisher's known markers, not general boilerplate
//! detection.

/// Spine items held while looking for START. Real headers fit in two: the
/// cover wrapper and the header item.
pub const HOLD_ITEMS: usize = 3;
/// Characters held while looking for START.
pub const HOLD_CHARS: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Start,
    End,
}

/// Whether `line` is one of the markers.
fn mark(line: &str) -> Option<Mark> {
    let l = line.trim().to_lowercase();
    if !l.starts_with("***") || !l.contains("project gutenberg") {
        return None;
    }
    if l.contains("start of") {
        Some(Mark::Start)
    } else if l.contains("end of") {
        Some(Mark::End)
    } else {
        None
    }
}

/// The byte offset where the first line with marker `want` begins, and
/// where it ends (after its line break).
fn find(text: &str, want: Mark) -> Option<(usize, usize)> {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if mark(line) == Some(want) {
            return Some((at, at + line.len()));
        }
        at += line.len();
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Before START, holding items.
    Looking,
    /// After START: keep, watching for END.
    Body,
    /// After END: drop.
    Done,
    /// No START within the hold: everything as it is.
    Off,
}

/// Cuts a book's spine items in order. Feed each item's text; what comes
/// back is ready to split into passages, as `(item index, text)`.
#[derive(Debug)]
pub struct Cutter {
    state: State,
    held: Vec<(usize, String)>,
    held_chars: usize,
    dropped: u64,
}

impl Default for Cutter {
    fn default() -> Self {
        Self {
            state: State::Looking,
            held: Vec::new(),
            held_chars: 0,
            dropped: 0,
        }
    }
}

fn chars(s: &str) -> u64 {
    s.chars().count() as u64
}

impl Cutter {
    /// Characters left out so far.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Takes spine item `n`'s text; returns the items that can go out now.
    pub fn feed(&mut self, n: usize, text: String) -> Vec<(usize, String)> {
        match self.state {
            State::Off => vec![(n, text)],
            State::Done => {
                self.dropped += chars(&text);
                Vec::new()
            }
            State::Body => self.body(n, text).into_iter().collect(),
            State::Looking => match find(&text, Mark::Start) {
                Some((_, after)) => {
                    for (_, t) in self.held.drain(..) {
                        self.dropped += chars(&t);
                    }
                    self.held_chars = 0;
                    self.dropped += chars(&text[..after]);
                    self.state = State::Body;
                    self.body(n, text[after..].to_owned()).into_iter().collect()
                }
                None => {
                    self.held_chars += text.len();
                    self.held.push((n, text));
                    if self.held.len() > HOLD_ITEMS || self.held_chars > HOLD_CHARS {
                        self.release()
                    } else {
                        Vec::new()
                    }
                }
            },
        }
    }

    /// The book ended: anything still held goes out unchanged.
    pub fn finish(&mut self) -> Vec<(usize, String)> {
        if self.state == State::Looking {
            self.release()
        } else {
            Vec::new()
        }
    }

    fn release(&mut self) -> Vec<(usize, String)> {
        self.state = State::Off;
        self.held_chars = 0;
        std::mem::take(&mut self.held)
    }

    /// Inside the book: keeps the text before an END line.
    fn body(&mut self, n: usize, text: String) -> Option<(usize, String)> {
        match find(&text, Mark::End) {
            Some((start, _)) => {
                self.dropped += chars(&text[start..]);
                self.state = State::Done;
                Some((n, text[..start].to_owned()))
            }
            None => Some((n, text)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &str =
        "*** START OF THE PROJECT GUTENBERG EBOOK ALICE'S ADVENTURES IN WONDERLAND ***";
    const END: &str = "*** END OF THE PROJECT GUTENBERG EBOOK ALICE'S ADVENTURES IN WONDERLAND ***";

    /// Feeds `items` in order; the kept text per item and the dropped count.
    fn run(items: &[&str]) -> (Vec<(usize, String)>, u64) {
        let mut c = Cutter::default();
        let mut out = Vec::new();
        for (n, t) in items.iter().enumerate() {
            out.extend(c.feed(n, (*t).to_owned()));
        }
        out.extend(c.finish());
        (out, c.dropped())
    }

    #[test]
    fn the_header_before_start_and_the_licence_from_end_are_dropped() {
        let header = format!(
            "The Project Gutenberg eBook of Alice\nThis ebook is for the use of anyone.\n{START}\nAlice's Adventures in Wonderland\nContents\n"
        );
        let footer =
            format!("{END}\nUpdated editions will replace the previous one.\nSection 1.\n");
        let (out, dropped) = run(&["cover", &header, "CHAPTER I. Down the Rabbit-Hole", &footer]);
        assert_eq!(
            out,
            vec![
                (1, "Alice's Adventures in Wonderland\nContents\n".to_owned()),
                (2, "CHAPTER I. Down the Rabbit-Hole".to_owned()),
                (3, String::new()),
            ]
        );
        let header_chars = header.chars().count()
            - "Alice's Adventures in Wonderland\nContents\n"
                .chars()
                .count();
        assert_eq!(
            dropped,
            ("cover".len() + header_chars + footer.chars().count()) as u64
        );
    }

    #[test]
    fn start_and_the_whole_book_in_one_item() {
        // pg7337's shape: the header and the text share one spine item.
        let one = format!("Header line\n{START}\n道德經\n第一章\n");
        let (out, _) = run(&[&one, &format!("{END}\nLicence\n")]);
        assert_eq!(out[0], (0, "道德經\n第一章\n".to_owned()));
    }

    #[test]
    fn a_book_without_markers_reads_as_before() {
        let items = ["one", "two", "three", "four", "five"];
        let (out, dropped) = run(&items);
        let texts: Vec<&str> = out.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, items);
        assert_eq!(
            out.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(dropped, 0);
        // Fewer items than the hold: released at the end.
        let (out, _) = run(&["only", "two"]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn end_without_start_is_ignored_and_this_is_a_marker_too() {
        let (out, dropped) = run(&["a", &format!("b\n{END}\nc")]);
        assert_eq!(out[1].1, format!("b\n{END}\nc"));
        assert_eq!(dropped, 0);
        let old = "*** START OF THIS PROJECT GUTENBERG EBOOK THE RAVEN ***";
        let (out, _) = run(&[&format!("header\n{old}\nThe Raven")]);
        assert_eq!(out, vec![(0, "The Raven".to_owned())]);
    }

    #[test]
    fn the_hold_is_bounded() {
        // A START after more than HOLD_ITEMS items is too late: nothing cut.
        let late = format!("x\n{START}\nbody");
        let items = ["a", "b", "c", "d", late.as_str()];
        let (out, dropped) = run(&items);
        assert_eq!(out.len(), 5);
        assert_eq!(out[4].1, late);
        assert_eq!(dropped, 0);
        // Or more than HOLD_CHARS characters.
        let big = "w ".repeat(HOLD_CHARS);
        let (out, _) = run(&[&big, &format!("{START}\nbody")]);
        assert_eq!(out.len(), 2);
        assert!(out[1].1.starts_with("***"));
    }

    #[test]
    fn only_marker_lines_count() {
        assert_eq!(
            mark("  *** START OF THE PROJECT GUTENBERG EBOOK X ***  "),
            Some(Mark::Start)
        );
        assert_eq!(
            mark("*** END OF THIS PROJECT GUTENBERG EBOOK X ***"),
            Some(Mark::End)
        );
        assert_eq!(mark("the start of the Project Gutenberg story"), None);
        assert_eq!(mark("*** a heading ***"), None);
    }
}
