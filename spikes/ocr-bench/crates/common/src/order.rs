//! Reading order for engines that return lines with positions (PP-OCR,
//! Vision). One function, the same for both, is part of the engine as
//! Slice 4.3 would ship it (ADR 0020): two columns read down the left,
//! then down the right, with a heading across both read first.
//!
//! A recursive XY cut: a region is split at its widest empty band, across
//! (a horizontal gap) or down (a vertical gap), whichever is wider, until
//! no band is left; then its lines are read top to bottom, left to right.

/// A line of text and its box, in page pixels (y grows downward).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub text: String,
}

/// The widest gap between the intervals, as (gap, where to cut), or None
/// when they overlap end to end.
fn widest_gap(mut spans: Vec<(f32, f32)>) -> Option<(f32, f32)> {
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut reach = spans.first()?.1;
    let mut best: Option<(f32, f32)> = None;
    for &(start, end) in &spans[1..] {
        if start > reach {
            let gap = start - reach;
            if best.is_none_or(|(g, _)| gap > g) {
                best = Some((gap, (reach + start) / 2.0));
            }
        }
        reach = reach.max(end);
    }
    best
}

fn cut(lines: Vec<Line>, out: &mut Vec<Line>) {
    if lines.len() < 2 {
        out.extend(lines);
        return;
    }
    let across = widest_gap(lines.iter().map(|l| (l.y0, l.y1)).collect());
    let down = widest_gap(lines.iter().map(|l| (l.x0, l.x1)).collect());
    match (across, down) {
        // A gutter at least as wide as the widest gap between rows splits
        // the region into columns.
        (a, Some((gap, at))) if a.is_none_or(|(g, _)| gap >= g) => {
            let (left, right): (Vec<Line>, Vec<Line>) =
                lines.into_iter().partition(|l| (l.x0 + l.x1) / 2.0 < at);
            cut(left, out);
            cut(right, out);
        }
        (Some((_, at)), _) => {
            let (top, bottom): (Vec<Line>, Vec<Line>) =
                lines.into_iter().partition(|l| (l.y0 + l.y1) / 2.0 < at);
            cut(top, out);
            cut(bottom, out);
        }
        _ => {
            let mut lines = lines;
            lines.sort_by(|a, b| {
                let row = a.y0.total_cmp(&b.y0);
                // Lines that share most of their height are one row.
                let overlap = a.y1.min(b.y1) - a.y0.max(b.y0);
                let height = (a.y1 - a.y0).min(b.y1 - b.y0);
                if overlap > height / 2.0 {
                    a.x0.total_cmp(&b.x0)
                } else {
                    row
                }
            });
            out.extend(lines);
        }
    }
}

/// The lines in reading order.
pub fn reading_order(lines: Vec<Line>) -> Vec<Line> {
    let mut out = Vec::with_capacity(lines.len());
    cut(lines, &mut out);
    out
}

/// The lines' text in reading order, one line each.
pub fn ordered_text(lines: Vec<Line>) -> String {
    reading_order(lines)
        .into_iter()
        .map(|l| l.text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(x0: f32, y0: f32, x1: f32, text: &str) -> Line {
        Line {
            x0,
            y0,
            x1,
            y1: y0 + 30.0,
            text: text.to_owned(),
        }
    }

    #[test]
    fn two_columns_under_a_heading_read_left_then_right() {
        // A heading across both columns, then lines 40 px apart in two
        // columns with a 75 px gutter and a paragraph break (a wider gap)
        // at the same height in each.
        let mut lines = vec![line(100.0, 50.0, 1600.0, "Title")];
        for (i, y) in [150.0, 190.0, 230.0, 330.0, 370.0].into_iter().enumerate() {
            lines.push(line(100.0, y, 812.0, &format!("L{i}")));
            lines.push(line(887.0, y, 1600.0, &format!("R{i}")));
        }
        // Whatever order the engine returned them in.
        lines.reverse();
        assert_eq!(
            ordered_text(lines),
            "Title\nL0\nL1\nL2\nL3\nL4\nR0\nR1\nR2\nR3\nR4"
        );
    }

    #[test]
    fn one_column_reads_top_to_bottom_with_indents_and_short_lines() {
        let lines = vec![
            line(160.0, 300.0, 1400.0, "c"),
            line(100.0, 100.0, 1500.0, "a"),
            line(100.0, 200.0, 600.0, "b"),
            line(100.0, 400.0, 1500.0, "d"),
        ];
        assert_eq!(ordered_text(lines), "a\nb\nc\nd");
    }

    #[test]
    fn pieces_of_one_row_read_left_to_right() {
        // One row the engine split in two, beside nothing else.
        let lines = vec![
            line(700.0, 100.0, 1200.0, "world"),
            line(100.0, 102.0, 650.0, "hello"),
        ];
        assert_eq!(ordered_text(lines), "hello\nworld");
    }

    #[test]
    fn a_footer_across_both_columns_comes_last() {
        let mut lines = vec![line(700.0, 2000.0, 1000.0, "page 7")];
        for (i, y) in [100.0, 140.0, 180.0].into_iter().enumerate() {
            lines.push(line(100.0, y, 812.0, &format!("L{i}")));
            lines.push(line(887.0, y, 1600.0, &format!("R{i}")));
        }
        assert_eq!(ordered_text(lines), "L0\nL1\nL2\nR0\nR1\nR2\npage 7");
    }
}
