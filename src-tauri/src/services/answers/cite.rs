//! Citations checked as the answer streams (phase2-spec §7.6, ADR 0015).
//!
//! The model cites with `[n]`, `[n, m]`, `[n-m]` or `[n][m]`. Each number
//! is checked against the sources Atlas sent. A valid one stays in the text
//! as `[n]`, and its first use is reported. An invented one is removed from
//! the text, with the spaces before it, and counted. Text that only might
//! be a citation (`[`, `[1`, `[1,`) is held until it closes or proves not to
//! be one, so no unchecked number ever reaches the screen.

/// What the parser lets through.
#[derive(Debug, PartialEq, Eq)]
pub enum Out {
    Text(String),
    /// Source `n` was cited for the first time.
    Cited(u32),
}

/// The longest citation-like run held back before it is let through as
/// text.
const LONGEST: usize = 48;
/// The most numbers a range may expand to.
const RANGE: u32 = 20;

pub struct Citations {
    sources: u32,
    held: String,
    cited: Vec<u32>,
    dropped: u32,
}

enum Parsed {
    /// Could still become a citation.
    Incomplete,
    /// Not a citation: `[` is ordinary text.
    No,
    /// A citation of `len` bytes, with these numbers.
    Yes { len: usize, numbers: Vec<u32> },
}

impl Citations {
    /// For an answer from `sources` numbered sources (1..=sources).
    pub fn new(sources: u32) -> Self {
        Self {
            sources,
            held: String::new(),
            cited: Vec::new(),
            dropped: 0,
        }
    }

    /// The distinct valid sources cited, in order of first use.
    pub fn cited(&self) -> &[u32] {
        &self.cited
    }

    /// How many invented references were removed.
    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    /// Takes more answer text; returns what can be shown now.
    pub fn feed(&mut self, text: &str) -> Vec<Out> {
        self.held.push_str(text);
        self.drain(false)
    }

    /// The answer ended: what was held is text.
    pub fn finish(&mut self) -> Vec<Out> {
        self.drain(true)
    }

    fn drain(&mut self, last: bool) -> Vec<Out> {
        let mut out = Vec::new();
        let mut text = String::new();
        let buf = std::mem::take(&mut self.held);
        let mut at = 0;
        while let Some(found) = buf[at..].find('[') {
            let open = at + found;
            match parse(&buf[open..]) {
                Parsed::Incomplete if !last => {
                    // Hold the spaces before it too: they go if it's invented.
                    let lead = spaces_before(&buf[at..open]);
                    text.push_str(&buf[at..open - lead]);
                    self.held = buf[open - lead..].to_owned();
                    push_text(&mut out, &mut text);
                    return out;
                }
                Parsed::Incomplete | Parsed::No => {
                    text.push_str(&buf[at..=open]);
                    at = open + 1;
                }
                Parsed::Yes { len, numbers } => {
                    let valid: Vec<u32> = numbers
                        .iter()
                        .copied()
                        .filter(|n| (1..=self.sources).contains(n))
                        .collect();
                    self.dropped += (numbers.len() - valid.len()) as u32;
                    let before = &buf[at..open];
                    if valid.is_empty() {
                        text.push_str(&before[..before.len() - spaces_before(before)]);
                    } else {
                        text.push_str(before);
                        let mut seen = Vec::new();
                        for n in valid {
                            if seen.contains(&n) {
                                continue;
                            }
                            seen.push(n);
                            text.push_str(&format!("[{n}]"));
                            if !self.cited.contains(&n) {
                                self.cited.push(n);
                                push_text(&mut out, &mut text);
                                out.push(Out::Cited(n));
                            }
                        }
                    }
                    at = open + len;
                }
            }
        }
        let rest = &buf[at..];
        if last {
            text.push_str(rest);
        } else {
            // Trailing spaces wait: a citation may follow them.
            let lead = spaces_before(rest);
            text.push_str(&rest[..rest.len() - lead]);
            self.held = rest[rest.len() - lead..].to_owned();
        }
        push_text(&mut out, &mut text);
        out
    }
}

fn push_text(out: &mut Vec<Out>, text: &mut String) {
    if !text.is_empty() {
        out.push(Out::Text(std::mem::take(text)));
    }
}

/// Bytes of spaces and tabs (not line breaks) ending `s`.
fn spaces_before(s: &str) -> usize {
    s.len() - s.trim_end_matches([' ', '\t']).len()
}

/// Reads a citation at the start of `s`, which begins with `[`.
fn parse(s: &str) -> Parsed {
    let b = s.as_bytes();
    let mut i = 1;
    let mut numbers = Vec::new();
    let mut range_from: Option<u32> = None;
    loop {
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        // A number of one to three digits.
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() && i - start < 4 {
            i += 1;
        }
        if i == b.len() {
            return incomplete(i);
        }
        let digits = i - start;
        if digits == 0 || digits > 3 {
            return Parsed::No;
        }
        let n: u32 = s[start..i].parse().unwrap_or(0);
        match range_from.take() {
            Some(from) if from <= n && n - from < RANGE => numbers.extend(from..=n),
            Some(_) => return Parsed::No,
            None => numbers.push(n),
        }
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        if i == b.len() {
            return incomplete(i);
        }
        match b[i] {
            b']' => {
                return Parsed::Yes {
                    len: i + 1,
                    numbers,
                };
            }
            b',' | b';' => i += 1,
            b'-' => {
                range_from = numbers.pop();
                i += 1;
            }
            _ => return Parsed::No,
        }
    }
}

fn incomplete(read: usize) -> Parsed {
    if read >= LONGEST {
        Parsed::No
    } else {
        Parsed::Incomplete
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `parts` through a parser for `sources` sources.
    fn run(sources: u32, parts: &[&str]) -> (String, Vec<u32>, u32) {
        let mut c = Citations::new(sources);
        let mut text = String::new();
        let mut events = Vec::new();
        let mut take = |outs: Vec<Out>| {
            for o in outs {
                match o {
                    Out::Text(t) => text.push_str(&t),
                    Out::Cited(n) => events.push(n),
                }
            }
        };
        for p in parts {
            take(c.feed(p));
        }
        take(c.finish());
        assert_eq!(events, c.cited(), "one event per newly cited source");
        (text, events, c.dropped())
    }

    #[test]
    fn valid_citations_stay_and_invented_ones_go() {
        let (text, cited, dropped) = run(3, &["Rome fell [1]. Then [7] it rose [2][9]."]);
        assert_eq!(text, "Rome fell [1]. Then it rose [2].");
        assert_eq!(cited, vec![1, 2]);
        assert_eq!(dropped, 2);
    }

    #[test]
    fn a_citation_split_anywhere_is_read_the_same() {
        let whole = "Yes [1, 3] and no [12].";
        for a in 1..whole.len() {
            for b in a..whole.len() {
                let (text, cited, dropped) = run(3, &[&whole[..a], &whole[a..b], &whole[b..]]);
                assert_eq!(text, "Yes [1][3] and no.", "cuts {a}, {b}");
                assert_eq!(cited, vec![1, 3]);
                assert_eq!(dropped, 1);
            }
        }
    }

    #[test]
    fn groups_and_ranges() {
        assert_eq!(run(5, &["a [2-4] b"]).0, "a [2][3][4] b");
        assert_eq!(run(5, &["a [1;2; 2] b"]).0, "a [1][2] b");
        assert_eq!(run(2, &["a [1-3] b"]), ("a [1][2] b".into(), vec![1, 2], 1));
    }

    #[test]
    fn brackets_that_are_not_citations_are_text() {
        for text in [
            "see [the appendix]",
            "in [2020] it",
            "array[i]",
            "[a](https://x)",
            "x [1",
            "x [",
            "[]",
        ] {
            assert_eq!(run(3, &[text]), (text.into(), vec![], 0), "{text:?}");
        }
    }

    #[test]
    fn removing_an_invented_reference_keeps_line_breaks() {
        assert_eq!(run(1, &["End.\n\n[4] Next"]).0, "End.\n\n Next");
        assert_eq!(run(1, &["End  [4]\n"]).0, "End\n");
    }
}
