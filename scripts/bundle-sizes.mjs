#!/usr/bin/env node
// Prints a Markdown table of the installable bundles under a Tauri target
// profile folder, e.g. `node scripts/bundle-sizes.mjs src-tauri/target/release`.

import fs from 'node:fs';
import path from 'node:path';

const root = path.resolve(process.argv[2] ?? 'src-tauri/target/release');
const bundle = path.join(root, 'bundle');
const wanted = /(\.dmg|-setup\.exe|\.AppImage|\.deb)$/;

function* walk(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) yield* walk(p);
    else if (wanted.test(e.name)) yield p;
  }
}

const rows = fs.existsSync(bundle) ? [...walk(bundle)] : [];
const exe = ['atlas', 'atlas.exe'].map((n) => path.join(root, n)).find((p) => fs.existsSync(p));
const mib = (p) => (fs.statSync(p).size / 1048576).toFixed(1);

console.log(`### Bundles (${path.basename(root)})\n`);
console.log('| File | Size MiB |');
console.log('| --- | ---: |');
for (const p of rows) console.log(`| ${path.basename(p)} | ${mib(p)} |`);
if (exe) console.log(`| ${path.basename(exe)} (app binary) | ${mib(exe)} |`);
// PDFium: in the macOS bundle's Frameworks, or next to the binary elsewhere.
const pdfium = [
  path.join(bundle, 'macos', 'Atlas.app', 'Contents', 'Frameworks', 'libpdfium.dylib'),
  path.join(root, 'pdfium.dll'),
  path.join(root, 'libpdfium.so'),
].find((p) => fs.existsSync(p));
if (pdfium) console.log(`| ${path.basename(pdfium)} (PDFium, bundled) | ${mib(pdfium)} |`);
console.log('');
if (rows.length === 0) process.exitCode = 1;
