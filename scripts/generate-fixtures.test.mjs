// Tests for scripts/generate-fixtures.mjs (node --test). PDFium reading the
// generated PDFs is checked in src-tauri/tests/pdfium.rs.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';

import { selfTestPdf } from './lib/fixture-text.mjs';
import { treeHash } from './lib/tree-hash.mjs';

const script = path.join(path.dirname(fileURLToPath(import.meta.url)), 'generate-fixtures.mjs');

/**
 * Output of the Phase 0 generator for `--files 60 --flat 5`, recorded on
 * `main` before Slice 1.2 changed it. Without --text-kb the output must not
 * change, byte for byte (phase1-spec §11).
 */
const PHASE0_GOLDEN = 'f61b6804b4aab0239eb7b157b6af621bf1c9bff36edb6337530a9eefb060fa70';

function generate(...args) {
  const base = fs.mkdtempSync(path.join(os.tmpdir(), 'atlas-fx-test-'));
  const dir = path.join(base, 'fx');
  const stdout = execFileSync(process.execPath, [script, dir, ...args], { encoding: 'utf8' });
  const summary = JSON.parse(stdout.split('\nTo delete later')[0]);
  return { base, dir, summary };
}

function cleanup(base) {
  const locked = path.join(base, 'fx', 'locked');
  if (fs.existsSync(locked)) fs.chmodSync(locked, 0o755);
  fs.rmSync(base, { recursive: true, force: true });
}

function files(dir, pattern) {
  const found = [];
  const walk = (d) => {
    for (const name of fs.readdirSync(d)) {
      const abs = path.join(d, name);
      const st = fs.lstatSync(abs);
      if (st.isSymbolicLink()) continue;
      if (st.isDirectory()) {
        if (name !== 'locked') walk(abs);
      } else if (pattern.test(name)) found.push(abs);
    }
  };
  walk(dir);
  return found.sort();
}

/** Checks the cross-reference table and returns the page count and page 1's text. */
function readPdf(bytes) {
  const text = bytes.toString('latin1');
  const start = Number(/startxref\n(\d+)\n%%EOF\n$/.exec(text)?.[1]);
  assert.ok(Number.isInteger(start), 'startxref');
  assert.equal(text.slice(start, start + 5), 'xref\n');
  const [, first, count] = /^xref\n(\d+) (\d+)\n/.exec(text.slice(start));
  assert.equal(Number(first), 0);
  const rows = text
    .slice(start)
    .split('\n')
    .slice(2, 2 + Number(count));
  assert.equal(rows[0], '0000000000 65535 f ');
  rows.slice(1).forEach((row, n) => {
    const offset = Number(row.slice(0, 10));
    assert.equal(
      text.slice(offset, offset + `${n + 1} 0 obj`.length),
      `${n + 1} 0 obj`,
      `object ${n + 1}`,
    );
  });
  const pages = Number(/\/Type \/Pages \/Kids \[[^\]]*\] \/Count (\d+)/.exec(text)[1]);
  const firstStream = /stream\n([\s\S]*?)\nendstream/.exec(text)[1];
  const page1 = [...firstStream.matchAll(/\(((?:\\.|[^\\)])*)\) Tj/g)]
    .map((m) => m[1].replace(/\\(.)/g, '$1'))
    .join(' ');
  return { pages, page1 };
}

/** Parses a zip's central directory and verifies each entry against it. */
function readZip(bytes) {
  const end = bytes.length - 22;
  assert.equal(bytes.readUInt32LE(end), 0x06054b50, 'end of central directory');
  const count = bytes.readUInt16LE(end + 10);
  let p = bytes.readUInt32LE(end + 16);
  const entries = [];
  for (let n = 0; n < count; n++) {
    assert.equal(bytes.readUInt32LE(p), 0x02014b50);
    const method = bytes.readUInt16LE(p + 10);
    const crc = bytes.readUInt32LE(p + 16);
    const size = bytes.readUInt32LE(p + 24);
    const nameLen = bytes.readUInt16LE(p + 28);
    const extraLen = bytes.readUInt16LE(p + 30);
    const commentLen = bytes.readUInt16LE(p + 32);
    const local = bytes.readUInt32LE(p + 42);
    const name = bytes.toString('utf8', p + 46, p + 46 + nameLen);
    assert.equal(bytes.readUInt32LE(local), 0x04034b50, `${name}: local header`);
    const localName = bytes.readUInt16LE(local + 26);
    const localExtra = bytes.readUInt16LE(local + 28);
    const dataStart = local + 30 + localName + localExtra;
    const data = bytes.subarray(dataStart, dataStart + size);
    assert.equal(method, 0, `${name}: stored`);
    assert.equal(zlib.crc32(data), crc, `${name}: CRC`);
    entries.push({ name, data, localExtra, local });
    p += 46 + nameLen + extraLen + commentLen;
  }
  return entries;
}

test('without --text-kb the output is the Phase 0 output, byte for byte', () => {
  const { base, dir } = generate('--files', '60', '--flat', '5');
  try {
    assert.equal(treeHash(dir), PHASE0_GOLDEN);
  } finally {
    cleanup(base);
  }
});

test('--text-kb writes valid multi-page PDFs and EPUBs with unique markers', () => {
  const { base, dir, summary } = generate('--files', '30', '--flat', '3', '--text-kb', '8');
  try {
    assert.equal(summary.catalogableFiles, 39);
    assert.equal(summary.textKb, 8);

    const pdfs = files(dir, /\.pdf$/i);
    let pages = 0;
    const seen = new Set();
    for (const file of pdfs) {
      const bytes = fs.readFileSync(file);
      const pdf = readPdf(bytes);
      assert.ok(pdf.pages >= 2, `${file}: ${pdf.pages} pages`);
      const marker = /(\w+) marker (\d+)/.exec(pdf.page1);
      assert.ok(marker, `${file}: marker on page 1`);
      if (!file.includes(`${path.sep}duplicates${path.sep}`)) {
        assert.ok(!seen.has(marker[0]), `${marker[0]} is unique`);
        seen.add(marker[0]);
        pages += pdf.pages;
      }
    }
    const intro = files(dir, /^Introduction\.pdf$/);
    assert.equal(intro.length, 6);
    assert.equal(new Set(intro.map((f) => fs.readFileSync(f).toString('hex'))).size, 1);
    // The six duplicates count once.
    assert.equal(summary.pdfPages, pages + readPdf(fs.readFileSync(intro[0])).pages);

    const epubs = files(dir, /\.epub$/i);
    assert.ok(epubs.length >= 10);
    for (const file of epubs) {
      const entries = readZip(fs.readFileSync(file));
      assert.equal(entries[0].name, 'mimetype');
      assert.equal(entries[0].local, 0);
      assert.equal(entries[0].localExtra, 0);
      assert.equal(entries[0].data.toString('ascii'), 'application/epub+zip');
      const byName = Object.fromEntries(entries.map((e) => [e.name, e.data.toString('utf8')]));
      assert.match(byName['META-INF/container.xml'], /full-path="OEBPS\/content\.opf"/);
      const opf = byName['OEBPS/content.opf'];
      for (const tag of ['dc:identifier', 'dc:title', 'dc:creator', 'dc:language']) {
        assert.match(opf, new RegExp(`<${tag}[^>]*>[^<]+</${tag}>`), `${file}: ${tag}`);
      }
      for (const [, href] of opf.matchAll(/<item [^>]*href="([^"]+)"/g)) {
        assert.ok(byName[`OEBPS/${href}`], `${file}: ${href} is in the archive`);
      }
      assert.match(byName['OEBPS/chapter-01.xhtml'], /\w+ marker \d+/);
    }
  } finally {
    cleanup(base);
  }
});

test('the same arguments give the same bytes', () => {
  const a = generate('--files', '12', '--text-kb', '4');
  const b = generate('--files', '12', '--text-kb', '4');
  try {
    assert.equal(treeHash(a.dir), treeHash(b.dir));
  } finally {
    cleanup(a.base);
    cleanup(b.base);
  }
});

test("the worker's self-test PDF is the one this generator writes", () => {
  const committed = path.join(
    path.dirname(script),
    '..',
    'src-tauri',
    'src',
    'services',
    'extraction',
    'selftest.pdf',
  );
  const { bytes, pages } = selfTestPdf();
  assert.equal(pages, 2);
  assert.ok(
    fs.readFileSync(committed).equals(bytes),
    "regenerate it: node -e \"import('./scripts/lib/fixture-text.mjs').then(m => require('fs').writeFileSync('src-tauri/src/services/extraction/selftest.pdf', m.selfTestPdf().bytes))\"",
  );
  assert.match(readPdf(bytes).page1, /Selftest marker 0/);
});
