#!/usr/bin/env node
// Verifies the committed spike corpus (spikes/extraction-bench/corpus):
// every file matches its SHA-256 in manifest.json, has a source and a
// license, every document points at listed files, and nothing unlisted is
// lying around. Exits 1 on any problem.

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const corpus = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '..',
  'extraction-bench',
  'corpus',
);
const manifest = JSON.parse(fs.readFileSync(path.join(corpus, 'manifest.json'), 'utf8'));
const problems = [];

for (const [rel, f] of Object.entries(manifest.files)) {
  const file = path.join(corpus, rel);
  if (!fs.existsSync(file)) {
    problems.push(`${rel}: missing`);
    continue;
  }
  const hash = createHash('sha256').update(fs.readFileSync(file)).digest('hex');
  if (hash !== f.sha256)
    problems.push(`${rel}: SHA-256 mismatch (line endings rewritten? see .gitattributes)`);
  if (!f.source) problems.push(`${rel}: no source`);
  if (!f.license) problems.push(`${rel}: no license`);
}
for (const d of manifest.documents) {
  if (!manifest.files[d.path]) problems.push(`${d.id}: ${d.path} not in files`);
  if (d.truth?.path && !manifest.files[d.truth.path])
    problems.push(`${d.id}: truth ${d.truth.path} not in files`);
}
const walk = (dir) =>
  fs
    .readdirSync(dir, { withFileTypes: true })
    .flatMap((e) => (e.isDirectory() ? walk(path.join(dir, e.name)) : [path.join(dir, e.name)]));
for (const file of walk(corpus)) {
  const rel = path.relative(corpus, file).split(path.sep).join('/');
  if (rel !== 'manifest.json' && !manifest.files[rel]) problems.push(`${rel}: not in the manifest`);
}

const bytes = Object.values(manifest.files).reduce((a, f) => a + f.bytes, 0);
const licenses = {};
for (const f of Object.values(manifest.files)) licenses[f.license] = (licenses[f.license] ?? 0) + 1;
if (problems.length) {
  console.error(problems.map((p) => `corpus: ${p}`).join('\n'));
  process.exit(1);
}
console.log(
  `corpus: ${Object.keys(manifest.files).length} files (${(bytes / 2 ** 20).toFixed(1)} MiB), ${manifest.documents.length} documents, all checksums match`,
);
for (const [l, n] of Object.entries(licenses)) console.log(`  ${n} × ${l}`);
