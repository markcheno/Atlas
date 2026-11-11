/**
 * An Ask Atlas answer as typed pieces (phase2-spec §5.5, ADR 0015): plain
 * text, bold and italic runs, and citations. The answer is never treated
 * as markup: only closed `**…**`, `*…*` and `_…_` change the style, and
 * `[n]` (already checked against the sources by the engine) becomes a
 * citation. Everything else, `<b>` included, stays text. While the answer
 * streams, an emphasis mark that hasn't closed yet shows as it is.
 */

export type Piece =
  { kind: 'text'; text: string; bold: boolean; italic: boolean } | { kind: 'cite'; n: number };

const CITATION = /\[(\d{1,3})\]/g;
const BOLD = /\*\*(?=\S)([\s\S]*?\S)\*\*/g;
// `*` hugging its text, or `_` not inside a word (snake_case stays as it is).
const ITALIC =
  /\*(?=[^\s*])([^*\n]*?[^\s*])\*|(?<![\p{L}\p{N}_])_(?=[^\s_])([^_\n]*?[^\s_])_(?![\p{L}\p{N}_])/gu;

interface Run {
  text: string;
  bold: boolean;
  italic: boolean;
}

/** Splits `text` at matches of `re`, styling the matched part's group. */
function styled(runs: Run[], re: RegExp, style: Partial<Run>): Run[] {
  const out: Run[] = [];
  for (const run of runs) {
    let at = 0;
    for (const m of run.text.matchAll(re)) {
      const inner = m.slice(1).find((g) => g !== undefined) ?? '';
      if (m.index > at) out.push({ ...run, text: run.text.slice(at, m.index) });
      out.push({ ...run, ...style, text: inner });
      at = m.index + m[0].length;
    }
    if (at < run.text.length) out.push({ ...run, text: run.text.slice(at) });
  }
  return out;
}

/** One paragraph's pieces. */
function pieces(paragraph: string): Piece[] {
  let runs: Run[] = [{ text: paragraph, bold: false, italic: false }];
  runs = styled(runs, BOLD, { bold: true });
  runs = styled(runs, ITALIC, { italic: true });
  const out: Piece[] = [];
  for (const run of runs) {
    let at = 0;
    for (const m of run.text.matchAll(CITATION)) {
      if (m.index > at) out.push({ kind: 'text', ...run, text: run.text.slice(at, m.index) });
      out.push({ kind: 'cite', n: Number(m[1]) });
      at = m.index + m[0].length;
    }
    if (at < run.text.length) out.push({ kind: 'text', ...run, text: run.text.slice(at) });
  }
  return out;
}

/** The answer's paragraphs (split at blank lines), each as pieces. */
export function paragraphs(text: string): Piece[][] {
  return text
    .split(/\n[ \t]*\n/)
    .map((p) => p.trim())
    .filter((p) => p !== '')
    .map(pieces);
}
