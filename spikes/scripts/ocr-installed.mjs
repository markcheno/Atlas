#!/usr/bin/env node
// The OCR spike's installer measures (docs/spike-ocr.md, ADR 0020): what
// each engine adds to Atlas's real installers, and whether it works from
// where they install it. No app code changes: each variant's engine binary
// goes into the bundle as a stand-in (`bundle.externalBin`, renamed
// `ocr-standin`) with its files (`bundle.resources` under `ocr/`, ONNX
// Runtime beside PDFium), through `pnpm tauri build --config <patch>`.
//
//   node spikes/scripts/ocr-installed.mjs [--out <dir>] [--only v1,v2] [--runs 3]
//
// For each variant, and first for `baseline` (the stand-in that renders
// and reads nothing): build this OS's bundles, record their sizes, install
// or extract them as CI does, and run `ocr-standin --self-check` from the
// installed place `runs` times. The delta of a variant is its bundle's size
// less the baseline's. Writes ocr-installers-<platform>.json and .md.
//
// Needs the app's setup (`just setup`), `run-ocr.mjs`'s release build of
// spikes/ocr-bench, and spike-tools (onnxruntime, ppocr-models, tessdata).

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { readPaths } from './fetch-tools.mjs';

const spikes = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const repo = path.resolve(spikes, '..');
const bench = path.join(spikes, 'ocr-bench');
const tauriDir = path.join(repo, 'src-tauri');
const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1];
};
const out = path.resolve(opt('out', path.join(os.tmpdir(), 'atlas-spikes')));
const only = opt('only', null)?.split(',');
const runs = Number(opt('runs', '3'));
const tools = readPaths();
const isMac = process.platform === 'darwin';
const isWin = process.platform === 'win32';
const benchTarget = process.env.CARGO_TARGET_DIR
  ? path.resolve(bench, process.env.CARGO_TARGET_DIR)
  : path.join(bench, 'target');
const appTarget = path.join(tauriDir, 'target', 'release');
const exeSuffix = isWin ? '.exe' : '';
const bundles = isMac ? 'app,dmg' : isWin ? 'nsis' : 'deb,appimage';

const triple = spawnSync('rustc', ['-vV'], { encoding: 'utf8' })
  .stdout.split('\n')
  .find((l) => l.startsWith('host: '))
  .slice(6)
  .trim();

const ort = tools.onnxruntime?.library;
const VARIANTS = [
  { name: 'baseline', bin: 'baseline', files: {} },
  {
    name: 'tesseract-fast',
    bin: 'ocr-tesseract',
    files: {
      'tessdata/fast/eng.traineddata': path.join(tools.tessdata.dir, 'fast/eng.traineddata'),
    },
  },
  {
    name: 'tesseract-best',
    bin: 'ocr-tesseract',
    files: {
      'tessdata/best/eng.traineddata': path.join(tools.tessdata.dir, 'best/eng.traineddata'),
    },
  },
  ...[
    ['ppocr-v5-mobile', 'v5-mobile-det', 'v5-mobile-en-rec'],
    ['ppocr-v6-tiny', 'v6-tiny-det', 'v6-tiny-rec'],
    ['ppocr-v6-small', 'v6-small-det', 'v6-small-rec'],
  ].map(([name, det, rec]) => ({
    name,
    bin: 'ocr-ppocr',
    onnxRuntime: true,
    files: Object.fromEntries(
      [det, rec].flatMap((m) =>
        ['onnx', 'yml'].map((ext) => [
          `ppocr/${m}.${ext}`,
          path.join(tools['ppocr-models'].dir, `${m}.${ext}`),
        ]),
      ),
    ),
  })),
  ...(isMac ? [{ name: 'vision-accurate-raw', bin: 'ocr-vision', files: {} }] : []),
];

function run(cmd, cmdArgs, opts = {}) {
  const r = spawnSync(cmd, cmdArgs, {
    stdio: 'inherit',
    shell: isWin && cmd === 'pnpm',
    ...opts,
  });
  if (r.status !== 0) throw new Error(`${cmd} ${cmdArgs.join(' ')} failed (${r.status})`);
  return r;
}

/** Stages the variant's stand-in and files; returns the config patch's path. */
function stage(v) {
  const dir = path.join(bench, 'build', 'payload', v.name);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(path.join(dir, 'ocr'), { recursive: true });
  fs.copyFileSync(
    path.join(benchTarget, 'release', `${v.bin}${exeSuffix}`),
    path.join(dir, `ocr-standin-${triple}${exeSuffix}`),
  );
  fs.writeFileSync(path.join(dir, 'ocr', 'variant.txt'), `${v.name}\n`);
  for (const [to, from] of Object.entries(v.files)) {
    fs.mkdirSync(path.dirname(path.join(dir, 'ocr', to)), { recursive: true });
    fs.copyFileSync(from, path.join(dir, 'ocr', to));
  }
  const resources = { [path.join(dir, 'ocr') + path.sep]: 'ocr/' };
  const bundle = { externalBin: [path.join(dir, 'ocr-standin')], resources };
  if (v.onnxRuntime) {
    if (isMac) {
      // Arrays replace in a merge patch: PDFium is restated.
      bundle.macOS = { frameworks: ['vendor/pdfium/darwin-arm64/lib/libpdfium.dylib', ort] };
    } else {
      resources[ort] = path.basename(ort);
    }
  }
  const patch = path.join(dir, 'tauri.patch.json');
  fs.writeFileSync(patch, JSON.stringify({ bundle }, null, 2));
  return patch;
}

function sizeOf(p) {
  const st = fs.statSync(p);
  if (!st.isDirectory()) return st.size;
  return fs.readdirSync(p).reduce((a, e) => a + sizeOf(path.join(p, e)), 0);
}

function find(dir, re) {
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir).flatMap((e) => {
    const p = path.join(dir, e);
    if (re.test(e)) return [p];
    return fs.statSync(p).isDirectory() && !e.endsWith('.app') ? find(p, re) : [];
  });
}

/** The installed or extracted stand-ins, by layout. */
function installed(scratch) {
  const bundle = path.join(appTarget, 'bundle');
  const layouts = [];
  if (isMac) {
    const app = find(bundle, /\.app$/)[0];
    spawnSync('codesign', ['--verify', '--deep', '--strict', app], { stdio: 'inherit' });
    layouts.push({ layout: 'macOS .app', exe: path.join(app, 'Contents', 'MacOS', 'ocr-standin') });
  } else if (isWin) {
    const setup = find(bundle, /-setup\.exe$/)[0];
    const dir = path.join(scratch, 'install');
    run(setup, ['/S', `/D=${dir}`]);
    layouts.push({ layout: 'Windows installed', exe: path.join(dir, 'ocr-standin.exe') });
  } else {
    const deb = find(bundle, /\.deb$/)[0];
    const debDir = path.join(scratch, 'deb');
    run('dpkg-deb', ['-x', deb, debDir]);
    layouts.push({ layout: 'Linux .deb', exe: path.join(debDir, 'usr', 'bin', 'ocr-standin') });
    const image = find(bundle, /\.AppImage$/)[0];
    run(image, ['--appimage-extract'], { cwd: scratch, stdio: 'ignore' });
    const root = path.join(scratch, 'squashfs-root');
    layouts.push({
      layout: 'Linux AppImage',
      exe: path.join(root, 'usr', 'bin', 'ocr-standin'),
      env: { APPDIR: root },
    });
  }
  return layouts;
}

const report = {
  date: new Date().toISOString(),
  machine: { platform: process.platform, arch: process.arch, ci: Boolean(process.env.CI) },
  bundles,
  variants: {},
};
for (const v of VARIANTS) {
  if (only && !only.includes(v.name) && v.name !== 'baseline') continue;
  console.log(`installers: ${v.name}…`);
  const patch = stage(v);
  fs.rmSync(path.join(appTarget, 'bundle'), { recursive: true, force: true });
  run('pnpm', ['tauri', 'build', '--ci', '--bundles', bundles, '--config', patch], {
    cwd: repo,
  });
  const sizes = {};
  for (const f of find(path.join(appTarget, 'bundle'), /(\.dmg|-setup\.exe|\.deb|\.AppImage)$/))
    sizes[path.extname(f) === '.exe' ? 'nsis' : path.extname(f).slice(1)] = sizeOf(f);
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'atlas-ocr-installed-'));
  const checks = [];
  for (const l of installed(scratch)) {
    for (let i = 0; i < runs; i++) {
      const started = Date.now();
      const r = spawnSync(l.exe, ['--self-check'], {
        encoding: 'utf8',
        env: { PATH: process.env.PATH, ...(l.env ?? {}) },
        timeout: 180_000,
      });
      let line = null;
      try {
        line = JSON.parse(r.stdout.trim().split('\n').pop());
      } catch {
        // reported below
      }
      checks.push({
        layout: l.layout,
        run: i + 1,
        status: r.status,
        wallMs: Date.now() - started,
        ok: r.status === 0 && line?.ok === true,
        cer: line?.cer,
        error: line?.error ?? (r.status === 0 ? undefined : r.stderr?.slice(-500)),
      });
    }
  }
  fs.rmSync(scratch, { recursive: true, force: true });
  report.variants[v.name] = { sizes, checks };
}

const base = report.variants.baseline?.sizes ?? {};
for (const info of Object.values(report.variants))
  info.deltaBytes = Object.fromEntries(
    Object.entries(info.sizes).map(([k, size]) => [k, size - (base[k] ?? 0)]),
  );

const mib = (b) => (b / 1048576).toFixed(1);
const kinds = Object.keys(base);
const lines = [
  `## OCR in the installers: ${process.platform} ${process.arch}`,
  '',
  `Each installer carrying the variant's stand-in and files, less the one carrying the baseline stand-in. "Check" is \`ocr-standin --self-check\` run ${runs} times from each installed layout.`,
  '',
  `| Variant | ${kinds.map((k) => `${k} MiB (Δ)`).join(' | ')} | Check |`,
  `| --- | ${kinds.map(() => '---').join(' | ')} | --- |`,
];
for (const [name, info] of Object.entries(report.variants)) {
  const ok = info.checks.every((c) => c.ok);
  lines.push(
    `| ${name} | ${kinds.map((k) => `${mib(info.sizes[k] ?? 0)} (${mib(info.deltaBytes[k] ?? 0)})`).join(' | ')} | ${ok ? `passed ${info.checks.length} of ${info.checks.length}` : `FAILED ${info.checks.filter((c) => !c.ok).length} of ${info.checks.length}`} |`,
  );
}
const md = lines.join('\n') + '\n';
fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(
  path.join(out, `ocr-installers-${process.platform}.json`),
  JSON.stringify(report, null, 2),
);
fs.writeFileSync(path.join(out, `ocr-installers-${process.platform}.md`), md);
if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, md + '\n');
console.log(md);
