#!/usr/bin/env node
// Extraction spike driver (docs/spike-0.9.md).
//
//   node spikes/scripts/run-extraction.mjs [--out <dir>] [--no-build]
//                                          [--docling <sidecar executable>] [--only pdfium,…]
//
// Two passes per candidate over spikes/extraction-bench/corpus:
// - robustness: every document in its own process, 30 s timeout, so a crash
//   or hang is recorded instead of stopping the run;
// - throughput: all text documents in one process, timed per document by
//   the extractor itself (startup is measured separately).
// Then scores everything (crates/score) and writes
// extraction-<platform>.json and .md into <out>.

import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const spikes = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const bench = path.join(spikes, 'extraction-bench');
const corpus = path.join(bench, 'corpus');
const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1];
};
const flag = (name) => args.includes(`--${name}`);
const out = path.resolve(opt('out', path.join(os.tmpdir(), 'atlas-spikes')));
const only = opt('only', null)?.split(',');
const TIMEOUT_MS = 30_000;
const targetDir = process.env.CARGO_TARGET_DIR ? path.resolve(bench, process.env.CARGO_TARGET_DIR) : path.join(bench, 'target');
const exe = (name) => path.join(targetDir, 'release', process.platform === 'win32' ? `${name}.exe` : name);

const manifest = JSON.parse(fs.readFileSync(path.join(corpus, 'manifest.json'), 'utf8'));
const tools = (() => {
  try {
    return JSON.parse(fs.readFileSync(path.join(spikes, '.tools', 'paths.json'), 'utf8'));
  } catch {
    return {};
  }
})();

const PDF_KINDS = ['pdf-generated', 'pdf-real', 'pdf-scanned', 'malformed-pdf'];
const CANDIDATES = [
  { name: 'pdfium', cmd: exe('pdf-pdfium'), kinds: PDF_KINDS, env: { ATLAS_PDFIUM: tools.pdfium?.library ?? '' }, library: tools.pdfium?.library },
  { name: 'pdf-extract', cmd: exe('pdf-extract'), kinds: PDF_KINDS },
  { name: 'rbook', cmd: exe('epub-rbook'), kinds: ['epub', 'malformed-epub'] },
];
const docling = opt('docling', null);
if (docling) CANDIDATES.push({ name: 'docling', cmd: path.resolve(docling), kinds: PDF_KINDS, sidecar: true });

fs.mkdirSync(out, { recursive: true });
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'atlas-xbench-'));

function build() {
  console.log('extraction: building (release)…');
  const r = spawnSync('cargo', ['build', '--release', '--locked', '--workspace'], { cwd: bench, stdio: 'inherit' });
  if (r.status !== 0) throw new Error('cargo build failed');
}

// Runs a candidate on files; resolves with its JSON lines, exit status and wall time.
function runCandidate(c, files, timeoutMs) {
  const resultFile = path.join(tmp, `${c.name}-${Date.now()}-${Math.random().toString(36).slice(2)}.jsonl`);
  return new Promise((resolve) => {
    const started = process.hrtime.bigint();
    const child = spawn(c.cmd, files, {
      env: { ...process.env, ...c.env, XBENCH_OUT: resultFile },
      stdio: ['ignore', 'ignore', 'pipe'],
    });
    let stderr = '';
    child.stderr.on('data', (d) => (stderr = (stderr + d).slice(-2000)));
    let timedOut = false;
    const timer = timeoutMs ? setTimeout(() => ((timedOut = true), child.kill('SIGKILL')), timeoutMs) : null;
    child.on('exit', (code, signal) => {
      if (timer) clearTimeout(timer);
      const wallMs = Number(process.hrtime.bigint() - started) / 1e6;
      let lines = [];
      try {
        lines = fs.readFileSync(resultFile, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
        fs.rmSync(resultFile);
      } catch {
        // no output: crash before the first line
      }
      resolve({ lines, code, signal, timedOut, wallMs, stderr });
    });
    child.on('error', (e) => resolve({ lines: [], code: null, signal: null, timedOut: false, wallMs: 0, stderr: e.message }));
  });
}

const docsFor = (c) => manifest.documents.filter((d) => c.kinds.includes(d.kind));

async function main() {
  if (!flag('no-build')) build();
  const report = {
    date: new Date().toISOString(),
    machine: {
      os: `${os.type()} ${os.release()}`,
      platform: process.platform,
      arch: process.arch,
      cpu: os.cpus()[0]?.model ?? 'unknown',
      cores: os.cpus().length,
      memGiB: os.totalmem() / 2 ** 30,
      ci: Boolean(process.env.CI),
    },
    corpus: { documents: manifest.documents.length, bytes: Object.values(manifest.files).reduce((a, f) => a + f.bytes, 0) },
    candidates: {},
    documents: [],
  };
  const resultsPath = path.join(tmp, 'results.jsonl');
  fs.writeFileSync(resultsPath, '');

  for (const c of CANDIDATES) {
    if (only && !only.includes(c.name)) continue;
    if (!fs.existsSync(c.cmd)) {
      console.log(`extraction: ${c.name} not built (${c.cmd}); skipped`);
      continue;
    }
    const info = { binaryBytes: fs.statSync(c.cmd).size };
    if (c.library) info.libraryBytes = fs.statSync(c.library).size;

    // Startup: the process with no files (median of 5).
    const starts = [];
    for (let i = 0; i < 5; i++) starts.push((await runCandidate(c, [], 120_000)).wallMs);
    info.startupMs = starts.sort((a, b) => a - b)[2];

    // Robustness pass.
    console.log(`extraction: ${c.name}, one process per document…`);
    for (const d of docsFor(c)) {
      const file = path.join(corpus, d.path);
      const r = await runCandidate(c, [file], c.sidecar ? 300_000 : TIMEOUT_MS);
      const line = r.lines.find((l) => !l.done);
      let outcome = line?.outcome ?? (r.timedOut ? 'timeout' : 'crash');
      const row = {
        candidate: c.name,
        id: d.id,
        kind: d.kind,
        outcome,
        ms: line?.ms ?? null,
        wallMs: r.wallMs,
        exit: r.signal ?? r.code,
        error: line?.error ?? (outcome === 'ok' ? undefined : r.stderr.trim().split('\n').slice(-2).join(' ')),
        peakRssBytes: r.lines.find((l) => l.done)?.peakRssBytes ?? null,
        // Malformed inputs can produce huge text (the zip bomb); keep only the size.
        pages: d.kind.startsWith('malformed') ? [] : (line?.pages ?? []),
        textChars: (line?.pages ?? []).reduce((a, p) => a + p.length, 0),
      };
      process.stdout.write(`  ${d.id}: ${outcome}${row.ms ? ` (${row.ms.toFixed(0)} ms)` : ''}\n`);
      fs.appendFileSync(resultsPath, JSON.stringify(row) + '\n');
    }

    // Throughput pass: text documents in one process.
    const textDocs = docsFor(c).filter((d) => d.expect === 'text');
    const batch = await runCandidate(c, textDocs.map((d) => path.join(corpus, d.path)), c.sidecar ? 3_600_000 : 600_000);
    const perDoc = batch.lines.filter((l) => !l.done);
    const pages = perDoc.reduce((a, l) => a + (l.pages?.length ?? 0), 0);
    const ms = perDoc.reduce((a, l) => a + (l.ms ?? 0), 0);
    const book = perDoc.find((l) => l.file.endsWith('book.pdf'));
    info.batch = {
      documents: perDoc.length,
      pages,
      extractMs: ms,
      pagesPerSecond: ms ? (pages / ms) * 1000 : null,
      wallMs: batch.wallMs,
      peakRssBytes: batch.lines.find((l) => l.done)?.peakRssBytes ?? null,
      bookPagesPerSecond: book ? (book.pages.length / book.ms) * 1000 : null,
      firstDocumentMs: perDoc[0]?.ms ?? null,
    };
    info.version = batch.lines.find((l) => l.done)?.version ?? null;
    report.candidates[c.name] = info;
  }

  // Score.
  const scorer = spawnSync(exe('score'), ['--corpus', corpus, '--results', resultsPath], { encoding: 'utf8', maxBuffer: 256 << 20 });
  if (scorer.status !== 0) throw new Error(`score failed: ${scorer.stderr}`);
  report.documents = scorer.stdout.split('\n').filter(Boolean).map((l) => JSON.parse(l));
  summarize(report);

  const stem = `extraction-${process.platform}`;
  fs.writeFileSync(path.join(out, `${stem}.json`), JSON.stringify(report, null, 2));
  const md = markdown(report);
  fs.writeFileSync(path.join(out, `${stem}.md`), md);
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, md + '\n');
  console.log(md);
  fs.rmSync(tmp, { recursive: true, force: true });
}

const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);

function summarize(report) {
  for (const [name, info] of Object.entries(report.candidates)) {
    const rows = report.documents.filter((d) => d.candidate === name);
    const pick = (f) => rows.filter(f).map((d) => d.scores);
    const exact = pick((d) => d.kind === 'pdf-generated' && d.scores.truthKind === 'exact' && !d.id.includes('no-tounicode'));
    const twoCol = pick((d) => d.id === 'gen-two-column');
    const real = pick((d) => d.kind === 'pdf-real');
    const scanned = pick((d) => d.kind === 'pdf-scanned');
    const textDocs = pick((d) => d.scores.expect === 'text');
    const epubs = pick((d) => d.kind === 'epub');
    info.summary = {
      bornDigitalF1: mean(exact.map((s) => s.f1 ?? 0)),
      bornDigitalMinF1: exact.length ? Math.min(...exact.map((s) => s.f1 ?? 0)) : null,
      twoColumnOrder: mean(twoCol.map((s) => s.order ?? 0)),
      realRecall: mean(real.map((s) => s.recall ?? 0)),
      realOrder: mean(real.map((s) => s.order ?? 0)),
      epubF1: mean(epubs.map((s) => s.f1 ?? 0)),
      epubOrder: mean(epubs.map((s) => s.order ?? 0)),
      scannedFlagged: scanned.length ? `${scanned.filter((s) => s.correct).length}/${scanned.length}` : null,
      ocrFalsePositives: `${textDocs.filter((s) => s.ocrFalsePositive).length}/${textDocs.length}`,
      ligatureChars: textDocs.reduce((a, s) => a + (s.chars?.ligatures ?? 0), 0),
      replacementChars: textDocs.reduce((a, s) => a + (s.chars?.replacement ?? 0), 0),
      robust: `${rows.filter((d) => d.scores.robust).length}/${rows.length}`,
      crashes: rows.filter((d) => ['crash', 'timeout', 'panic'].includes(d.outcome)).map((d) => `${d.id}: ${d.outcome}`),
    };
  }
}

const f = (x, d = 3) => (x === null || x === undefined ? '–' : typeof x === 'number' ? x.toFixed(d) : x);

function markdown(report) {
  const m = report.machine;
  const lines = [];
  lines.push(`## Extraction: ${m.os} ${m.arch}`);
  lines.push('');
  lines.push(`${m.cpu} · ${m.cores} logical cores · ${f(m.memGiB, 0)} GiB RAM · corpus ${report.corpus.documents} documents, ${f(report.corpus.bytes / 2 ** 20, 1)} MiB`);
  lines.push('');
  lines.push('| Candidate | Born-digital F1 (min) | Two-column order | Real PDFs recall / order | EPUB F1 / order | Scans flagged | OCR false positives | Ligature chars left | No crash or hang | Pages/s (book) | Startup ms | Peak RSS MiB | Binary MiB (+ library) |');
  lines.push('| --- | --- | ---: | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |');
  for (const [name, c] of Object.entries(report.candidates)) {
    const s = c.summary;
    lines.push(
      `| ${name} | ${f(s.bornDigitalF1)} (${f(s.bornDigitalMinF1)}) | ${f(s.twoColumnOrder)} | ${f(s.realRecall)} / ${f(s.realOrder)} | ${f(s.epubF1)} / ${f(s.epubOrder)} | ${s.scannedFlagged ?? '–'} | ${s.ocrFalsePositives} | ${s.ligatureChars} | ${s.robust} | ${f(c.batch.bookPagesPerSecond, 0)} | ${f(c.startupMs, 0)} | ${f(c.batch.peakRssBytes / 2 ** 20, 0)} | ${f(c.binaryBytes / 2 ** 20, 1)}${c.libraryBytes ? ` (+ ${f(c.libraryBytes / 2 ** 20, 1)})` : ''} |`,
    );
  }
  lines.push('');
  lines.push('| Document | Kind | ' + Object.keys(report.candidates).join(' | ') + ' |');
  lines.push('| --- | --- | ' + Object.keys(report.candidates).map(() => '---').join(' | ') + ' |');
  const ids = [...new Set(report.documents.map((d) => d.id))].sort();
  for (const id of ids) {
    const kind = report.documents.find((d) => d.id === id).kind;
    const cells = Object.keys(report.candidates).map((name) => {
      const d = report.documents.find((x) => x.id === id && x.candidate === name);
      if (!d) return '';
      const s = d.scores;
      if (s.f1 !== undefined) return `${d.outcome} F1 ${f(s.f1, 2)} R ${f(s.recall, 2)} O ${f(s.order, 2)}${s.ocrFalsePositive ? ' ⚠OCR' : ''}`;
      return `${d.outcome}${s.correct === false ? ' ✗' : ' ✓'}`;
    });
    lines.push(`| ${id} | ${kind} | ${cells.join(' | ')} |`);
  }
  lines.push('');
  lines.push('F1, R (recall) and O (reading order) against the truth text; ✓/✗ against the expected outcome (needs OCR, encrypted, or simply no crash).');
  return lines.join('\n') + '\n';
}

main().catch((e) => {
  console.error(`extraction: ${e.message}`);
  process.exit(1);
});
