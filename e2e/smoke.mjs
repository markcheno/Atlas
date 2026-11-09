#!/usr/bin/env node
// Automated smoke test of the built app (phase0-spec §14), for Windows and
// Linux, where tauri-driver can drive the webview. macOS has no WebDriver
// for WKWebView; it uses the manual checklist in docs/platform-validation.md.
//
//   node e2e/smoke.mjs [--app <debug binary>] [--out <dir>] [--native-driver <path>]
//
// Needs a *debug* build (`pnpm tauri build --debug`): the run uses debug-only
// hooks to answer the native folder pickers and record open requests
// (src-tauri/src/infrastructure/e2e.rs). Everything lives in a scratch
// folder; no real configuration or library is touched.

import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { WebDriver, button, lit, sleep } from './webdriver.mjs';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1];
};

if (process.platform === 'darwin') {
  console.error('smoke: macOS has no WebDriver for its webview; use the manual checklist.');
  process.exit(2);
}

const exe = process.platform === 'win32' ? 'atlas.exe' : 'atlas';
const app = path.resolve(opt('app', path.join(repo, 'src-tauri', 'target', 'debug', exe)));
if (!fs.existsSync(app)) {
  console.error(`smoke: ${app} not found; run \`pnpm tauri build --debug\` first`);
  process.exit(2);
}
// Long, canonical form: Windows temp paths can be 8.3 short names.
const scratch = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), 'atlas-e2e-')));
const out = path.resolve(opt('out', path.join(scratch, 'out')));
fs.mkdirSync(out, { recursive: true });

// A fresh 1,000-file fixture with body text (about 4 KiB a file), so the
// run can read text and search passages.
const fixture = path.join(scratch, 'Books');
const summary = JSON.parse(
  execFileSync(process.execPath, [
    path.join(repo, 'scripts', 'generate-fixtures.mjs'),
    fixture,
    '--files',
    '1000',
    '--text-kb',
    '4',
  ])
    .toString()
    .split('\nTo delete later')[0],
);
const expected = summary.catalogableFiles;
const dataRoot = path.join(scratch, 'AtlasData');
fs.mkdirSync(dataRoot);
const openerLog = path.join(scratch, 'opened.log');

const env = {
  ...process.env,
  ATLAS_CONFIG_DIR: path.join(scratch, 'config'),
  ATLAS_E2E_PICK_DATA_ROOT: dataRoot,
  ATLAS_E2E_PICK_SOURCES: fixture,
  ATLAS_E2E_OPENER_LOG: openerLog,
  // Catalog slowly enough (debug builds only) that "pause and resume
  // cataloging" always finds the scan still running, however fast the runner.
  ATLAS_INVENTORY_RATE: '150',
};

const report = {
  os: process.platform,
  arch: process.arch,
  osRelease: os.release(),
  app,
  expectedFiles: expected,
  steps: [],
  timingsMs: {},
};
const t0 = Date.now();
let shot = 0;

// Preflight: launch the app directly (no WebDriver) with its own scratch
// configuration, and check it is still running after 15 s. This tells an
// app that crashes on start apart from a driver that can't attach.
report.preflight = await new Promise((resolve) => {
  const started = Date.now();
  const child = spawn(app, [], {
    env: { ...env, ATLAS_CONFIG_DIR: path.join(scratch, 'preflight-config') },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  const keep = (b) => (output = (output + b.toString()).slice(-4000));
  child.stdout.on('data', keep);
  child.stderr.on('data', keep);
  const timer = setTimeout(() => {
    child.kill();
    resolve({ alive: true, ms: Date.now() - started, output });
  }, 15_000);
  child.on('error', (err) => {
    clearTimeout(timer);
    resolve({ alive: false, error: err.message, output });
  });
  child.on('exit', (code, signal) => {
    clearTimeout(timer);
    resolve({ alive: false, code, signal, ms: Date.now() - started, output });
  });
});
console.log(`smoke: preflight ${JSON.stringify(report.preflight)}`);
await sleep(1_000);

const driverArgs = [];
const native = opt('native-driver', null);
if (native) driverArgs.push('--native-driver', native);
const driver = spawn(opt('driver', 'tauri-driver'), driverArgs, { env, stdio: 'inherit' });
driver.on('error', (err) => fail(`could not start tauri-driver: ${err.message}`));

const wd = new WebDriver();

async function step(name, f) {
  const started = Date.now();
  process.stdout.write(`smoke: ${name}… `);
  try {
    const detail = await f();
    const ms = Date.now() - started;
    report.steps.push({ name, ok: true, ms, detail: detail ?? null });
    console.log(`ok (${ms} ms)`);
    await capture(name);
  } catch (err) {
    report.steps.push({ name, ok: false, error: err.message });
    console.log('FAILED');
    await capture(`${name} (failed)`);
    throw err;
  }
}

async function capture(name) {
  try {
    const png = await wd.screenshot();
    shot += 1;
    const file = `${String(shot).padStart(2, '0')}-${name.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}.png`;
    fs.writeFileSync(path.join(out, file), Buffer.from(png, 'base64'));
  } catch {
    // Screenshots are evidence, not a requirement.
  }
}

const heading = (text) => `//h1[normalize-space(.)=${lit(text)}]`;
const status = () => wd.waitFor("//*[@data-testid='inventory-status']");
const statusText = async () => wd.text(await status());
const formatted = expected.toLocaleString('en-US');
const readingStatus = async () =>
  wd.text(await wd.waitFor("//*[@data-testid='extraction-status']"));
// The cataloging line's controls; the reading line's have their own labels.
const catalogButton = (label) => `//button[normalize-space(.)=${lit(label)} and not(@aria-label)]`;

/** Command lines of this app's text-reading worker processes. */
function workerProcesses() {
  try {
    if (process.platform === 'win32') {
      const outText = execFileSync(
        'powershell.exe',
        [
          '-NoProfile',
          '-Command',
          'Get-CimInstance Win32_Process -Filter "Name = \'atlas.exe\'" | ForEach-Object { $_.CommandLine }',
        ],
        { encoding: 'utf8' },
      );
      return outText.split(/\r?\n/).filter((l) => l.includes('--extract-worker'));
    }
    // Wide output: command lines are never cut short.
    const outText = execFileSync('ps', ['-eww', '-o', 'args'], { encoding: 'utf8' });
    const name = path.basename(app);
    return outText.split('\n').filter((l) => l.includes('--extract-worker') && l.includes(name));
  } catch {
    return [];
  }
}

async function launch() {
  const started = Date.now();
  await wd.start({ 'tauri:options': { application: app } });
  return started;
}

async function run() {
  await waitForPort(4444, 30_000);

  await step('launch shows the welcome screen', async () => {
    const started = await launch();
    await wd.waitFor(button('Get Started'), { timeout: 60_000 });
    report.timingsMs.launchToWelcome = Date.now() - started;
  });

  await step('choose the data folder', async () => {
    await wd.click(await wd.waitFor(button('Get Started')));
    await wd.waitFor(heading('Choose where Atlas keeps its library'));
    await wd.click(await wd.waitFor(button('Choose another location…'), { enabled: true }));
    // Wait for the picked folder to replace the suggested one.
    await wd.waitFor(
      `//*[@data-testid='candidate-path'][contains(., ${lit(path.basename(dataRoot))})]`,
    );
    await wd.click(await wd.waitFor(button('Use this location'), { enabled: true }));
    await wd.waitFor(heading('Add the folders where you keep your books and papers'));
    if (!fs.existsSync(path.join(dataRoot, 'atlas.json'))) throw new Error('no atlas.json');
  });

  await step('add a library folder and see rows', async () => {
    const picked = Date.now();
    await wd.click(await wd.waitFor(button('Add folders…'), { enabled: true }));
    await wd.click(await wd.waitFor(button('Continue'), { enabled: true }));
    await wd.waitFor(heading('Checking this computer'));
    await wd.click(await wd.waitFor(button('Continue'), { enabled: true }));
    await wd.waitFor(heading('What are you curious about?'));
    await wd.click(await wd.waitFor(button('Library')));
    await wd.waitFor(heading('Library'));
    await wd.waitFor("//button[starts-with(@aria-label, 'Open ')]", { timeout: 60_000 });
    report.timingsMs.selectionToFirstRows = Date.now() - picked;
  });

  await step('pause and resume cataloging', async () => {
    await wd.click(await wd.waitFor(catalogButton('Pause')));
    await wd.until('paused status', async () => (await statusText()).includes('Paused by you'));
    const paused = await statusText();
    await sleep(500);
    await capture('paused');
    await wd.click(await wd.waitFor(catalogButton('Resume')));
    await wd.until('resumed status', async () => !(await statusText()).includes('Paused'));
    return { paused };
  });

  await step(`cataloging finishes with ${formatted} files`, async () => {
    const line = await wd.until(
      'up to date',
      async () => {
        const t = await statusText();
        return t.startsWith('Up to date') && t;
      },
      300_000,
    );
    if (!line.includes(formatted)) throw new Error(`status says "${line}"`);
    await wd.waitFor(`//*[@data-testid='range'][contains(., ${lit(`of ${formatted}`)})]`);
    return { status: line };
  });

  await step('search ignores accents and case', async () => {
    const box = await wd.waitFor("//input[@placeholder='Search file names and folders']");
    await wd.type(box, 'CAFE creme');
    await wd.waitFor("//button[starts-with(@aria-label, 'Open Café crème')]");
    // Paths are shown in their ordinary form, never Windows' verbatim \\?\ form.
    const page = await wd.execute('return document.body.innerText');
    if (page.includes('\\\\?\\')) throw new Error('a path is shown in verbatim (\\\\?\\) form');
  });

  await step('open asks the OS to open the original', async () => {
    await wd.click(await wd.waitFor("//button[starts-with(@aria-label, 'Open Café crème')]"));
    const line = await wd.until('open request', async () => {
      const text = fs.existsSync(openerLog) ? fs.readFileSync(openerLog, 'utf8') : '';
      return text.split('\n').find((l) => l.startsWith('open ') && l.includes('Café crème'));
    });
    const opened = line.slice('open '.length);
    if (opened.startsWith('\\\\?\\')) throw new Error(`verbatim path sent to the OS: ${opened}`);
    if (!fs.existsSync(opened)) throw new Error(`opened a path that doesn't exist: ${opened}`);
    const rel = path.relative(fs.realpathSync.native(fixture), fs.realpathSync.native(opened));
    if (rel.startsWith('..') || path.isAbsolute(rel)) throw new Error('opened outside the folder');
    return { opened: rel };
  });

  await step('quit while reading leaves no worker process', async () => {
    // Catch the worker at work if reading hasn't finished yet.
    const seen = await wd
      .until('a text-reading worker', async () => workerProcesses().length > 0, 20_000)
      .catch(() => false);
    const readingBeforeQuit = await readingStatus();
    await wd.quit();
    await wd
      .until('no worker after quit', async () => workerProcesses().length === 0, 15_000)
      .catch(() => {
        throw new Error(`worker still running after quit: ${workerProcesses().join(' | ')}`);
      });
    await launch();
    await wd.waitFor(heading('What are you curious about?'), { timeout: 60_000 });
    return { workerSeenBeforeQuit: seen, readingBeforeQuit };
  });

  await step('search finds a passage', async () => {
    const line = await wd.until(
      'text up to date',
      async () => {
        const t = await readingStatus();
        return t.startsWith('Text up to date') && t;
      },
      300_000,
    );
    report.timingsMs.textUpToDate = Date.now() - t0;
    const box = await wd.waitFor("//input[@id='home-search']");
    // \uE007 is WebDriver's Enter key.
    await wd.type(box, '"Astronomy marker 0"\uE007');
    await wd.waitFor(heading('Search'));
    const hit = await wd.waitFor(
      `//li[@data-testid='search-hit'][.//*[normalize-space(.)=${lit('Café crème book 000000.epub')}]]`,
      { timeout: 30_000 },
    );
    const marked = await wd.all(
      `//li[@data-testid='search-hit'][.//*[normalize-space(.)=${lit('Café crème book 000000.epub')}]]//mark`,
    );
    if (marked.length === 0) throw new Error('the result has no highlighted words');
    const text = await wd.text(hit);
    if (!/Astronomy marker 0/i.test(text)) throw new Error(`snippet lacks the phrase: ${text}`);
    return { status: line, result: text.split('\n').slice(0, 4).join(' / ') };
  });

  await step('duplicates list the six Introduction copies', async () => {
    await wd.click(await wd.waitFor(button('Library')));
    await wd.waitFor(heading('Library'));
    // The switch's radio inputs are visually hidden; their labels are clicked.
    await wd.click(await wd.waitFor("//label[normalize-space(.)='Duplicates']"));
    const group = `//li[@data-testid='duplicate-group'][.//h3[normalize-space(.)='Introduction']]`;
    await wd.waitFor(group, { timeout: 30_000 });
    const copies = await wd.all(`${group}//ul/li`);
    if (copies.length !== 6) throw new Error(`${copies.length} copies listed, expected 6`);
    const summary = await wd.text(await wd.waitFor("//*[@data-testid='duplicates-summary']"));
    return { summary, copies: copies.length };
  });

  await step('settings show storage', async () => {
    await wd.click(await wd.waitFor(button('Settings')));
    const estimate = await wd.until('storage measured', async () => {
      const els = await wd.all("//*[@data-testid='indexing-estimate']");
      return els.length > 0 && (await wd.text(els[0]));
    });
    if (!estimate.includes('every file')) throw new Error(`estimate says "${estimate}"`);
    const usage = await wd.text(await wd.waitFor("//*[@data-testid='storage-usage']"));
    if (!/Text Atlas has read\s+[\d.]+ (KB|MB|GB)/.test(usage)) {
      throw new Error(`no text size in "${usage}"`);
    }
    await wd.click(await wd.waitFor(button('Clean caches'), { enabled: true }));
    const cleaned = await wd.until('clean result', async () => {
      const t = await wd.text(await wd.waitFor("//*[@data-testid='clean-result']"));
      return t.trim() && t;
    });
    return { estimate, cleaned };
  });

  await step('settings show AI Models', async () => {
    await wd.click(await wd.waitFor(button('Settings')));
    await wd.waitFor("//h2[normalize-space(.)='AI Models']");
    await wd.waitFor(button('Add a service…'));
  });

  await step('quit and relaunch reopens the same library', async () => {
    await wd.quit();
    await sleep(1_000);
    const started = await launch();
    await wd.until(
      'reopened library',
      async () => (await statusText()).includes(`Up to date · ${formatted}`),
      60_000,
    );
    report.timingsMs.relaunchToLibrary = Date.now() - started;
    if ((await wd.all(button('Get Started'))).length > 0) throw new Error('onboarding shown again');
  });
}

function waitForPort(port, timeout) {
  const deadline = Date.now() + timeout;
  return new Promise((resolve, reject) => {
    const attempt = () => {
      const s = net.connect(port, '127.0.0.1');
      s.once('connect', () => {
        s.end();
        resolve();
      });
      s.once('error', () => {
        s.destroy();
        if (Date.now() > deadline) reject(new Error('tauri-driver did not start'));
        else setTimeout(attempt, 200);
      });
    };
    attempt();
  });
}

function finish(ok) {
  report.ok = ok;
  report.totalMs = Date.now() - t0;
  const file = path.join(out, `e2e-${process.platform}.json`);
  fs.writeFileSync(file, JSON.stringify(report, null, 2));
  console.log(`smoke: ${ok ? 'passed' : 'FAILED'}; report and screenshots in ${out}`);
}

function fail(message) {
  console.error(`smoke: ${message}`);
  finish(false);
  driver.kill();
  process.exit(1);
}

try {
  await run();
  await wd.quit();
  finish(true);
  driver.kill();
  process.exit(0);
} catch (err) {
  await wd.quit();
  fail(err.message);
}
