#!/usr/bin/env node
// Runs the Phase 0 benchmark suite (phase0-spec §13; docs/benchmarks.md):
// generates fixtures if needed, runs `cargo bench --bench inventory` for
// each scenario, and writes JSON results plus a Markdown summary.
//
//   node scripts/bench.mjs [--out <dir>] [--sweep]
//
// --sweep adds the 30k throttle sweep (about 10 minutes) used to choose
// the cataloging rate. Fixtures go in <out>/fixtures and are reused.

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1];
};
const out = path.resolve(opt('out', path.join(os.tmpdir(), 'atlas-bench')));
const sweep = args.includes('--sweep');
fs.mkdirSync(path.join(out, 'fixtures'), { recursive: true });

function fixture(name, files, flat) {
  const dir = path.join(out, 'fixtures', name);
  const meta = `${dir}.json`;
  if (!fs.existsSync(meta)) {
    const text = execFileSync(process.execPath, [
      path.join(repo, 'scripts', 'generate-fixtures.mjs'),
      dir,
      '--files',
      String(files),
      '--flat',
      String(flat),
    ]).toString();
    fs.writeFileSync(meta, text.split('\nTo delete later')[0]);
  }
  return { dir, expect: JSON.parse(fs.readFileSync(meta, 'utf8')).catalogableFiles };
}

const small = fixture('1k', 1000, 0);
// §11: 30,000 files including one flat folder of 20,000.
const large = fixture('30k', 10000, 20000);

const scenarios = [
  {
    id: '1k-ac',
    f: small,
    label: '1k',
    rates: 'policy,unthrottled',
    power: 'ac',
    extra: ['--browse', '5', '--pause', '--recovery'],
  },
  { id: '1k-battery', f: small, label: '1k', rates: 'policy', power: 'battery:50', extra: [] },
  {
    id: '30k-ac',
    f: large,
    label: '30k',
    rates: 'policy,unthrottled',
    power: 'ac',
    extra: ['--browse', '5', '--pause', '--recovery'],
  },
];
if (sweep) {
  scenarios.push({
    id: '30k-sweep',
    f: large,
    label: '30k',
    rates: 'unthrottled,1000,500,250,100',
    power: 'ac',
    extra: [],
  });
}

// Build once, outside the timings.
execFileSync(
  'cargo',
  [
    'bench',
    '--manifest-path',
    path.join(repo, 'src-tauri', 'Cargo.toml'),
    '--locked',
    '--bench',
    'inventory',
    '--no-run',
  ],
  { stdio: 'inherit' },
);

const results = [];
for (const s of scenarios) {
  const file = path.join(out, `bench-${s.id}.json`);
  console.error(`bench: ${s.id}`);
  execFileSync(
    'cargo',
    [
      'bench',
      '--manifest-path',
      path.join(repo, 'src-tauri', 'Cargo.toml'),
      '--locked',
      '--bench',
      'inventory',
      '--',
      '--fixture',
      s.f.dir,
      '--label',
      s.label,
      '--rates',
      s.rates,
      '--power',
      s.power,
      '--expect',
      String(s.f.expect),
      '--out',
      file,
      ...s.extra,
    ],
    { stdio: ['ignore', 'ignore', 'inherit'] },
  );
  results.push({ id: s.id, ...JSON.parse(fs.readFileSync(file, 'utf8')) });
}

const md = summarize(results);
fs.writeFileSync(path.join(out, `bench-${process.platform}.md`), md);
console.log(md);
if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, md);

function summarize(all) {
  const m = all[0].machine;
  const mib = (b) => (b / 1048576).toFixed(1);
  const ms = (v) => (v == null ? '–' : String(v));
  const lines = [
    `## Benchmarks: ${m.os} ${m.arch}${m.ci ? ' (CI runner)' : ''}`,
    '',
    `${m.osVersion ?? ''} · ${m.cpu ?? 'unknown CPU'} · ${m.logicalCores} logical cores · ${mib(m.totalMemoryBytes)} MiB RAM`,
    '',
    '| Scenario | Rate | Files | Wall s | Files/s | Scan CPU % of a core | Peak RSS MiB | First rows ms | Browse p95 ms (while scanning) | Warm p95 ms | Cold first page ms | DB+WAL MiB (closed) |',
    '| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |',
  ];
  for (const r of all) {
    for (const run of r.runs) {
      const closed = run.catalogClosed;
      lines.push(
        `| ${r.id} | ${run.rate} | ${run.files} | ${run.wallSeconds} | ${run.filesPerSecond} | ${run.scanCpuPercentOfOneCore} | ${mib(run.sampledPeakRssBytes)} | ${ms(run.firstRowsMs)} | ${ms(run.browseWhileScanning?.p95Ms)} | ${run.warmQueries.p95Ms} | ${run.coldFirstPageMs} | ${mib(closed.dbBytes + closed.walBytes)} |`,
      );
    }
  }
  const rec = all.filter((r) => r.recovery);
  if (rec.length) {
    lines.push(
      '',
      '| Recovery after kill | Files at kill | Resume to complete s | Duplicate rows |',
      '| --- | ---: | ---: | ---: |',
    );
    for (const r of rec) {
      lines.push(
        `| ${r.id} | ${r.recovery.filesAtKill} | ${r.recovery.resumeToCompleteSeconds} | ${r.recovery.duplicateRows} |`,
      );
    }
  }
  const paused = all.flatMap((r) =>
    r.runs.filter((x) => x.pauseResume).map((x) => ({ id: r.id, rate: x.rate, ...x.pauseResume })),
  );
  if (paused.length) {
    lines.push(
      '',
      '| Pause/resume | Rate | Files written after pause | Resume to next write ms |',
      '| --- | --- | ---: | ---: |',
    );
    for (const p of paused)
      lines.push(
        `| ${p.id} | ${p.rate} | ${p.filesWrittenAfterPause} | ${p.resumeToNextWriteMs} |`,
      );
  }
  return `${lines.join('\n')}\n`;
}
