#!/usr/bin/env node
// The OCR spike's driver (docs/spike-ocr.md, ADR 0020).
//
//   node spikes/scripts/run-ocr.mjs [--out <dir>] [--no-build] [--build-only]
//                                   [--only v1,v2] [--dpi 300,200]
//                                   [--corpus <build/corpus>]
//
// Builds spikes/ocr-bench (release), then for every variant and DPI runs
// the engine over the corpus in one process (pages in order, the first
// timed apart), with a time limit of 60 s a page; a process that passes
// it is killed and its unread pages count as failed. Then the time from a
// new process to its first page (median of 5), the scorer, and
// ocr-<platform>.json and .md in <out>.
//
// Needs `just spike-tools pdfium onnxruntime cmake ppocr-models tessdata`
// and the built corpus (`just spike-ocr-corpus`).

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { VARIANTS, build, exe, runEngine, shippedBytes } from './ocr-lib.mjs';

const spikes = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const bench = path.join(spikes, 'ocr-bench');
const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1];
};
const flag = (name) => args.includes(`--${name}`);
const out = path.resolve(opt('out', path.join(os.tmpdir(), 'atlas-spikes')));
const corpus = path.resolve(opt('corpus', path.join(bench, 'build', 'corpus')));
const only = opt('only', null)?.split(',');
const dpis = opt('dpi', '300,200').split(',').map(Number);
async function main() {
  if (!flag('no-build')) build();
  if (flag('build-only')) return;
  // The corpus's jobs name their PDFs relative to the corpus folder, so a
  // corpus built on one runner reads on another.
  const jobs = fs
    .readFileSync(path.join(corpus, 'jobs.jsonl'), 'utf8')
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      const job = JSON.parse(line);
      return JSON.stringify({ ...job, pdf: path.resolve(corpus, job.pdf) });
    });
  fs.mkdirSync(out, { recursive: true });
  const results = path.join(out, `ocr-results-${process.platform}`);
  fs.rmSync(results, { recursive: true, force: true });
  fs.mkdirSync(results, { recursive: true });
  const jobsFile = path.join(results, 'jobs.jsonl');
  fs.writeFileSync(jobsFile, jobs.join('\n') + '\n');
  const firstJob = path.join(results, 'first-job.jsonl');
  fs.writeFileSync(firstJob, jobs[0] + '\n');

  const report = {
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
    pages: jobs.length,
    variants: {},
  };

  for (const v of VARIANTS) {
    if (only && !only.includes(v.name)) continue;
    if (v.macOnly && process.platform !== 'darwin') continue;
    if (!fs.existsSync(exe(v.bin))) {
      console.log(`ocr: ${v.bin} not built; ${v.name} skipped`);
      continue;
    }
    const info = {
      binary: v.bin,
      binaryBytes: fs.statSync(exe(v.bin)).size,
      shippedBytes: shippedBytes(v),
      runs: {},
    };
    for (const dpi of dpis) {
      console.log(`ocr: ${v.name} at ${dpi} DPI, ${jobs.length} pages…`);
      const r = await runEngine(
        v,
        dpi,
        jobsFile,
        path.join(results, `${v.name}@${dpi}.jsonl`),
        jobs.length,
      );
      info.runs[dpi] = {
        wallMs: r.wallMs,
        code: r.code,
        signal: r.signal,
        timedOut: r.timedOut,
        stderr: r.code === 0 ? undefined : r.stderr,
      };
    }
    // A new process to its first page, at 300 DPI (median of 5).
    const cold = [];
    for (let i = 0; i < 5; i++) {
      const r = await runEngine(v, 300, firstJob, path.join(results, 'cold.tmp'), 1);
      cold.push(r.wallMs);
    }
    fs.rmSync(path.join(results, 'cold.tmp'), { force: true });
    info.firstPageInNewProcessMs = {
      runs: cold,
      first: cold[0],
      median: [...cold].sort((a, b) => a - b)[2],
    };
    report.variants[v.name] = info;
  }

  const scored = spawnSync(exe('score'), ['--corpus', corpus, '--results', results], {
    encoding: 'utf8',
    maxBuffer: 256 * 1024 * 1024,
  });
  if (scored.status !== 0) throw new Error(`score failed: ${scored.stderr}`);
  report.scores = JSON.parse(scored.stdout);

  const stem = `ocr-${process.platform}`;
  fs.writeFileSync(path.join(out, `${stem}.json`), JSON.stringify(report, null, 2));
  const md = markdown(report);
  fs.writeFileSync(path.join(out, `${stem}.md`), md);
  if (process.env.GITHUB_STEP_SUMMARY)
    fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, md + '\n');
  console.log(md);
}

const pct = (x) => (x == null ? '–' : `${(100 * x).toFixed(1)}%`);
const sec = (x) => (x == null ? '–' : `${x.toFixed(2)} s`);
const mib = (x) => (x == null ? '–' : `${(x / 1048576).toFixed(1)} MiB`);

function markdown(report) {
  const m = report.machine;
  const lines = [
    `## OCR spike: ${m.platform} ${m.arch} (${m.cpu}, ${m.cores} cores${m.ci ? ', CI' : ''})`,
    '',
    `${report.pages} pages. WER and CER are the mean of the pages' rates (ADR 0020).`,
    '',
    '| Variant | DPI | WER | CER | Book | Two-column | Degraded | Poor layer | Order | s/page (median, p90) | First page | Peak memory | Shipped files | Failed | Over 60 s |',
    '| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |',
  ];
  for (const [name, info] of Object.entries(report.variants)) {
    for (const dpi of Object.keys(info.runs)) {
      const s = report.scores.runs[`${name}@${dpi}`];
      if (!s) continue;
      const cat = (c) => pct(s.byCategory[c]?.wer);
      lines.push(
        `| ${name} | ${dpi} | ${pct(s.wer)} | ${pct(s.cer)} | ${cat('book')} | ${cat('two-column')} | ${cat('degraded')} | ${cat('poor-layer')} | ${s.readingOrder.toFixed(3)} | ${sec(s.ocrSMedian)}, ${sec(s.ocrSP90)} | ${sec(s.firstPageS)} | ${s.peakRssMib?.toFixed(0) ?? '–'} MiB | ${mib(info.shippedBytes)} | ${s.failed} | ${s.over60S} |`,
      );
    }
  }
  const layer = report.scores.textLayer;
  if (layer?.pages)
    lines.push(
      '',
      `The poor-layer pages' own text layer: WER ${pct(layer.wer)}, CER ${pct(layer.cer)} over ${layer.pages} pages.`,
    );
  lines.push(
    '',
    '| New process to first page (300 DPI) | median | first run |',
    '| --- | --- | --- |',
  );
  for (const [name, info] of Object.entries(report.variants))
    lines.push(
      `| ${name} | ${(info.firstPageInNewProcessMs.median / 1e3).toFixed(2)} s | ${(info.firstPageInNewProcessMs.first / 1e3).toFixed(2)} s |`,
    );
  return lines.join('\n') + '\n';
}

await main();
