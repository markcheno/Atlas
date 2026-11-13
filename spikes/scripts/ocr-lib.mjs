// What the OCR spike's scripts share (docs/spike-ocr.md): the variants,
// where their files come from, the build, and running one engine.

import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { readPaths } from './fetch-tools.mjs';

const spikes = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const bench = path.join(spikes, 'ocr-bench');

export const PAGE_LIMIT_MS = 60_000;
export const targetDir = process.env.CARGO_TARGET_DIR
  ? path.resolve(bench, process.env.CARGO_TARGET_DIR)
  : path.join(bench, 'target');
export const exe = (name) =>
  path.join(targetDir, 'release', process.platform === 'win32' ? `${name}.exe` : name);

export const tools = readPaths();
export const env = {
  ATLAS_PDFIUM: tools.pdfium?.library ?? '',
  ATLAS_TESSDATA: tools.tessdata?.dir ?? '',
  ATLAS_PPOCR_MODELS: tools['ppocr-models']?.dir ?? '',
  ORT_DYLIB_PATH: tools.onnxruntime?.library ?? '',
};

/** Each variant: its binary and the files it would ship. */
export const VARIANTS = [
  { name: 'tesseract-fast', bin: 'ocr-tesseract', ships: ['tessdata:fast/eng.traineddata'] },
  { name: 'tesseract-best', bin: 'ocr-tesseract', ships: ['tessdata:best/eng.traineddata'] },
  {
    name: 'ppocr-v5-mobile',
    bin: 'ocr-ppocr',
    ships: ['onnxruntime', 'ppocr-models:v5-mobile-det.onnx', 'ppocr-models:v5-mobile-en-rec.onnx'],
  },
  {
    name: 'ppocr-v6-tiny',
    bin: 'ocr-ppocr',
    ships: ['onnxruntime', 'ppocr-models:v6-tiny-det.onnx', 'ppocr-models:v6-tiny-rec.onnx'],
  },
  {
    name: 'ppocr-v6-small',
    bin: 'ocr-ppocr',
    ships: ['onnxruntime', 'ppocr-models:v6-small-det.onnx', 'ppocr-models:v6-small-rec.onnx'],
  },
  { name: 'vision-accurate-lc', bin: 'ocr-vision', ships: [], macOnly: true },
  { name: 'vision-accurate-raw', bin: 'ocr-vision', ships: [], macOnly: true },
];

/** Bytes of what a variant would ship beside the app (uncompressed). */
export function shippedBytes(v) {
  let total = 0;
  for (const s of v.ships) {
    const [tool, file] = s.split(':');
    const p = file ? path.join(tools[tool].dir, file) : tools[tool].library;
    total += fs.statSync(p).size;
  }
  return total;
}

export function build() {
  console.log('ocr: building (release)…');
  const cmake = tools.cmake?.bin;
  const r = spawnSync('cargo', ['build', '--release', '--locked', '--workspace'], {
    cwd: bench,
    stdio: 'inherit',
    env: {
      ...process.env,
      ...(cmake
        ? { CMAKE: cmake, PATH: `${path.dirname(cmake)}${path.delimiter}${process.env.PATH}` }
        : {}),
    },
  });
  if (r.status !== 0) throw new Error('cargo build failed');
}

/** Runs one engine over `jobsFile`; resolves with timing and how it ended. */
export function runEngine(v, dpi, jobsFile, resultFile, pages) {
  return new Promise((resolve) => {
    const started = process.hrtime.bigint();
    const child = spawn(exe(v.bin), ['--dpi', String(dpi), '--variant', v.name, jobsFile], {
      env: { ...process.env, ...env, XBENCH_OUT: resultFile },
      stdio: ['ignore', 'ignore', 'pipe'],
    });
    let stderr = '';
    child.stderr.on('data', (d) => (stderr = (stderr + d).slice(-4000)));
    const limit = PAGE_LIMIT_MS * pages + 120_000;
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill('SIGKILL');
    }, limit);
    child.on('close', (code, signal) => {
      clearTimeout(timer);
      resolve({
        code,
        signal,
        timedOut,
        wallMs: Number(process.hrtime.bigint() - started) / 1e6,
        stderr,
      });
    });
  });
}
