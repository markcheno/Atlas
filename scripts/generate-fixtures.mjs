#!/usr/bin/env node
// Generates a synthetic, copyright-free library for testing and benchmarks
// (phase0-spec §11, phase1-spec §11). Nothing is downloaded.
//
//   node scripts/generate-fixtures.mjs <output-dir> [--files N] [--flat N] [--text-kb N]
//
// Without --text-kb, files are tiny identical stubs, byte for byte as in
// Phase 0. With --text-kb N, every PDF and EPUB gets about N KiB of
// deterministic body text with a unique marker phrase ("<subject> marker
// <i>"); PDFs have about 3,000 characters a page and EPUBs are valid zips.
//
// <output-dir> must not exist yet (or be empty), must be outside this
// repository, and must not be inside an Atlas data root.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { authorFor, bodyText, textEpub, textPdf } from './lib/fixture-text.mjs';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

function usage(message) {
  if (message) console.error(`error: ${message}`);
  console.error(
    'usage: generate-fixtures.mjs <output-dir> [--files N] [--flat N] [--text-kb N] [--untitled-every N] [--copies N] [--pairs N]',
  );
  process.exit(2);
}

const args = process.argv.slice(2);
const out = args.find((a, i) => !a.startsWith('--') && !args[i - 1]?.startsWith('--'));
if (!out) usage('missing output directory');
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  if (i === -1) return fallback;
  const n = Number(args[i + 1]);
  if (!Number.isInteger(n) || n < 0) usage(`--${name} needs a whole number`);
  return n;
};
const fileCount = opt('files', 1000);
const flatCount = opt('flat', 0);
const textKb = opt('text-kb', 0);
// Every Nth text document has no title in its metadata (PDFs show it in
// large type on page 1 instead); 0 keeps every title.
const untitledEvery = opt('untitled-every', 0);
// Extra copies of existing files under copies/, for duplicate groups.
const copyCount = opt('copies', 0);
// Books in both formats (Slice 4.5.1), with --text-kb: see `pairs/` below.
const pairCount = opt('pairs', 0);
if (pairCount > 0 && textKb === 0) usage('--pairs needs --text-kb');

const target = path.resolve(out);
const inside = (child, parent) => {
  const rel = path.relative(parent, child);
  return rel === '' || (!rel.startsWith('..') && !path.isAbsolute(rel));
};
if (inside(target, repo)) usage('the output directory must be outside the repository');
for (let dir = target; ; dir = path.dirname(dir)) {
  if (fs.existsSync(path.join(dir, 'atlas.json')))
    usage('refusing to write inside an Atlas data root');
  if (path.dirname(dir) === dir) break;
}
if (fs.existsSync(target) && fs.readdirSync(target).length > 0) {
  usage('the output directory must be empty or not exist');
}

/** A valid one-page PDF showing a line of text, with a correct xref table. */
function minimalPdf(text) {
  const safe = text.replace(/[()\\]/g, '');
  const stream = `BT /F1 18 Tf 72 720 Td (${safe}) Tj ET`;
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
    `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ];
  let body = '%PDF-1.4\n';
  const offsets = [];
  objects.forEach((obj, i) => {
    offsets.push(Buffer.byteLength(body));
    body += `${i + 1} 0 obj\n${obj}\nendobj\n`;
  });
  const xref = Buffer.byteLength(body);
  body += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const off of offsets) body += `${String(off).padStart(10, '0')} 00000 n \n`;
  body += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(body, 'latin1');
}

const PDF = minimalPdf('Atlas synthetic fixture');
const EPUB = Buffer.from('PK\u0003\u0004mimetypeapplication/epub+zip');
const subjects = ['Astronomy', 'Engineering', 'Mathematics', 'History', 'Software', 'Biology'];
const quirks = [
  'Café crème',
  'Ünïcode — naïve résumé',
  '日本語の本',
  'with  double  spaces',
  ' leading space',
  '[draft] (v2) #1 & co',
  'MiXeD CaSe',
  'a'.repeat(120),
];

let pdfOrEpub = 0;
let other = 0;
let pdfPages = 0;
let epubChapters = 0;

/** The Phase 0 stub, or (with --text-kb) a document with its own text. */
function document(kind, subject, i, title, untitled = false) {
  if (textKb === 0) return kind === 'epub' ? EPUB : PDF;
  const doc = {
    id: `${subject}-${i}`,
    title,
    author: authorFor(i),
    paragraphs: bodyText(subject, i, textKb),
    untitled,
  };
  if (kind === 'epub') {
    const { bytes, chapters } = textEpub(doc);
    epubChapters += chapters;
    return bytes;
  }
  const { bytes, pages } = textPdf(doc);
  pdfPages += pages;
  return bytes;
}
const written = [];
const write = (rel, data) => {
  const file = path.join(target, rel);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, data);
};

for (let i = 0; i < fileCount; i++) {
  const subject = subjects[i % subjects.length];
  const depth = i % 5;
  const dirs = [subject];
  for (let d = 0; d < depth; d++) dirs.push(`level-${d}-${(i >> d) % 7}`);
  const quirk = i % 17 === 0 ? `${quirks[i % quirks.length]} ` : '';
  const isEpub = i % 3 === 0;
  const ext = isEpub ? (i % 2 ? 'EPUB' : 'epub') : i % 11 === 0 ? 'PDF' : 'pdf';
  const stem = `${quirk}book ${String(i).padStart(6, '0')}`;
  const untitled = untitledEvery > 0 && (i + 1) % untitledEvery === 0;
  const rel = path.join(...dirs, `${stem}.${ext}`);
  write(rel, document(isEpub ? 'epub' : 'pdf', subject, i, `${subject} ${stem.trim()}`, untitled));
  written.push(rel);
  pdfOrEpub++;
}

// Byte-identical copies of spread-out files, each in its own folder.
for (let c = 0; c < copyCount && written.length > 0; c++) {
  const rel = written[(c * 7919) % written.length];
  write(
    path.join('copies', `copy-${String(c).padStart(5, '0')}`, path.basename(rel)),
    fs.readFileSync(path.join(target, rel)),
  );
  pdfOrEpub++;
}

// Same file name in different folders. With text they share one text, so
// they are also the same content (duplicates by hash).
const introduction = document('pdf', 'Introduction', 0, 'Introduction');
for (const subject of subjects) {
  write(path.join(subject, 'duplicates', 'Introduction.pdf'), introduction);
  pdfOrEpub++;
}

// One book in two formats (Slice 4.5.1): `pairs/` holds N books as both a
// PDF and an EPUB with one text (pages and chapters cut it differently),
// and the cases that must not pair, or must:
//   notes.pdf / notes.epub       one name, different texts: two books
//   Sampler.pdf / Sampler.epub   the EPUB is the PDF's first chapter: two
//   A/Atlas.pdf + B/Atlas.epub   different folders: no pair
//   Café.PDF / cafe.epub         case and accents fold: one book
const pairDoc = (kind, i, title, paragraphs) => {
  const doc = { id: `Pairs-${i}`, title, author: authorFor(i), paragraphs };
  return kind === 'epub' ? textEpub(doc).bytes : textPdf(doc).bytes;
};
let pairFiles = 0;
if (pairCount > 0) {
  const both = (rel, i, title, pdfText, epubText = pdfText) => {
    write(path.join('pairs', `${rel}.pdf`), pairDoc('pdf', i, title, pdfText));
    write(path.join('pairs', `${rel}.epub`), pairDoc('epub', i, title, epubText));
    pairFiles += 2;
  };
  for (let i = 0; i < pairCount; i++) {
    const stem = `paired ${String(i).padStart(4, '0')}`;
    both(stem, 9000 + i, `Pairs ${stem}`, bodyText('Pairs', 9000 + i, textKb));
  }
  both('notes', 9500, 'Notes', bodyText('Pairs', 9500, textKb), bodyText('Pairs', 9501, textKb));
  const sampler = bodyText('Pairs', 9600, textKb);
  both(
    'Sampler',
    9600,
    'Sampler',
    sampler,
    sampler.slice(0, Math.max(1, Math.floor(sampler.length / 5))),
  );
  const atlas = bodyText('Pairs', 9700, textKb);
  write(path.join('pairs', 'A', 'Atlas.pdf'), pairDoc('pdf', 9700, 'Atlas', atlas));
  write(path.join('pairs', 'B', 'Atlas.epub'), pairDoc('epub', 9700, 'Atlas', atlas));
  pairFiles += 2;
  const cafe = bodyText('Pairs', 9800, textKb);
  write(path.join('pairs', 'Café.PDF'), pairDoc('pdf', 9800, 'Café', cafe));
  write(path.join('pairs', 'cafe.epub'), pairDoc('epub', 9800, 'Café', cafe));
  pairFiles += 2;
  pdfOrEpub += pairFiles;
}

// Unsupported types and look-alikes.
for (const name of ['notes.txt', 'cover.png', 'archive.epub.zip', 'paper.pdf.part', 'README']) {
  write(path.join('misc', name), Buffer.from('not a document'));
  other++;
}

// Empty folders.
fs.mkdirSync(path.join(target, 'empty', 'nested-empty'), { recursive: true });

// One huge flat folder.
for (let i = 0; i < flatCount; i++) {
  const stem = `flat-${String(i).padStart(6, '0')}`;
  write(path.join('flat', `${stem}.pdf`), document('pdf', 'Flat', i, stem));
  pdfOrEpub++;
}

// Symlinks: a loop back to the root, and a link to a folder outside it.
const outsideDir = `${target}-outside`;
let symlinks = 0;
try {
  fs.mkdirSync(outsideDir, { recursive: true });
  fs.writeFileSync(path.join(outsideDir, 'outside.pdf'), PDF);
  fs.symlinkSync(target, path.join(target, 'loop-to-root'), 'dir');
  fs.symlinkSync(outsideDir, path.join(target, 'link-outside'), 'dir');
  symlinks = 2;
} catch (err) {
  console.warn(`symlinks skipped: ${err.code ?? err.message}`);
}

// An unreadable folder, where the platform allows it.
let unreadable = false;
if (process.platform !== 'win32') {
  const locked = path.join(target, 'locked');
  write(path.join('locked', 'hidden.pdf'), PDF);
  fs.chmodSync(locked, 0o000);
  unreadable = true;
}

const summary = {
  output: target,
  // PDF/EPUB files Atlas should catalog (the locked folder's file excluded).
  catalogableFiles: pdfOrEpub,
  unsupportedFiles: other,
  symlinks,
  unreadableFolder: unreadable ? path.join(target, 'locked') : null,
  outsideFolder: symlinks ? outsideDir : null,
};
if (textKb > 0) {
  // Pages and chapters of distinct documents (the six duplicates count once).
  Object.assign(summary, { textKb, pdfPages, epubChapters, marker: '<subject> marker <i>' });
}
if (untitledEvery > 0) summary.untitledEvery = untitledEvery;
if (copyCount > 0) summary.copies = copyCount;
// Pairs expected to be one book: the N, and Café.
if (pairCount > 0)
  Object.assign(summary, { pairs: pairCount, pairFiles, sameBooks: pairCount + 1 });
console.log(JSON.stringify(summary, null, 2));
if (unreadable) {
  console.log(
    `\nTo delete later: chmod 755 "${path.join(target, 'locked')}" && rm -r "${target}" "${outsideDir}"`,
  );
}
