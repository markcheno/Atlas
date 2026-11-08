#!/usr/bin/env node
// Vector-engine spike driver (docs/spike-0.9.md).
//
//   node spikes/scripts/run-vectors.mjs [--n 100000] [--queries 1000] [--out <dir>]
//                                       [--only vec-usearch,…] [--alpha] [--no-build]
//
// Builds spikes/vector-bench (release), generates the data set once per size
// into <out>/data-<n>, runs every engine variant one after another (nothing
// else should be running), runs the crash test on a 100k set, and writes
// vectors-<platform>-<n>.json and .md into <out>.
//
// The sqlite-vec DiskANN alpha is informational and builds its index
// slowly; it runs only with --alpha.

import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const spikes = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const bench = path.join(spikes, 'vector-bench');
const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1];
};
const flag = (name) => args.includes(`--${name}`);

const n = Number(opt('n', 100_000));
const queries = Number(opt('queries', n > 200_000 ? 200 : 1000));
const out = path.resolve(opt('out', path.join(os.tmpdir(), 'atlas-spikes')));
const only = opt('only', null)?.split(',');
const targetDir = process.env.CARGO_TARGET_DIR
  ? path.resolve(bench, process.env.CARGO_TARGET_DIR)
  : path.join(bench, 'target');
const exe = (name) =>
  path.join(targetDir, 'release', process.platform === 'win32' ? `${name}.exe` : name);

const RUNS = [
  ['baseline', null],
  ['vec-sqlite', 'f32'],
  ['vec-sqlite', 'int8'],
  ['vec-sqlite', 'bit'],
  ['vec-sqlite-simd', 'f32-l2'],
  ['vec-sqlite-simd', 'int8-l2'],
  ['vec-sqlite-simd', 'bit'],
  ['vec-sqlite-alpha', 'diskann'],
  ['vec-usearch', 'f16'],
  ['vec-usearch', 'i8'],
  ['vec-lancedb', 'ivfpq'],
  ['vec-lancedb', 'hnswsq'],
];
// One crash test per storage design.
const CRASH = [
  ['baseline', null],
  ['vec-sqlite', 'f32'],
  ['vec-usearch', 'f16'],
  ['vec-lancedb', 'ivfpq'],
];

fs.mkdirSync(out, { recursive: true });

function freeGiB(dir) {
  try {
    const s = fs.statfsSync(dir);
    return (s.bavail * s.bsize) / 2 ** 30;
  } catch {
    return Infinity;
  }
}

function toolEnv() {
  const env = { ...process.env };
  const paths = path.join(spikes, '.tools', 'paths.json');
  if (fs.existsSync(paths)) {
    const p = JSON.parse(fs.readFileSync(paths, 'utf8'));
    if (p.protoc) env.PROTOC = p.protoc.bin;
  }
  return env;
}

function build() {
  console.log('vectors: building (release)…');
  const r = spawnSync('cargo', ['build', '--release', '--locked', '--workspace'], {
    cwd: bench,
    env: toolEnv(),
    stdio: 'inherit',
  });
  if (r.status !== 0) throw new Error('cargo build failed');
}

function datagen(size) {
  const dir = path.join(out, `data-${size}`);
  if (fs.existsSync(path.join(dir, 'meta.json'))) return dir;
  const need = (size * 1024 * 4 * 2.2) / 2 ** 30;
  if (freeGiB(out) < need + 2) throw new Error(`need about ${need.toFixed(1)} GiB free in ${out}`);
  console.log(`vectors: generating ${size.toLocaleString('en-US')} vectors…`);
  const r = spawnSync(exe('datagen'), ['--out', dir, '--n', String(size), '--queries', '1000'], {
    stdio: 'inherit',
  });
  if (r.status !== 0) throw new Error('datagen failed');
  return dir;
}

function runOne(bin, variant, data, size) {
  const index = path.join(out, `index-${bin}-${variant ?? 'default'}-${size}`);
  const a = ['bench', '--data', data, '--index', index, '--queries', String(queries)];
  if (variant) a.push('--variant', variant);
  const started = Date.now();
  const r = spawnSync(exe(bin), a, {
    encoding: 'utf8',
    maxBuffer: 64 << 20,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  fs.rmSync(index, { recursive: true, force: true });
  const lines = (r.stdout ?? '').trim().split('\n');
  if (r.status !== 0) {
    return {
      bin,
      variant,
      ok: false,
      error: (r.stderr ?? '').trim().split('\n').slice(-3).join(' '),
      wallSeconds: (Date.now() - started) / 1000,
    };
  }
  return {
    bin,
    variant,
    ok: true,
    ...JSON.parse(lines[lines.length - 1]),
    wallSeconds: (Date.now() - started) / 1000,
  };
}

// Kill -9 an insert after a few durable steps, then reopen.
function crashOne(bin, variant, data) {
  const index = path.join(out, `crash-${bin}-${variant ?? 'default'}`);
  const base = ['--data', data, '--index', index, ...(variant ? ['--variant', variant] : [])];
  return new Promise((resolve) => {
    const child = spawn(exe(bin), ['crash-insert', ...base], {
      stdio: ['ignore', 'pipe', 'ignore'],
    });
    let committed = 0;
    let buffer = '';
    let killedAt = null;
    child.stdout.on('data', (d) => {
      buffer += d.toString();
      for (const m of buffer.matchAll(/committed (\d+)/g)) committed = Number(m[1]);
      buffer = buffer.slice(-200);
      // Kill while the next step is underway: wait for the third commit,
      // then a short random delay.
      if (killedAt === null && committed >= 3000) {
        killedAt = committed;
        setTimeout(() => child.kill('SIGKILL'), 50 + Math.floor(Math.random() * 200));
      }
    });
    child.on('exit', () => {
      const v = spawnSync(exe(bin), ['crash-verify', ...base], { encoding: 'utf8' });
      const lines = (v.stdout ?? '').trim().split('\n');
      let verify;
      try {
        verify = JSON.parse(lines[lines.length - 1]);
      } catch {
        verify = { usable: false, error: (v.stderr ?? '').trim().split('\n').slice(-2).join(' ') };
      }
      fs.rmSync(index, { recursive: true, force: true });
      resolve({ bin, variant, committedBeforeKill: committed, ...verify });
    });
  });
}

function binarySizes() {
  const sizes = {};
  for (const bin of new Set(RUNS.map((r) => r[0]))) {
    const f = exe(bin);
    if (fs.existsSync(f)) sizes[bin] = fs.statSync(f).size;
  }
  return sizes;
}

const fmt = (x, d = 1) =>
  x === null || x === undefined || Number.isNaN(x) ? '–' : Number(x).toFixed(d);

function markdown(report) {
  const rows = [];
  rows.push(
    `## Vector engines: ${report.machine.os} ${report.machine.arch}, ${report.n.toLocaleString('en-US')} × ${report.dim}`,
  );
  rows.push('');
  rows.push(
    `${report.machine.cpu} · ${report.machine.cores} logical cores · ${fmt(report.machine.memGiB, 0)} GiB RAM · ${report.queries} queries, single thread, k = 10`,
  );
  rows.push('');
  rows.push(
    '| Engine | Variant | Build s | Disk / raw | Open → first answer ms | Best setting ≥ 0.95 recall | Recall | p95 ms | Filter 40% recall / p95 | Filter 5% recall / p95 | Filter 0.5% recall / p95 | Peak RSS MiB |',
  );
  rows.push('| --- | --- | ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |');
  for (const r of report.runs) {
    if (!r.ok) {
      rows.push(
        `| ${r.bin} | ${r.variant ?? ''} | failed: ${r.error.replace(/\|/g, '/')} | | | | | | | | | |`,
      );
      continue;
    }
    const best = r.sweep.find((p) => p.recall >= 0.95) ?? r.sweep[r.sweep.length - 1];
    const f = (i) =>
      r.filtered[i]
        ? `${fmt(r.filtered[i].recall, 3)} / ${fmt(r.filtered[i].latency.p95Ms, 2)}`
        : '–';
    rows.push(
      `| ${r.engine.name} | ${r.engine.label ?? r.variant ?? r.engine.variant} | ${fmt(r.buildSeconds)} | ${fmt(r.diskOverRaw, 2)} | ${fmt(r.openToFirstAnswerMs, 1)} | ${best.knob === 'exact' || best.knob.startsWith('nprobes=') ? best.knob : `${best.knob} ${best.value}`} | ${fmt(best.recall, 3)} | ${fmt(best.latency.p95Ms, 2)} | ${f(0)} | ${f(1)} | ${f(2)} | ${fmt(r.peakRssBytes / 2 ** 20, 0)} |`,
    );
  }
  rows.push('');
  rows.push('**Recall–latency sweeps** (setting: recall@10 / p95 ms)');
  rows.push('');
  for (const r of report.runs.filter((x) => x.ok && x.sweep.length > 1)) {
    rows.push(
      `- ${r.engine.name} ${r.engine.label ?? ''}: ${r.sweep.map((p) => `${p.knob === 'exact' ? '' : p.knob.startsWith('nprobes=') ? p.knob : `${p.knob} ${p.value}`}: ${fmt(p.recall, 3)} / ${fmt(p.latency.p95Ms, 2)}`).join(' · ')}`,
    );
  }
  rows.push('');
  rows.push(
    '| Deletes (1%) | Variant | Delete ms | Deleted ids returned | Disk after / before | Compacted to |',
  );
  rows.push('| --- | --- | ---: | ---: | ---: | ---: |');
  for (const r of report.runs.filter((x) => x.ok)) {
    const d = r.delete;
    rows.push(
      `| ${r.engine.name} | ${r.engine.label ?? r.variant ?? ''} | ${fmt(d.ms, 0)} | ${d.deletedIdsReturned} | ${fmt(d.diskBytesAfter / r.diskBytes, 2)} | ${d.compacted ? `${fmt(d.diskBytesAfterCompact / r.diskBytes, 2)} (${fmt(d.compactMs, 0)} ms)` : 'no compaction'} |`,
    );
  }
  if (report.crash?.length) {
    rows.push('');
    rows.push(
      '| Kill -9 during insert | Variant | Committed before kill | Usable after reopen | Rows after reopen | Note |',
    );
    rows.push('| --- | --- | ---: | --- | ---: | --- |');
    for (const c of report.crash) {
      rows.push(
        `| ${c.bin} | ${c.variant ?? ''} | ${c.committedBeforeKill} | ${c.usable ? 'yes' : 'no'} | ${c.rows ?? '–'} | ${c.integrity ?? c.error ?? (c.tornBytes ? `${c.tornBytes} torn bytes` : '')} |`,
      );
    }
  }
  rows.push('');
  rows.push('| Binary | Size MiB | Delta over baseline MiB |');
  rows.push('| --- | ---: | ---: |');
  for (const [bin, size] of Object.entries(report.binaryBytes)) {
    rows.push(
      `| ${bin} | ${fmt(size / 2 ** 20)} | ${bin === 'baseline' ? '–' : fmt((size - report.binaryBytes.baseline) / 2 ** 20)} |`,
    );
  }
  return rows.join('\n') + '\n';
}

async function main() {
  if (!flag('no-build')) build();
  const data = datagen(n);
  const meta = JSON.parse(fs.readFileSync(path.join(data, 'meta.json'), 'utf8'));
  const report = {
    n,
    dim: meta.dim,
    queries,
    data: meta,
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
    runs: [],
    crash: [],
    binaryBytes: binarySizes(),
  };
  for (const [bin, variant] of RUNS) {
    if (only && !only.includes(bin)) continue;
    if (bin === 'vec-sqlite-alpha' && !flag('alpha')) continue;
    if (!fs.existsSync(exe(bin))) continue;
    console.log(`vectors: ${bin} ${variant ?? ''} at ${n.toLocaleString('en-US')}…`);
    const r = runOne(bin, variant, data, n);
    console.log(r.ok ? `  ok in ${fmt(r.wallSeconds, 0)} s` : `  FAILED: ${r.error}`);
    report.runs.push(r);
  }
  if (!flag('no-crash')) {
    const crashData = datagen(Math.min(n, 100_000));
    for (const [bin, variant] of CRASH) {
      if (only && !only.includes(bin)) continue;
      if (!fs.existsSync(exe(bin))) continue;
      console.log(`vectors: crash test ${bin} ${variant ?? ''}…`);
      report.crash.push(await crashOne(bin, variant, crashData));
    }
  }
  const stem = `vectors-${process.platform}-${n}`;
  fs.writeFileSync(path.join(out, `${stem}.json`), JSON.stringify(report, null, 2));
  const md = markdown(report);
  fs.writeFileSync(path.join(out, `${stem}.md`), md);
  if (process.env.GITHUB_STEP_SUMMARY)
    fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, md + '\n');
  console.log(md);
  console.log(`vectors: wrote ${path.join(out, stem)}.{json,md}`);
}

main().catch((e) => {
  console.error(`vectors: ${e.message}`);
  process.exit(1);
});
