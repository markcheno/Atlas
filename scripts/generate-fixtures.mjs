#!/usr/bin/env node
// Generates a synthetic, copyright-free library for testing and benchmarks
// (phase0-spec §11). Files are tiny stubs; nothing is downloaded.
//
//   node scripts/generate-fixtures.mjs <output-dir> [--files N] [--flat N]
//
// <output-dir> must not exist yet (or be empty), must be outside this
// repository, and must not be inside an Atlas data root.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

function usage(message) {
  if (message) console.error(`error: ${message}`);
  console.error('usage: generate-fixtures.mjs <output-dir> [--files N] [--flat N]');
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

const PDF = Buffer.from('%PDF-1.7\n% Atlas synthetic fixture\n%%EOF\n');
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
  write(
    path.join(...dirs, `${quirk}book ${String(i).padStart(6, '0')}.${ext}`),
    isEpub ? EPUB : PDF,
  );
  pdfOrEpub++;
}

// Same file name in different folders.
for (const subject of subjects) {
  write(path.join(subject, 'duplicates', 'Introduction.pdf'), PDF);
  pdfOrEpub++;
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
  write(path.join('flat', `flat-${String(i).padStart(6, '0')}.pdf`), PDF);
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
console.log(JSON.stringify(summary, null, 2));
if (unreadable) {
  console.log(
    `\nTo delete later: chmod 755 "${path.join(target, 'locked')}" && rm -r "${target}" "${outsideDir}"`,
  );
}
