#!/usr/bin/env node
// The OCR spike on the owner's own scans (docs/spike-ocr.md): no reference
// text, so it times each engine and shows each page beside every engine's
// text, for the owner to read. Not part of ADR 0020's rule.
//
//   node spikes/scripts/ocr-mine.mjs <folder of PDFs> [--out <dir>]
//                                    [--pages-per-file 3] [--no-build]
//
// - At most 10 PDFs (evenly spaced by name when there are more), and up
//   to `pages-per-file` image-only pages of each (fewer than 20 visible
//   characters, Atlas's rule), evenly spaced.
// - The PDFs are only read; each one's SHA-256 is taken before and after,
//   and the run says whether they are unchanged.
// - The page (atlas-ocr-mine.html) holds the pages' images and text, so it
//   is written outside the input folder, and inside the repository only
//   under spikes/ocr-bench/target/ (ignored by git). The default is
//   spikes/ocr-bench/target/mine/.
// - The terminal shows numbers only: documents by position, never names.

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { VARIANTS, bench, build, env, exe, runEngine } from './ocr-lib.mjs';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1];
};
const flag = (name) => args.includes(`--${name}`);
const folder = args.find((a, i) => !a.startsWith('--') && !args[i - 1]?.startsWith('--'));
const MAX_FILES = 10;
const perFile = Number(opt('pages-per-file', '3'));
const IMAGE_DPI = 110;

/** `p` with symlinks resolved as far as the path exists. */
function resolved(p) {
  let at = path.resolve(p);
  const rest = [];
  for (;;) {
    try {
      return path.join(fs.realpathSync(at), ...rest.reverse());
    } catch {
      const parent = path.dirname(at);
      if (parent === at) return path.resolve(p);
      rest.push(path.basename(at));
      at = parent;
    }
  }
}

const inside = (child, parent) => {
  const rel = path.relative(parent, child);
  return rel === '' || (!rel.startsWith('..') && !path.isAbsolute(rel));
};

/** Refuses an output folder where the page shouldn't go. */
export function checkOutput(outArg, input) {
  if (outArg.split(/[\\/]/).includes('..'))
    throw new Error('the output folder may not contain `..`');
  const out = resolved(outArg);
  if (inside(out, resolved(input)))
    throw new Error('the output folder is inside the folder of PDFs: choose one outside it');
  const allowed = resolved(path.join(bench, 'target'));
  if (inside(out, resolved(repo)) && !inside(out, allowed))
    throw new Error(
      'the output folder is inside the repository: the page holds your pages, so choose spikes/ocr-bench/target or a folder outside the repository',
    );
  return out;
}

const sha256 = (file) => createHash('sha256').update(fs.readFileSync(file)).digest('hex');

function spread(items, n) {
  if (items.length <= n) return items;
  if (n === 1) return [items[Math.floor(items.length / 2)]];
  return Array.from({ length: n }, (_, i) => items[Math.round((i * (items.length - 1)) / (n - 1))]);
}

const escape = (s) =>
  s.replace(
    /[&<>"']/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c],
  );

async function main() {
  if (!folder) {
    console.error('usage: ocr-mine.mjs <folder of PDFs> [--out <dir>] [--pages-per-file 3]');
    process.exit(2);
  }
  const out = checkOutput(opt('out', path.join(bench, 'target', 'mine')), folder);
  if (!flag('no-build')) build();

  const all = fs
    .readdirSync(folder)
    .filter((f) => f.toLowerCase().endsWith('.pdf'))
    .sort()
    .map((f) => path.join(path.resolve(folder), f));
  const files = spread(all, MAX_FILES);
  if (!files.length) throw new Error('no PDFs in that folder');
  const before = Object.fromEntries(files.map((f) => [f, sha256(f)]));

  // Image-only pages, by Atlas's rule.
  const listed = spawnSync(exe('baseline'), ['--pages', ...files], {
    encoding: 'utf8',
    env: { ...process.env, ...env },
    maxBuffer: 64 << 20,
  });
  const jobs = [];
  listed.stdout
    .split('\n')
    .filter(Boolean)
    .forEach((line, i) => {
      const r = JSON.parse(line);
      if (r.error) return;
      const scanned = r.visibleChars.flatMap((n, p) => (n < 20 ? [p + 1] : []));
      for (const page of spread(scanned, perFile))
        jobs.push({ id: `doc${i + 1}-p${page}`, pdf: r.pdf, page, doc: i + 1 });
    });
  if (!jobs.length) throw new Error('none of these PDFs has an image-only page');

  fs.mkdirSync(out, { recursive: true });
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'atlas-ocr-mine-'));
  const jobsFile = path.join(work, 'jobs.jsonl');
  fs.writeFileSync(jobsFile, jobs.map((j) => JSON.stringify(j)).join('\n') + '\n');
  const results = path.join(work, 'results');
  fs.mkdirSync(results);

  // The page images, small enough to sit in one page.
  const images = path.join(work, 'images');
  spawnSync(
    exe('baseline'),
    ['--dpi', String(IMAGE_DPI), '--variant', 'none', '--dump', images, jobsFile],
    { env: { ...process.env, ...env, XBENCH_OUT: path.join(work, 'images.jsonl') } },
  );
  spawnSync(exe('baseline'), ['--dpi', '300', '--variant', 'text-layer', jobsFile], {
    env: { ...process.env, ...env, XBENCH_OUT: path.join(work, 'layer.jsonl') },
  });

  const numbers = { documents: files.length, pages: jobs.length, engines: {} };
  for (const v of VARIANTS) {
    if (v.macOnly && process.platform !== 'darwin') continue;
    console.log(`mine: ${v.name}, ${jobs.length} pages…`);
    const file = path.join(results, `${v.name}.jsonl`);
    const r = await runEngine(v, 300, jobsFile, file, jobs.length);
    const lines = fs.existsSync(file)
      ? fs
          .readFileSync(file, 'utf8')
          .split('\n')
          .filter(Boolean)
          .map((l) => JSON.parse(l))
      : [];
    const pages = lines.filter((l) => l.id);
    const secs = pages
      .slice(1)
      .filter((l) => l.ocrMs != null)
      .map((l) => l.ocrMs / 1e3)
      .sort((a, b) => a - b);
    const q = (p) => (secs.length ? secs[Math.round((secs.length - 1) * p)] : null);
    const done = lines.find((l) => l.done);
    numbers.engines[v.name] = {
      initMs: lines.find((l) => l.ready)?.initMs ?? null,
      firstPageS: pages[0]?.ocrMs != null ? pages[0].ocrMs / 1e3 : null,
      medianS: q(0.5),
      p90S: q(0.9),
      peakRssMiB: done ? done.peakRssBytes / 1048576 : null,
      meanChars: pages.length
        ? pages.reduce((a, l) => a + (l.text?.length ?? 0), 0) / pages.length
        : 0,
      failed: jobs.length - pages.filter((l) => l.outcome === 'ok').length,
      timedOut: r.timedOut,
    };
  }
  const pair = spawnSync(exe('score'), ['--pairwise', results], { encoding: 'utf8' });
  numbers.disagreement = pair.status === 0 ? JSON.parse(pair.stdout).pairs : [];
  const after = Object.fromEntries(files.map((f) => [f, sha256(f)]));
  numbers.originalsUnchanged = files.every((f) => before[f] === after[f]);

  // The page.
  const read = (file) =>
    fs.existsSync(file)
      ? Object.fromEntries(
          fs
            .readFileSync(file, 'utf8')
            .split('\n')
            .filter(Boolean)
            .map((l) => JSON.parse(l))
            .filter((l) => l.id)
            .map((l) => [l.id, l]),
        )
      : {};
  const engines = Object.keys(numbers.engines).map((n) => [
    n,
    read(path.join(results, `${n}.jsonl`)),
  ]);
  const layer = read(path.join(work, 'layer.jsonl'));
  const sections = jobs.map((j) => {
    const png = path.join(images, `${j.id}.png`);
    const img = fs.existsSync(png)
      ? `<img alt="Page ${j.page} of document ${j.doc}" src="data:image/png;base64,${fs.readFileSync(png).toString('base64')}">`
      : '<p>No image.</p>';
    const columns = engines
      .map(([name, rows]) => {
        const r = rows[j.id];
        const head = `${escape(name)} · ${r?.ocrMs != null ? (r.ocrMs / 1e3).toFixed(2) + ' s' : (r?.outcome ?? 'no result')}`;
        return `<div class="col"><h3>${head}</h3><pre>${escape(r?.text ?? r?.error ?? '')}</pre></div>`;
      })
      .join('');
    const own = layer[j.id]?.text?.trim()
      ? `<div class="col"><h3>The PDF's own text layer</h3><pre>${escape(layer[j.id].text)}</pre></div>`
      : '';
    return `<section><h2>${escape(path.basename(j.pdf))}, page ${j.page}</h2><div class="row"><div class="img">${img}</div>${columns}${own}</div></section>`;
  });
  const html = `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>OCR on your scans</title>
<style>body{font:14px/1.4 system-ui,sans-serif;margin:16px;color:#111;background:#fcfcfb}
section{border-top:1px solid #ddd;padding:12px 0}.row{display:flex;gap:12px;overflow-x:auto}
.img img{width:420px;border:1px solid #ccc}.col{min-width:300px;max-width:380px}
pre{white-space:pre-wrap;font:12px/1.35 ui-monospace,Menlo,Consolas,monospace;background:#f3f2ee;padding:8px;border-radius:6px}
h2{font-size:15px}h3{font-size:13px;margin:0 0 4px}
@media (prefers-color-scheme:dark){body{background:#1a1a19;color:#f0efec}pre{background:#242423}}</style></head>
<body><h1>OCR on your scans</h1><p>Each page as the engines saw it (shown smaller), and what each read. Nothing here was sent anywhere.</p>
${sections.join('\n')}</body></html>`;
  const page = path.join(out, 'atlas-ocr-mine.html');
  fs.writeFileSync(page, html);
  fs.rmSync(work, { recursive: true, force: true });

  console.log(`ocr-mine-numbers ${JSON.stringify(numbers)}`);
  console.log(
    `\n${numbers.documents} documents, ${numbers.pages} pages. Originals unchanged: ${numbers.originalsUnchanged ? 'yes' : 'NO'}.`,
  );
  for (const [name, e] of Object.entries(numbers.engines))
    console.log(
      `${name}: median ${e.medianS?.toFixed(2) ?? '–'} s a page (90% under ${e.p90S?.toFixed(2) ?? '–'} s), first ${e.firstPageS?.toFixed(2) ?? '–'} s, ${e.peakRssMiB?.toFixed(0) ?? '–'} MiB at most${e.failed ? `, ${e.failed} failed` : ''}`,
    );
  console.log(`\nOpen this page in a browser:\n${page}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    await main();
  } catch (e) {
    console.error(`spike-ocr-mine: ${e.message}`);
    process.exit(1);
  }
}
