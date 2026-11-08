#!/usr/bin/env node
// Downloads the pinned PDFium build (tools.lock.json), verifies its SHA-256,
// and unpacks it into src-tauri/vendor/pdfium/<platform>/ (gitignored).
// The app bundles the library and its notices from there
// (src-tauri/tauri.<os>.conf.json), so this must run before any Rust build;
// `just setup` and the CI setup action run it. An already-verified copy is
// kept.
//
//   node scripts/fetch-pdfium.mjs [--platform darwin-arm64|linux-x64|win32-x64] [--print-path]
//
// --print-path prints the library's absolute path (for ATLAS_PDFIUM_LIBRARY).

import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const lock = JSON.parse(fs.readFileSync(path.join(repo, 'tools.lock.json'), 'utf8'));
const pdfium = lock.tools.pdfium;
const PLATFORMS = Object.keys(pdfium.assets);

export function platformKey() {
  const key = `${process.platform}-${process.arch}`;
  if (!PLATFORMS.includes(key)) {
    throw new Error(
      `PDFium is pinned for ${PLATFORMS.join(', ')}, not ${key}; Atlas builds only on those platforms`,
    );
  }
  return key;
}

export const vendorDir = (platform) => path.join(repo, 'src-tauri', 'vendor', 'pdfium', platform);
export const libraryPath = (platform) =>
  path.join(vendorDir(platform), ...pdfium.library[platform].split('/'));

/** Unpacks the pinned archive for `platform` unless a verified copy exists. */
export async function fetchPdfium(platform, log = console.log) {
  const asset = pdfium.assets[platform];
  if (!asset) throw new Error(`no PDFium pin for ${platform}`);
  const dir = vendorDir(platform);
  const marker = path.join(dir, '.sha256');
  if (fs.existsSync(marker) && fs.readFileSync(marker, 'utf8') === asset.sha256) {
    return { dir, library: libraryPath(platform), fetched: false };
  }

  const res = await fetch(asset.url);
  if (!res.ok) throw new Error(`PDFium: HTTP ${res.status} for ${asset.url}`);
  const bytes = Buffer.from(await res.arrayBuffer());
  const digest = createHash('sha256').update(bytes).digest('hex');
  if (digest !== asset.sha256) {
    throw new Error(`PDFium: SHA-256 mismatch (expected ${asset.sha256}, got ${digest})`);
  }

  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  const archive = path.join(dir, path.basename(new URL(asset.url).pathname));
  fs.writeFileSync(archive, bytes);
  // bsdtar (macOS, Windows) and GNU tar (Linux) both read .tgz.
  execFileSync('tar', ['-xf', archive, '-C', dir]);
  fs.rmSync(archive);

  // The notices the bundles ship as licenses/pdfium/. The archive's root
  // LICENSE is the MIT licence of bblanchon's build scripts; PDFium's own
  // BSD-3-Clause licence is licenses/pdfium.txt.
  const notices = path.join(dir, 'notices');
  fs.mkdirSync(notices);
  fs.copyFileSync(path.join(dir, 'LICENSE'), path.join(notices, 'LICENSE-pdfium-binaries.txt'));
  for (const name of fs.readdirSync(path.join(dir, 'licenses')).sort()) {
    fs.copyFileSync(path.join(dir, 'licenses', name), path.join(notices, name));
  }
  if (!fs.existsSync(libraryPath(platform))) {
    throw new Error(`PDFium: expected ${libraryPath(platform)} after unpacking`);
  }
  // Written last, so an interrupted unpack is fetched again.
  fs.writeFileSync(marker, asset.sha256);
  log(
    `fetch-pdfium: ${pdfium.version} for ${platform} (${(bytes.length / 1048576).toFixed(1)} MiB)`,
  );
  return { dir, library: libraryPath(platform), fetched: true };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const i = args.indexOf('--platform');
  const printPath = args.includes('--print-path');
  try {
    const platform = i === -1 ? platformKey() : args[i + 1];
    const got = await fetchPdfium(platform, printPath ? () => {} : console.log);
    if (printPath) console.log(got.library);
    else if (!got.fetched)
      console.log(`fetch-pdfium: ${pdfium.version} for ${platform} already present`);
  } catch (err) {
    console.error(`fetch-pdfium: ${err.message}`);
    process.exit(1);
  }
}
