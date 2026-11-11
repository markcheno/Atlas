#!/usr/bin/env node
// Runs the benchmark suite (phase0-spec §13, phase1-spec §13;
// docs/benchmarks.md): generates fixtures if needed, runs `cargo bench
// --bench inventory` for each cataloging scenario, `cargo bench --bench
// search` on the 30k text fixture, `cargo bench --bench extraction` on
// the same fixture and `cargo bench --bench vectors` on synthetic vectors,
// and writes JSON results plus a Markdown summary.
//
//   node scripts/bench.mjs [--out <dir>] [--sweep] [--search-only] [--extraction-only]
//                          [--vectors-only] [--embedding-only] [--search-size 30k|1k]
//                          [--extraction-size 30k|1k] [--vectors-size 100000]
//                          [--embedding-size 30k|10k|1k] [--max-p95-ms 250] [--reader-only]
//
// --search-size picks the search fixture: 30k (default; the P1-19 reference
// measurement, run locally) or 1k (CI: the 30k text fixture takes over 40
// minutes on Windows runners). --extraction-size does the same for the
// extraction benchmark (default: the search size); at 1k it also reads at
// the Slow duty cycle (simulated battery), which takes too long at 30k.
// --sweep adds the 30k throttle sweep (about 10 minutes) used to choose
// the cataloging rate. --search-only runs just the search benchmark,
// --extraction-only just the extraction benchmark; --evidence is the
// close-outs' run (`just ci-remote evidence`): Phase 1's extraction on the
// 30k text fixture, unthrottled, failing unless every file is read, and
// (Phase 2, Slice 2.8) the embedding benchmark on the 10k text fixture;
// --embedding-only runs just the embedding benchmark (Phase 2, P2-18): the
// embed job on the text fixture of --embedding-size (default: the search
// size) with hybrid search timed while it runs; --max-p95-ms then applies
// to that p95.
// --vectors-only runs just the vector index benchmark (Phase 2), at
// --vectors-size vectors of 1,024 dimensions (default 100,000, as CI; the
// reference machine also records 300,000 and 1,000,000);
// --reader-only runs just the page view benchmark (Phase 3, Slice 3.7):
// pages of the spike corpus's PDFs rendered at 1,200 px in the reader's
// worker, with and without a passage to outline, plus a cache hit.
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
const vectorsOnly = args.includes('--vectors-only');
const embeddingOnly = args.includes('--embedding-only');
const readerOnly = args.includes('--reader-only');
if ([searchOnly, extractionOnly, vectorsOnly, embeddingOnly, readerOnly].filter(Boolean).length > 1)
  throw new Error(
    '--search-only, --extraction-only, --vectors-only, --embedding-only and --reader-only exclude each other',
  );
const runInventory =
  !searchOnly && !extractionOnly && !vectorsOnly && !embeddingOnly && !readerOnly;
const runSearch = !extractionOnly && !vectorsOnly && !embeddingOnly && !readerOnly;
const runExtraction = !searchOnly && !vectorsOnly && !embeddingOnly && !readerOnly;
const runVectors = !searchOnly && !extractionOnly && !embeddingOnly && !readerOnly;
const runEmbedding = evidence || (!searchOnly && !extractionOnly && !vectorsOnly && !readerOnly);
const runReader = !evidence && !searchOnly && !extractionOnly && !vectorsOnly && !embeddingOnly;
const vectorsSize = Number(opt('vectors-size', '100000'));
if (!Number.isInteger(vectorsSize) || vectorsSize < 100)
  throw new Error('--vectors-size is a whole number of at least 100');
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
    : size === '10k'
      ? fixture('10k-text-copies', 10000, 0, 20, 700)
      : fixture('1k-text-copies', 1000, 0, 20, 100);
const searchLabel = `${searchSize}-text`;
const searchFixture = runSearch ? textFixture(searchSize) : null;
const extractionLabel = `${extractionSize}-text`;
const extractionFixture = runExtraction ? textFixture(extractionSize) : null;
// The close-out's embedding evidence is 10k files on every runner: 30k
// took 69 minutes on the reference machine (Slice 2.4), too long for them.
const embeddingSize = evidence ? '10k' : opt('embedding-size', searchSize);
if (!['30k', '10k', '1k'].includes(embeddingSize))
  throw new Error('--embedding-size is 30k, 10k or 1k');
const embeddingLabel = `${embeddingSize}-text`;
const embeddingFixture = runEmbedding ? textFixture(embeddingSize) : null;

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
    '--bench',
    'vectors',
    '--bench',
    'embedding',
    '--bench',
    'reader',
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

let vectorsResult = null;
if (runVectors) {
  console.error(`bench: ${vectorsSize} vectors`);
  const vectorsFile = path.join(out, `bench-vectors-${vectorsSize}.json`);
  execFileSync(
    'cargo',
    [
      'bench',
      '--manifest-path',
      path.join(repo, 'src-tauri', 'Cargo.toml'),
      '--locked',
      '--bench',
      'vectors',
      '--',
      '--size',
      String(vectorsSize),
      '--out',
      vectorsFile,
    ],
    { stdio: ['ignore', 'ignore', 'inherit'] },
  );
  vectorsResult = JSON.parse(fs.readFileSync(vectorsFile, 'utf8'));
}

let embeddingResult = null;
if (runEmbedding) {
  console.error(`bench: ${embeddingLabel} embedding`);
  const embeddingFile = path.join(out, `bench-${embeddingLabel}-embedding.json`);
  execFileSync(
    'cargo',
    [
      'bench',
      '--manifest-path',
      path.join(repo, 'src-tauri', 'Cargo.toml'),
      '--locked',
      '--bench',
      'embedding',
      '--',
      '--fixture',
      embeddingFixture.dir,
      '--label',
      embeddingLabel,
      '--expect',
      String(embeddingFixture.expect),
      '--out',
      embeddingFile,
      ...(maxP95 && embeddingOnly ? ['--max-p95-ms', String(maxP95)] : []),
    ],
    { stdio: ['ignore', 'ignore', 'inherit'] },
  );
  embeddingResult = JSON.parse(fs.readFileSync(embeddingFile, 'utf8'));
}

let readerResult = null;
if (runReader) {
  console.error('bench: page view');
  const readerFile = path.join(out, 'bench-reader.json');
  execFileSync(
    'cargo',
    [
      'bench',
      '--manifest-path',
      path.join(repo, 'src-tauri', 'Cargo.toml'),
      '--locked',
      '--bench',
      'reader',
      '--',
      '--out',
      readerFile,
    ],
    { stdio: ['ignore', 'ignore', 'inherit'] },
  );
  readerResult = JSON.parse(fs.readFileSync(readerFile, 'utf8'));
}

const md =
  (results.length ? summarize(results) : '') +
  (searchResult ? summarizeSearch(searchResult) : '') +
  (extractionResult ? summarizeExtraction(extractionResult) : '') +
  (vectorsResult ? summarizeVectors(vectorsResult) : '') +
  (embeddingResult ? summarizeEmbedding(embeddingResult) : '') +
  (readerResult ? summarizeReader(readerResult) : '');
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

function summarizeVectors(r) {
  const mib = (b) => (b / 1048576).toFixed(1);
  const s = (ms) => (ms / 1000).toFixed(2);
  const lat = (l) => `${l.p50Ms} / ${l.p95Ms} / ${l.maxMs}`;
  const pct = (x) => `${(x * 100).toFixed(1)}%`;
  const lines = [
    '',
    `## Vector index: ${r.size.toLocaleString('en-US')} × ${r.dims} (${r.machine.os} ${r.machine.arch}${r.machine.ci ? ', CI runner' : ''})`,
    '',
    `usearch HNSW, cosine, f16, connectivity ${r.index.connectivity}, expansion ${r.index.expansionAdd} / ${r.index.expansionSearch}; ${r.threads} threads; synthetic clustered vectors; catalog pages of ${r.pageSize / 1024} KiB.`,
    '',
    '| Build s | Save s | Load s | Open → first answer s | Rebuild from catalog s | File MiB | File ÷ raw f32 | Index memory MiB | Catalog bytes per vector |',
    '| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |',
    `| ${s(r.buildMs)} | ${s(r.saveMs)} | ${s(r.loadMs)} | ${s(r.openToFirstAnswerMs)} | ${s(r.rebuildFromCatalogMs)} | ${mib(r.fileBytes)} | ${r.fileToRawF32} | ${mib(r.indexMemoryBytes)} | ${r.catalogBytesPerVector} |`,
    '',
    `| Search (${r.queries} queries, one thread) | Share | Recall@10 | p50 / p95 / max ms | At 200: p50 / p95 / max ms |`,
    '| --- | ---: | ---: | ---: | ---: |',
    `| Unfiltered | 100% | ${r.unfiltered.recallAt10} | ${lat(r.unfiltered.latency)} | ${r.unfiltered.latencyAt200 ? lat(r.unfiltered.latencyAt200) : '–'} |`,
    ...r.filtered.map(
      (f) =>
        `| One folder | ${pct(f.share)} | ${f.recallAt10 ?? '–'} | ${lat(f.latency)} | ${f.latencyAt200 ? lat(f.latencyAt200) : '–'} |`,
    ),
    '',
    `Brute force: ${r.unfiltered.bruteForceMsPerQuery} ms per query (${r.recallQueries} recall queries).`,
  ];
  return `${lines.join('\n')}\n`;
}

function summarizeEmbedding(r) {
  const mib = (b) => (b / 1048576).toFixed(1);
  const h = r.hybridWhileEmbedding;
  const lines = [
    '',
    `## Embedding: ${r.label} (${r.machine.os} ${r.machine.arch}${r.machine.ci ? ', CI runner' : ''})`,
    '',
    `The embed job with the deterministic backend at ${r.dims} dimensions; hybrid search timed while it runs, with ${r.queryDelayMs} ms of query embedding.`,
    '',
    '| Documents | Chunks | Embed s | Chunks/s | Catalog growth per 1,000 docs MiB | Bytes per vector | Index file MiB | File ÷ raw | Save ms | Hybrid p50 / p95 / max ms | Meaning ready | Events/s |',
    '| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |',
    `| ${r.documents} | ${r.chunks} | ${r.embedSeconds} | ${r.chunksPerSecond} | ${mib(r.catalogGrowthPer1000Documents)} | ${r.catalogBytesPerVector} | ${mib(r.indexFileBytes)} | ${r.indexFileToRawF32} | ${r.saveMs} | ${h ? `${h.p50Ms} / ${h.p95Ms} / ${h.maxMs}` : '–'} | ${Math.round(r.meaningReadyShare * 100)}% | ${r.eventsPerSecond} |`,
  ];
  const rel = r.related;
  if (rel) {
    lines.push(
      '',
      `Related documents after the job, for ${rel.cold.count} documents spread over the library (Slice 2.5).`,
      '',
      '| Related p50 / p95 / max ms (cold) | p95 ms (cached) | Related per document |',
      '| ---: | ---: | ---: |',
      `| ${rel.cold.p50Ms} / ${rel.cold.p95Ms} / ${rel.cold.maxMs} | ${rel.warm.p95Ms} | ${rel.meanHits} |`,
    );
  }
  return `${lines.join('\n')}\n`;
}

function summarizeReader(r) {
  const mib = (b) => (b / 1048576).toFixed(1);
  const t = (x) => `${x.p50Ms} / ${x.p95Ms} / ${x.maxMs}`;
  const lines = [
    '',
    `## Page view (${r.machine.os} ${r.machine.arch}${r.machine.ci ? ', CI runner' : ''})`,
    '',
    `${r.pages} pages of ${r.files} corpus PDFs rendered at ${r.width} px in the reader's worker (Slice 3.7).`,
    '',
    '| First render ms | Render p50 / p95 / max ms | With the passage p50 / p95 / max ms | Outlined | Cache hit ms | Worker peak MiB |',
    '| ---: | ---: | ---: | ---: | ---: | ---: |',
    `| ${r.firstRenderMs} | ${t(r.render)} | ${t(r.renderWithFind)} | ${r.found} of ${r.renderWithFind.renders} | ${r.cacheHitMs} | ${mib(r.workerPeakRssBytes)} |`,
  ];
  return `${lines.join('\n')}\n`;
}
