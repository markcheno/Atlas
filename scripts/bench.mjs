#!/usr/bin/env node
// Runs the benchmark suite (phase0-spec §13, phase1-spec §13;
// docs/benchmarks.md): generates fixtures if needed, runs `cargo bench
// --bench inventory` for each cataloging scenario, `cargo bench --bench
// search` on the 30k text fixture and `cargo bench --bench extraction` on
// the same fixture, and writes JSON results plus a Markdown summary.
//
//   node scripts/bench.mjs [--out <dir>] [--sweep] [--search-only] [--extraction-only]
//                          [--search-size 30k|1k] [--extraction-size 30k|1k]
//                          [--max-p95-ms 250]
//
// --search-size picks the search fixture: 30k (default; the P1-19 reference
// measurement, run locally) or 1k (CI: the 30k text fixture takes over 40
// minutes on Windows runners). --extraction-size does the same for the
// extraction benchmark (default: the search size); at 1k it also reads at
// the Slow duty cycle (simulated battery), which takes too long at 30k.
// --sweep adds the 30k throttle sweep (about 10 minutes) used to choose
// the cataloging rate. --search-only runs just the search benchmark,
// --extraction-only just the extraction benchmark; --evidence is the Phase 1
// close-out's run (`just ci-remote evidence`): extraction only, on the 30k
// text fixture, unthrottled, failing unless every file is read;
// --max-p95-ms makes it fail above that search p95 while reading (the
// reference machine's check, P1-19). Fixtures go in <out>/fixtures and are
// reused.

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
const searchOnly = args.includes('--search-only');
const evidence = args.includes('--evidence');
const extractionOnly = evidence || args.includes('--extraction-only');
if (searchOnly && extractionOnly)
  throw new Error('--search-only and --extraction-only exclude each other');
const runInventory = !searchOnly && !extractionOnly;
const runSearch = !extractionOnly;
const runExtraction = !searchOnly;
const maxP95 = opt('max-p95-ms', null);
const searchSize = opt('search-size', '30k');
if (!['30k', '1k'].includes(searchSize)) throw new Error('--search-size is 30k or 1k');
const extractionSize = evidence ? '30k' : opt('extraction-size', searchSize);
if (!['30k', '1k'].includes(extractionSize)) throw new Error('--extraction-size is 30k or 1k');
fs.mkdirSync(path.join(out, 'fixtures'), { recursive: true });

function fixture(name, files, flat, textKb = 0, copies = 0) {
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
      ...(textKb ? ['--text-kb', String(textKb)] : []),
      ...(copies ? ['--copies', String(copies)] : []),
    ]).toString();
    fs.writeFileSync(meta, text.split('\nTo delete later')[0]);
  }
  return { dir, expect: JSON.parse(fs.readFileSync(meta, 'utf8')).catalogableFiles };
}

const small = runInventory ? fixture('1k', 1000, 0) : null;
// §11: 30,000 files including one flat folder of 20,000.
const large = runInventory ? fixture('30k', 10000, 20000) : null;
// Phase 1 §11: the same layout with about 20 KiB of text per file, plus
// identical copies (Slice 1.6) so Library › Duplicates has groups to list.
const textFixture = (size) =>
  size === '30k'
    ? fixture('30k-text-copies', 10000, 20000, 20, 2000)
    : fixture('1k-text-copies', 1000, 0, 20, 100);
const searchLabel = `${searchSize}-text`;
const searchFixture = runSearch ? textFixture(searchSize) : null;
const extractionLabel = `${extractionSize}-text`;
const extractionFixture = runExtraction ? textFixture(extractionSize) : null;

const scenarios = !runInventory
  ? []
  : [
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
if (sweep && runInventory) {
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
    '--bench',
    'search',
    '--bench',
    'extraction',
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

let searchResult = null;
if (runSearch) {
  console.error(`bench: ${searchLabel} search`);
  const searchFile = path.join(out, `bench-${searchLabel}-search.json`);
  execFileSync(
    'cargo',
    [
      'bench',
      '--manifest-path',
      path.join(repo, 'src-tauri', 'Cargo.toml'),
      '--locked',
      '--bench',
      'search',
      '--',
      '--fixture',
      searchFixture.dir,
      '--label',
      searchLabel,
      '--expect',
      String(searchFixture.expect),
      '--out',
      searchFile,
      ...(maxP95 ? ['--max-p95-ms', String(maxP95)] : []),
    ],
    { stdio: ['ignore', 'ignore', 'inherit'] },
  );
  searchResult = JSON.parse(fs.readFileSync(searchFile, 'utf8'));
}

let extractionResult = null;
if (runExtraction) {
  console.error(`bench: ${extractionLabel} extraction`);
  const extractionFile = path.join(out, `bench-${extractionLabel}-extraction.json`);
  execFileSync(
    'cargo',
    [
      'bench',
      '--manifest-path',
      path.join(repo, 'src-tauri', 'Cargo.toml'),
      '--locked',
      '--bench',
      'extraction',
      '--',
      '--fixture',
      extractionFixture.dir,
      '--label',
      extractionLabel,
      '--duty',
      evidence
        ? 'unthrottled'
        : extractionSize === '30k'
          ? 'run,unthrottled'
          : 'run,slow,unthrottled',
      ...(evidence ? ['--require-all-read'] : ['--recovery']),
      '--expect',
      String(extractionFixture.expect),
      '--out',
      extractionFile,
    ],
    { stdio: ['ignore', 'ignore', 'inherit'] },
  );
  extractionResult = JSON.parse(fs.readFileSync(extractionFile, 'utf8'));
}

const md =
  (results.length ? summarize(results) : '') +
  (searchResult ? summarizeSearch(searchResult) : '') +
  (extractionResult ? summarizeExtraction(extractionResult) : '');
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
  const browsed = all.flatMap((r) =>
    r.runs.filter((x) => x.browseByQuery).map((x) => ({ id: r.id, rate: x.rate, ...x })),
  );
  if (browsed.length) {
    const kinds = Object.keys(browsed[0].browseByQuery);
    lines.push(
      '',
      `| Browse p95 / max ms while scanning | Rate | ${kinds.join(' | ')} | folder probe |`,
      `| --- | --- | ${kinds.map(() => '---:').join(' | ')} | ---: |`,
    );
    const cell = (p) => (p ? `${p.p95Ms} / ${p.maxMs}` : '–');
    for (const b of browsed)
      lines.push(
        `| ${b.id} | ${b.rate} | ${kinds.map((k) => cell(b.browseByQuery[k])).join(' | ')} | ${cell(b.rootProbeWhileScanning)} |`,
      );
  }
  return `${lines.join('\n')}\n`;
}

function summarizeSearch(r) {
  const mib = (b) => (b == null ? '–' : (b / 1048576).toFixed(1));
  const cell = (p) => (p ? `${p.p50Ms} / ${p.p95Ms} / ${p.maxMs}` : '–');
  const lines = [
    '',
    `## Search: ${r.label} (${r.machine.os} ${r.machine.arch}${r.machine.ci ? ', CI runner' : ''})`,
    '',
    '| Files | Passages | Text MiB | Read s | First searchable s | Index MiB | Catalog MiB | Rebuild s |',
    '| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |',
    `| ${r.files} | ${r.passages} | ${mib(r.textChars)} | ${r.readSeconds} | ${r.firstSearchableSeconds ?? '–'} | ${mib(r.indexBytes)} | ${mib(r.catalogBytes)} | ${r.rebuildSeconds} |`,
    '',
    '| search_documents p50 / p95 / max ms | While reading | Idle (warm) |',
    '| --- | ---: | ---: |',
    `| All queries | ${cell(r.whileReading.all)} | ${cell(r.idle.all)} |`,
  ];
  for (const k of Object.keys(r.whileReading.byQuery)) {
    lines.push(`| ${k} | ${cell(r.whileReading.byQuery[k])} | ${cell(r.idle.byQuery[k])} |`);
  }
  lines.push(
    '',
    `Cold (first query after reopening): ${r.cold.map((c) => `${c.query} ${c.ms} ms`).join(', ')}.`,
  );
  if (r.duplicates) {
    lines.push(
      '',
      '| Idle, p50 / p95 / max ms | Latency | |',
      '| --- | ---: | --- |',
      `| list_duplicates (first page) | ${cell(r.duplicates.latency)} | ${r.duplicates.groups} groups |`,
      `| get_storage_usage | ${cell(r.storage.latency)} | text ${mib(r.storage.textBytes)} MiB, index ${mib(r.storage.indexBytes)} MiB, catalog ${mib(r.storage.catalogBytes)} MiB |`,
    );
  }
  return `${lines.join('\n')}\n`;
}

function summarizeExtraction(r) {
  const mib = (b) => (b == null ? '–' : (b / 1048576).toFixed(1));
  const lines = [
    '',
    `## Text extraction: ${r.label} (${r.machine.os} ${r.machine.arch}${r.machine.ci ? ', CI runner' : ''})`,
    '',
    '| Duty cycle | Files | PDF pages | Passages | Read s | Files/s | Pages/s | Passages/s | First searchable s | App CPU % | Worker CPU % | App / worker peak RSS MiB | Worker starts | Peak WAL MiB | Policy seen |',
    '| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |',
  ];
  for (const x of r.runs) {
    lines.push(
      `| ${x.duty} | ${x.files} | ${x.pdfPages} | ${x.passages} | ${x.readSeconds} | ${x.filesPerSecond} | ${x.pagesPerSecond} | ${x.passagesPerSecond} | ${x.firstSearchableSeconds ?? '–'} | ${x.appCpuPercentOfOneCore} | ${x.workerCpuPercentOfOneCore} | ${x.appPeakRssMiB} / ${x.workerPeakRssMiB} | ${x.workerSpawns} | ${x.peakWalMiB} | ${Object.entries(
        x.policySeen ?? {},
      )
        .map(([k, n]) => `${k} ${n}`)
        .join(', ')} |`,
    );
  }
  lines.push(
    '',
    "| Text status after reading | Files | Text ready | Waiting | Needs OCR | Encrypted | Only in the cloud | Couldn't read |",
    '| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |',
  );
  for (const x of r.runs) {
    const c = x.countsByStatus;
    if (c)
      lines.push(
        `| ${x.duty} | ${c.total} | ${c.done} | ${c.pending} | ${c.needsOcr} | ${c.encrypted} | ${c.cloudOnly} | ${c.unreadable} |`,
      );
  }
  const g = r.runs[0].growth;
  lines.push(
    '',
    '| Catalog growth | Page size | Text stored MiB | Catalog MiB | Text pages MiB | Index MiB | Other MiB | Catalog ÷ text |',
    '| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |',
    `| ${r.runs[0].duty} | ${g.pageSize} | ${mib(g.textStoredBytes)} | ${mib(g.catalogBytes)} | ${mib(g.textBytes)} | ${mib(g.indexBytes)} | ${mib(g.otherBytes)} | ${g.catalogPerText} |`,
  );
  if (r.recovery) {
    const k = r.recovery;
    lines.push(
      '',
      '| Kill mid-read | Documents at kill | Partial documents | Workers left after kill | Resume to complete s | Duplicate documents | Still waiting |',
      '| --- | ---: | ---: | ---: | ---: | ---: | ---: |',
      `| ${r.label} | ${k.documentsAtKill} | ${k.partialDocumentsAtKill} | ${k.workersLeftAfterKill} of ${k.workersAtKill} | ${k.resumeToCompleteSeconds} | ${k.duplicateDocuments} | ${k.pendingAfterResume} |`,
    );
  }
  return `${lines.join('\n')}\n`;
}
