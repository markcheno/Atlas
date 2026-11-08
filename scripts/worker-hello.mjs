#!/usr/bin/env node
// Starts the extraction worker (`<exe> --extract-worker`) several times and
// checks that it answers `hello` with a passing PDFium self-test. Prints the
// time to hello: the first start (cold) and the median and max of the rest.
// Fails if any start does not say hello. CI runs it against the built
// bundles (phase1-spec §14, Slice 1.2).
//
//   node scripts/worker-hello.mjs <exe> [--runs 10] [--label "macOS release .app"] [--summary <file>]

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1];
};
const exe = args.find((a, i) => !a.startsWith('--') && !args[i - 1]?.startsWith('--'));
if (!exe) {
  console.error('usage: worker-hello.mjs <exe> [--runs N] [--label text] [--summary file]');
  process.exit(2);
}
const runs = Number(opt('runs', '10'));
const label = opt('label', path.basename(exe));
const summaryFile = opt('summary', process.env.GITHUB_STEP_SUMMARY);

function once() {
  return new Promise((resolve, reject) => {
    const start = process.hrtime.bigint();
    const child = spawn(exe, ['--extract-worker'], { stdio: ['pipe', 'pipe', 'pipe'] });
    let out = '';
    let err = '';
    let done = false;
    const timer = setTimeout(() => finish(new Error('no line within 30 s')), 30_000);
    const finish = (error, frame, ms) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      child.stdin.end();
      if (error) {
        child.kill();
        reject(new Error(`${error.message}\nstdout: ${out}\nstderr: ${err}`));
      } else resolve({ frame, ms });
    };
    child.on('error', (e) => finish(e));
    child.stderr.on('data', (b) => (err += b));
    child.stdout.on('data', (b) => {
      out += b;
      const nl = out.indexOf('\n');
      if (nl === -1) return;
      const ms = Number(process.hrtime.bigint() - start) / 1e6;
      try {
        finish(null, JSON.parse(out.slice(0, nl)), ms);
      } catch (e) {
        finish(e);
      }
    });
    child.on('exit', (code) => {
      if (!done) finish(new Error(`exited with ${code} before a line`));
    });
  });
}

const times = [];
let hello;
for (let n = 0; n < runs; n++) {
  const { frame, ms } = await once();
  if (frame.type !== 'hello' || frame.selfTest?.pages !== 2 || !(frame.selfTest?.chars > 0)) {
    console.error(
      `worker-hello: expected hello with a passing self-test, got ${JSON.stringify(frame)}`,
    );
    process.exit(1);
  }
  hello = frame;
  times.push(ms);
}
const warm = times.slice(1).sort((a, b) => a - b);
const round = (x) => Math.round(x * 10) / 10;
const result = {
  label,
  exe: path.resolve(exe),
  platform: `${process.platform}-${process.arch}`,
  runs,
  firstMs: round(times[0]),
  medianMs: round(warm[Math.floor(warm.length / 2)] ?? times[0]),
  maxMs: round(Math.max(...times)),
  pdfiumVersion: hello.pdfiumVersion,
  selfTest: hello.selfTest,
};
console.log(JSON.stringify(result, null, 2));
if (summaryFile) {
  fs.appendFileSync(
    summaryFile,
    `\n| Worker start (${label}) | first ${result.firstMs} ms | median ${result.medianMs} ms | max ${result.maxMs} ms | PDFium ${result.pdfiumVersion}, self-test ${hello.selfTest.pages} pages |\n`,
  );
}
