//! A title from a PDF's first page (phase1-spec P1-18): the largest text,
//! when it stands out from the body. Papers and reports almost always set
//! their title this way; a page of one size has none. Pure, so it is
//! tested without PDFium.

/// One character on the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageChar {
    pub ch: char,
    /// Font size in points, after scaling.
    pub size: f32,
    /// Baseline origin.
    pub x: f32,
    pub y: f32,
    /// Rotation in degrees (0 for ordinary text).
    pub angle: f32,
}

/// A run of characters on one baseline in one size.
#[derive(Debug, Clone)]
struct Run {
    text: String,
    size: f32,
    y: f32,
    letters: usize,
}

/// The title must be at least this much larger than the body text.
const STANDS_OUT: f32 = 1.25;
/// Lines within this fraction of the largest size continue the title.
const SAME_SIZE: f32 = 0.9;
const MAX_LINES: usize = 3;
const MAX_WORDS: usize = 25;
const MAX_CHARS: usize = 200;

fn upright(angle: f32) -> bool {
    let a = angle.rem_euclid(360.0);
    !(5.0..=355.0).contains(&a)
}

fn runs(chars: &[PageChar]) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let mut current: Option<Run> = None;
    let close = |current: &mut Option<Run>, out: &mut Vec<Run>| {
        if let Some(mut run) = current.take() {
            run.text = run.text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !run.text.is_empty() {
                out.push(run);
            }
        }
    };
    for c in chars {
        if c.ch == '\n' || c.ch == '\r' || !upright(c.angle) {
            close(&mut current, &mut out);
            continue;
        }
        if c.ch.is_whitespace() {
            if let Some(run) = current.as_mut() {
                run.text.push(' ');
            }
            continue;
        }
        if c.size <= 0.0 {
            continue;
        }
        let continues = current.as_ref().is_some_and(|run| {
            let ratio = c.size / run.size;
            (0.92..=1.08).contains(&ratio) && (c.y - run.y).abs() <= run.size * 0.5
        });
        if !continues {
            close(&mut current, &mut out);
            current = Some(Run {
                text: String::new(),
                size: c.size,
                y: c.y,
                letters: 0,
            });
        }
        let run = current.as_mut().expect("set above");
        run.text.push(c.ch);
        if c.ch.is_alphabetic() {
            run.letters += 1;
        }
    }
    close(&mut current, &mut out);
    out
}

/// The size most of the page's letters are set in.
fn body_size(runs: &[Run]) -> Option<f32> {
    let mut sizes: Vec<(f32, usize)> = Vec::new();
    for run in runs.iter().filter(|r| r.letters > 0) {
        match sizes
            .iter_mut()
            .find(|(s, _)| (run.size / *s - 1.0).abs() < 0.05)
        {
            Some((_, n)) => *n += run.letters,
            None => sizes.push((run.size, run.letters)),
        }
    }
    sizes.into_iter().max_by_key(|(_, n)| *n).map(|(s, _)| s)
}

fn ignored(run: &Run) -> bool {
    run.letters < 2 || run.text.to_lowercase().starts_with("arxiv:")
}

/// The first page's title, if its largest text stands out from the body.
pub fn first_page_title(chars: &[PageChar]) -> Option<String> {
    let runs = runs(chars);
    let body = body_size(&runs)?;
    let largest = runs
        .iter()
        .filter(|r| !ignored(r))
        .map(|r| r.size)
        .fold(0.0f32, f32::max);
    if largest < body * STANDS_OUT {
        return None;
    }
    let start = runs
        .iter()
        .position(|r| !ignored(r) && r.size >= largest * SAME_SIZE)?;
    let mut parts: Vec<&str> = Vec::new();
    let mut lines = 0;
    let mut last_y: Option<f32> = None;
    for run in &runs[start..] {
        if run.size < largest * SAME_SIZE {
            break;
        }
        if last_y.is_none_or(|y| (run.y - y).abs() > run.size * 0.5) {
            lines += 1;
            if lines > MAX_LINES {
                break;
            }
        }
        last_y = Some(run.y);
        parts.push(&run.text);
    }
    let title = parts.join(" ");
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.split_whitespace().count() > MAX_WORDS || title.chars().count() > MAX_CHARS {
        return None;
    }
    (title.chars().filter(|c| c.is_alphabetic()).count() >= 2).then_some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lays out `text` as one line at `y` in `size`, left to right.
    fn line(text: &str, size: f32, y: f32) -> Vec<PageChar> {
        let mut out: Vec<PageChar> = text
            .chars()
            .enumerate()
            .map(|(i, ch)| PageChar {
                ch,
                size,
                x: 72.0 + i as f32 * size * 0.5,
                y,
                angle: 0.0,
            })
            .collect();
        out.push(PageChar {
            ch: '\n',
            size,
            x: 0.0,
            y,
            angle: 0.0,
        });
        out
    }

    fn body(y: f32) -> Vec<PageChar> {
        line(
            "The gears mesh through a flexible spline that deforms as the wave generator turns.",
            10.0,
            y,
        )
    }

    #[test]
    fn the_largest_line_is_the_title() {
        let mut page = line("Harmonic Drives in Small Robots", 24.0, 700.0);
        page.extend(line("Ada Lin, Bo Okafor", 12.0, 670.0));
        for i in 0..20 {
            page.extend(body(640.0 - i as f32 * 12.0));
        }
        assert_eq!(
            first_page_title(&page).as_deref(),
            Some("Harmonic Drives in Small Robots")
        );
    }

    #[test]
    fn a_title_can_wrap_onto_more_lines() {
        let mut page = line("Strain Wave Gearing:", 18.0, 700.0);
        page.extend(line("A Survey of Designs", 18.0, 678.0));
        for i in 0..20 {
            page.extend(body(640.0 - i as f32 * 12.0));
        }
        assert_eq!(
            first_page_title(&page).as_deref(),
            Some("Strain Wave Gearing: A Survey of Designs")
        );
    }

    #[test]
    fn a_rotated_side_stamp_is_ignored() {
        // arXiv prints its identifier sideways in the margin, in large type.
        let mut page: Vec<PageChar> = "arXiv:2101.00001v1 [cs.RO] 1 Jan 2021"
            .chars()
            .enumerate()
            .map(|(i, ch)| PageChar {
                ch,
                size: 20.0,
                x: 20.0,
                y: 200.0 + i as f32 * 10.0,
                angle: 90.0,
            })
            .collect();
        page.extend(line("Harmonic Drives", 17.0, 700.0));
        for i in 0..20 {
            page.extend(body(640.0 - i as f32 * 12.0));
        }
        assert_eq!(first_page_title(&page).as_deref(), Some("Harmonic Drives"));
    }

    #[test]
    fn a_page_in_one_size_has_no_title() {
        let mut page = Vec::new();
        for i in 0..20 {
            page.extend(body(700.0 - i as f32 * 12.0));
        }
        assert_eq!(first_page_title(&page), None);
        assert_eq!(first_page_title(&[]), None);
    }

    #[test]
    fn a_large_paragraph_is_not_a_title() {
        let mut page = line(&"word ".repeat(40), 24.0, 700.0);
        for i in 0..20 {
            page.extend(body(640.0 - i as f32 * 12.0));
        }
        assert_eq!(first_page_title(&page), None);
    }

    #[test]
    fn a_large_page_number_is_not_a_title() {
        let mut page = line("12", 30.0, 760.0);
        for i in 0..20 {
            page.extend(body(700.0 - i as f32 * 12.0));
        }
        assert_eq!(first_page_title(&page), None);
    }
}
