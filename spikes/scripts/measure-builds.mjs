#!/usr/bin/env node
// Build cost of each spike candidate (docs/spike-0.9.md): a clean release
// build of one binary in its own fresh target directory, after `cargo
// fetch` (so download time doesn't count), plus binary sizes as built and
// stripped. Each candidate is compared with its workspace's baseline,
// which links what the app already links.
//
//   node spikes/scripts/measure-builds.mjs [--out <dir>] [--only vec-lancedb,…]
//
// Nothing else should be running. Writes builds-<platform>.json and .md.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const spikes = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1];
};
const out = path.resolve(opt('out', path.join(os.tmpdir(), 'atlas-spikes')));
const only = opt('only', null)?.split(',');
// Short paths on Windows (LanceDB's build tree is deep).
const scratch = process.platform === 'win32' ? 'C:\\b' : path.join(os.tmpdir(), 'atlas-spike-builds');

const TARGETS = [
  { workspace: 'extraction-bench', pkg: 'baseline', bin: 'baseline', role: 'baseline' },
  { workspace: 'extraction-bench', pkg: 'pdf-pdfium', bin: 'pdf-pdfium' },
  { workspace: 'extraction-bench', pkg: 'pdf-extract-bin', bin: 'pdf-extract' },
  { workspace: 'extraction-bench', pkg: 'epub-rbook', bin: 'epub-rbook' },
  { workspace: 'vector-bench', pkg: 'baseline', bin: 'baseline', role: 'baseline' },
  { workspace: 'vector-bench', pkg: 'vec-sqlite', bin: 'vec-sqlite' },
  { workspace: 'vector-bench', pkg: 'vec-sqlite-simd', bin: 'vec-sqlite-simd' },
  { workspace: 'vector-bench', pkg: 'vec-usearch', bin: 'vec-usearch' },
  { workspace: 'vector-bench', pkg: 'vec-lancedb', bin: 'vec-lancedb' },
];

const env = { ...process.env, CARGO_INCREMENTAL: '0' };
delete env.RUSTC_WRAPPER;
try {
  const p = JSON.parse(fs.readFileSync(path.join(spikes, '.tools', 'paths.json'), 'utf8'));
  if (p.protoc) env.PROTOC = p.protoc.bin;
} catch {
  // protoc is only needed for LanceDB
}

function strip(file) {
  if (process.platform === 'win32') return fs.statSync(file).size; // debug info lives in the .pdb
  const copy = `${file}.stripped`;
  fs.copyFileSync(file, copy);
  const r = spawnSync('strip', process.platform === 'darwin' ? ['-x', copy] : ['--strip-all', copy]);
  const size = r.status === 0 ? fs.statSync(copy).size : null;
  fs.rmSync(copy);
  return size;
}

function depCount(workspace, pkg) {
  const r = spawnSync('cargo', ['tree', '--locked', '-e', 'normal,build', '-p', pkg, '--prefix', 'none', '--no-dedupe'], {
    cwd: path.join(spikes, workspace),
    encoding: 'utf8',
    maxBuffer: 64 << 20,
  });
  return r.status === 0 ? new Set(r.stdout.split('\n').filter(Boolean).map((l) => l.split(' (')[0].trim())).size : null;
}

fs.mkdirSync(out, { recursive: true });
const results = [];
for (const t of TARGETS) {
  if (only && !only.includes(t.pkg) && t.role !== 'baseline') continue;
  const cwd = path.join(spikes, t.workspace);
  spawnSync('cargo', ['fetch', '--locked'], { cwd, env, stdio: 'inherit' });
  const target = path.join(scratch, `${t.workspace}-${t.pkg}`);
  fs.rmSync(target, { recursive: true, force: true });
  console.log(`builds: ${t.workspace}/${t.pkg}…`);
  const started = Date.now();
  const r = spawnSync('cargo', ['build', '--release', '--locked', '--offline', '-p', t.pkg, '--bin', t.bin], {
    cwd,
    env: { ...env, CARGO_TARGET_DIR: target },
    stdio: ['ignore', 'ignore', 'pipe'],
    encoding: 'utf8',
    maxBuffer: 64 << 20,
  });
  const seconds = (Date.now() - started) / 1000;
  const exe = path.join(target, 'release', process.platform === 'win32' ? `${t.bin}.exe` : t.bin);
  const ok = r.status === 0 && fs.existsSync(exe);
  const row = {
    ...t,
    ok,
    seconds,
    binaryBytes: ok ? fs.statSync(exe).size : null,
    strippedBytes: ok ? strip(exe) : null,
    crates: depCount(t.workspace, t.pkg),
    error: ok ? undefined : r.stderr.trim().split('\n').slice(-5).join('\n'),
  };
  console.log(`  ${ok ? 'ok' : 'FAILED'} in ${seconds.toFixed(0)} s${ok ? `, ${(row.binaryBytes / 2 ** 20).toFixed(1)} MiB` : ''}`);
  results.push(row);
  fs.rmSync(target, { recursive: true, force: true });
}

const machine = { os: `${os.type()} ${os.release()}`, platform: process.platform, arch: process.arch, cpu: os.cpus()[0]?.model, cores: os.cpus().length, ci: Boolean(process.env.CI) };
const lines = [`## Build cost: ${machine.os} ${machine.arch}`, '', `${machine.cpu} · ${machine.cores} logical cores · clean release build, one binary per fresh target directory, after cargo fetch`, ''];
lines.push('| Workspace | Binary | Clean build s | Δ vs baseline s | Binary MiB | Stripped MiB | Δ stripped MiB | Crates |');
lines.push('| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |');
const mib = (b) => (b === null || b === undefined ? '–' : (b / 2 ** 20).toFixed(1));
for (const r of results) {
  const base = results.find((x) => x.workspace === r.workspace && x.role === 'baseline');
  const dt = r.role === 'baseline' || !base ? '–' : (r.seconds - base.seconds).toFixed(0);
  const ds = r.role === 'baseline' || !base?.strippedBytes || !r.strippedBytes ? '–' : mib(r.strippedBytes - base.strippedBytes);
  lines.push(`| ${r.workspace} | ${r.bin} | ${r.ok ? r.seconds.toFixed(0) : 'failed'} | ${dt} | ${mib(r.binaryBytes)} | ${mib(r.strippedBytes)} | ${ds} | ${r.crates ?? '–'} |`);
}
const md = lines.join('\n') + '\n';
fs.writeFileSync(path.join(out, `builds-${process.platform}.json`), JSON.stringify({ machine, results }, null, 2));
fs.writeFileSync(path.join(out, `builds-${process.platform}.md`), md);
if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, md + '\n');
console.log(md);
