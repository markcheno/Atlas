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
import { CHAT_MODEL, EMBEDDING_MODEL, startFakeAi } from './fake-ai.mjs';
import { WebDriver, button, lit, sleep } from './webdriver.mjs';
import { textEpub } from '../scripts/lib/fixture-text.mjs';

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
// One short book in words of its own, for meaning search and Ask Atlas
// (Slice 2.7): the fixture's longer, random-word texts are too diluted for
// a short question to come close to them.
const LIGHTHOUSE_BOOK = "Lighthouse keeper's log.epub";
const LIGHTHOUSE_TITLE = "The Lighthouse Keeper's Log";
fs.writeFileSync(
  path.join(fixture, LIGHTHOUSE_BOOK),
  textEpub({
    id: 'smoke-lighthouse',
    title: LIGHTHOUSE_TITLE,
    author: 'Mara Quill',
    chapters: 1,
    paragraphs: [
      'The lighthouse keeper trimmed the lamp wick at dusk and logged every passing schooner through the fog.',
      'Each night the keeper wrote the schooner names, the fog horn signals and the tide in a leather log.',
    ],
  }).bytes,
);
// In other words than the book's: no phrase of it, a word it lacks.
const VAGUE_QUESTION = 'the keeper who logged passing schooners in fog';
const expected = summary.catalogableFiles + 1;
const dataRoot = path.join(scratch, 'AtlasData');
fs.mkdirSync(dataRoot);
const openerLog = path.join(scratch, 'opened.log');
// Where "a trail is exported" writes (Slice 3.8): outside the library
// folder and the data root, as Atlas requires.
const exportDir = path.join(scratch, 'Exports');
fs.mkdirSync(exportDir);

const env = {
  ...process.env,
  ATLAS_CONFIG_DIR: path.join(scratch, 'config'),
  ATLAS_E2E_PICK_DATA_ROOT: dataRoot,
  ATLAS_E2E_PICK_SOURCES: fixture,
  ATLAS_E2E_OPENER_LOG: openerLog,
  ATLAS_E2E_PICK_EXPORT_DIR: exportDir,
  // Catalog slowly enough (debug builds only) that "pause and resume
  // cataloging" always finds the scan still running, however fast the runner.
  ATLAS_INVENTORY_RATE: '150',
  // Index meaning without the job's rests (debug builds only).
  ATLAS_EMBED_UNTHROTTLED: '1',
  // Each meaning search's sizes and similarities on stderr (debug builds).
  ATLAS_E2E_TRACE_MEANING: '1',
};

// The scripted model service the app is pointed at in Settings (Slice 2.7).
const fake = await startFakeAi();

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

// tauri-driver forwards the session to the native driver (msedgedriver on
// Windows, WebKitWebDriver on Linux) on port 4445. On Windows runners that
// driver sometimes refused the first connection (Slices 1.5–1.7), failing
// the run before the app opened. Wait briefly for its port, and retry a
// session that fails to connect, twice (both of two attempts were refused
// once in Slice 2.6.2); the report records each retry.
const NATIVE_PORT = 4445;
const CONNECT_ERROR = /fetch failed|ECONNREFUSED|actively refused|client error \(Connect\)/i;

async function launch() {
  const started = Date.now();
  for (let attempt = 1; ; attempt += 1) {
    try {
      await wd.start({ 'tauri:options': { application: app } });
      return started;
    } catch (err) {
      if (attempt >= 3 || !CONNECT_ERROR.test(String(err?.cause ?? err?.message ?? err))) throw err;
      report.sessionRetries = (report.sessionRetries ?? 0) + 1;
      console.log(`smoke: the driver refused the session (${err.message}); retrying`);
      await sleep(3_000);
    }
  }
}

async function run() {
  await waitForPort(4444, 30_000);
  // Best effort: some versions start the native driver only per session.
  report.nativeDriverReady = await waitForPort(NATIVE_PORT, 10_000).then(
    () => true,
    () => false,
  );

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

  // phase1-spec §14: smoke covers pausing and resuming extraction. After
  // the relaunch about a tenth of the 1,000 files has been read (8–23 s of
  // reading left on the runners), so the reading line has its controls.
  await step('pause and resume reading', async () => {
    await wd.click(
      await wd.waitFor("//button[@aria-label='Pause reading text']", { timeout: 30_000 }),
    );
    const paused = await wd.until('reading paused', async () => {
      const t = await readingStatus();
      return t.startsWith('Paused reading text by you') && t;
    });
    await sleep(1_500);
    const still = await readingStatus();
    if (still !== paused) throw new Error(`reading went on while paused: ${paused} → ${still}`);
    await wd.click(await wd.waitFor("//button[@aria-label='Resume reading text']"));
    const resumed = await wd.until('reading resumed', async () => {
      const t = await readingStatus();
      return !t.startsWith('Paused') && t;
    });
    return { paused, resumed };
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

  // phase1-spec §14: open from a result. The search view is still showing
  // the hit for the passage found above.
  await step('open from a search result', async () => {
    const opens = () =>
      (fs.existsSync(openerLog) ? fs.readFileSync(openerLog, 'utf8') : '')
        .split('\n')
        .filter((l) => l.startsWith('open ') && l.includes('Café crème book 000000'));
    const before = opens().length;
    await wd.click(
      await wd.waitFor(
        `//li[@data-testid='search-hit'][.//*[normalize-space(.)=${lit('Café crème book 000000.epub')}]]//button[starts-with(@aria-label, 'Open ')]`,
      ),
    );
    const line = await wd.until('open request from the result', async () => {
      const all = opens();
      return all.length > before && all[all.length - 1];
    });
    const opened = line.slice('open '.length);
    if (!fs.existsSync(opened)) throw new Error(`opened a path that doesn't exist: ${opened}`);
    return {
      opened: path.relative(fs.realpathSync.native(fixture), fs.realpathSync.native(opened)),
    };
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

  // phase2-spec §14 (Slice 2.7): a model service set up through Settings
  // as a user would, the library indexed for meaning, a vague question
  // found by meaning, and Ask Atlas answering with a citation that opens.
  await step('set up the scripted AI service', async () => {
    await wd.click(await wd.waitFor(button('Add a service…')));
    const form = "//form[@aria-label='Add an AI service']";
    const field = (name, control) =>
      wd.waitFor(`${form}//label[.//span[normalize-space(.)=${lit(name)}]]//${control}`);
    await wd.select(await field('Service', 'select'), 'Other OpenAI-compatible');
    const name = await field('Name', 'input');
    await wd.clear(name);
    await wd.type(name, 'Scripted AI');
    const address = await field('Address', 'input');
    await wd.clear(address);
    await wd.type(address, fake.url);
    await wd.click(await wd.waitFor(`${form}${button('Test')}`, { enabled: true }));
    const result = await wd.text(
      await wd.waitFor(`${form}//*[@data-outcome='reachable']`, { timeout: 30_000 }),
    );
    await wd.click(await wd.waitFor(`${form}${button('Save')}`));
    await wd.waitFor("//button[@aria-label='Test Scripted AI']");
    return { url: fake.url, result };
  });

  await step('choose the embedding and chat models', async () => {
    const results = {};
    for (const [role, title, model] of [
      ['embeddings', 'Embeddings', EMBEDDING_MODEL],
      ['chat', 'Chat', CHAT_MODEL],
    ]) {
      const group = `//*[@role='group'][@aria-label=${lit(title)}]`;
      await wd.click(
        await wd.waitFor(`//button[@aria-label=${lit(`Choose a model for ${role}`)}]`),
      );
      const box = await wd.waitFor(`${group}//input[@placeholder='Choose or type a model id']`);
      await wd.clear(box);
      await wd.type(box, model);
      await wd.click(await wd.waitFor(`${group}${button('Test')}`, { enabled: true }));
      results[role] = await wd.text(
        await wd.waitFor(`${group}//*[@data-outcome='works']`, { timeout: 60_000 }),
      );
      await wd.click(await wd.waitFor(`${group}${button('Use')}`, { enabled: true }));
      await wd.waitFor(
        `//*[@data-testid=${lit(`role-${role === 'chat' ? 'chat' : 'embedding'}`)}]`,
        {
          timeout: 60_000,
        },
      );
    }
    const embedding = await wd.text(await wd.waitFor("//*[@data-testid='role-embedding']"));
    if (!embedding.includes('256 dimensions')) throw new Error(`embedding role: ${embedding}`);
    return { ...results, embedding };
  });

  await step('index meaning', async () => {
    const line = await wd.until(
      'meaning up to date',
      async () => {
        const els = await wd.all("//*[@data-testid='embedding-status']");
        const t = els.length > 0 ? await wd.text(els[0]) : '';
        return t.includes('Meaning up to date') && t;
      },
      600_000,
    );
    report.timingsMs.meaningUpToDate = Date.now() - t0;
    return { status: line };
  });

  await step('a vague question finds the document by meaning', async () => {
    await wd.click(await wd.waitFor(button('Search')));
    await wd.waitFor(heading('Search'));
    await wd.click(await wd.waitFor("//label[normalize-space(.)='Find sources']"));
    const box = await wd.waitFor("//input[@id='search-input']");
    await wd.clear(box);
    //  is WebDriver's Enter key.
    await wd.type(box, `${VAGUE_QUESTION}`);
    const hitPath = `//li[@data-testid='search-hit'][.//h3[normalize-space(.)=${lit(LIGHTHOUSE_TITLE)}]]`;
    // A person searches again if meaning was still settling; each attempt,
    // and any note about meaning search, goes in the report.
    const attempts = [];
    for (let attempt = 1; attempt <= 4; attempt += 1) {
      if (attempt > 1) {
        await sleep(2_000);
        await wd.click(await wd.waitFor(`//form[@role='search']${button('Search')}`));
      }
      const text = await wd.text(await wd.waitFor(hitPath, { timeout: 30_000 }));
      const notes = await wd.all("//*[@data-testid='meaning-note']");
      const note = notes.length > 0 ? await wd.text(notes[0]) : null;
      attempts.push({ why: text.split('\n').find((l) => /In the|Similar/.test(l)) ?? null, note });
      if (/similar meaning/i.test(text)) {
        return { result: text.split('\n').slice(0, 4).join(' / '), attempts };
      }
    }
    // Diagnostics: did the question reach the service, and does Related
    // (the book's own vectors, no query embedding) find neighbours?
    const asked = fake.requests.filter(
      (r) => r.path === '/v1/embeddings' && JSON.stringify(r.body.input).includes('schooners'),
    );
    let related;
    try {
      await wd.click(
        await wd.waitFor(`${hitPath}//button[starts-with(@aria-label, 'More like this')]`),
      );
      await sleep(3_000);
      const panel = await wd.all("//*[@role='dialog']");
      related = panel.length > 0 ? (await wd.text(panel[0])).slice(0, 600) : 'no panel';
    } catch (err) {
      related = `error: ${err.message}`;
    }
    throw new Error(
      `not found by meaning: ${JSON.stringify({
        attempts: attempts.map((a) => a.note),
        questionEmbeddings: asked.length,
        inputs: asked.slice(0, 2).map((r) => r.body.input),
        embeddingRequests: fake.requests.filter((r) => r.path === '/v1/embeddings').length,
        related,
      })}`,
    );
  });

  await step('ask Atlas answers with a citation that opens the source', async () => {
    await wd.click(await wd.waitFor("//label[normalize-space(.)='Ask Atlas']"));
    const field = await wd.waitFor("//textarea[@id='ask-input']");
    await wd.type(field, VAGUE_QUESTION);
    await wd.click(await wd.waitFor(button('Ask'), { enabled: true }));
    // Done: Ask is back in Stop's place, with the answer above Sources used.
    const article = await wd.until(
      'the whole answer',
      async () => {
        const ask = await wd.all(button('Ask'));
        const found = await wd.all("//article[@aria-labelledby='answer-heading']");
        return ask.length > 0 && found.length > 0 && found[0];
      },
      60_000,
    );
    const answer = await wd.text(article);
    if (!answer.includes('lighthouse keeper')) throw new Error(`answer: ${answer}`);
    const sources = await wd.all("//*[@data-testid='source-used']");
    if (sources.length === 0) throw new Error('no sources used listed');
    await wd.click(await wd.waitFor("//article//button[starts-with(@aria-label, 'Source 1: ')]"));
    const cited = await wd.text(
      await wd.waitFor("//*[@role='dialog']//*[@data-testid='cited-passage']", {
        timeout: 30_000,
      }),
    );
    if (!cited.includes('Source 1')) throw new Error(`cited passage: ${cited}`);
    await wd.click(await wd.waitFor("//button[@aria-label='Close details']"));
    const asked = fake.requests.filter(
      (r) =>
        r.path === '/v1/chat/completions' &&
        !String(r.body.messages?.at(-1)?.content).includes('Reply with the word OK'),
    );
    if (asked.length !== 1) throw new Error(`${asked.length} questions reached the service`);
    if (!asked[0].body.messages.at(-1).content.includes('<source id="1"')) {
      throw new Error('the question carried no sources');
    }
    return { answer, sources: sources.length, cited: cited.split('\n').slice(0, 3).join(' / ') };
  });

  // Phase 3 (phase3-spec §14): what was searched and asked is a trail.
  await step('a search and a question are recorded', async () => {
    await openNewestTrail();
    const asked = await wd.waitFor(
      `//li[@data-testid='trail-step'][@data-kind='ask']//h3[contains(., ${lit(VAGUE_QUESTION)})]`,
    );
    const searches = await wd.all("//li[@data-testid='trail-step'][@data-kind='search']");
    if (searches.length === 0) throw new Error('no search step in the trail');
    const steps = await wd.all("//li[@data-testid='trail-step']");
    return { steps: steps.length, searches: searches.length, asked: await wd.text(asked) };
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

  await step('the trail is resumed and its citation opens', async () => {
    await openNewestTrail();
    // The session can outlive a relaunch: then it is already continuing.
    const resume = await wd.all(button('Continue this trail'));
    if (resume.length > 0) await wd.click(resume[0]);
    await wd.waitFor("//*[@data-testid='continuing']");
    await wd.click(
      await wd.waitFor(
        "//li[@data-testid='trail-step'][@data-kind='ask']//article//button[starts-with(@aria-label, 'Source 1: ')]",
      ),
    );
    const cited = await wd.text(
      await wd.waitFor("//*[@role='dialog']//*[@data-testid='cited-passage']", {
        timeout: 30_000,
      }),
    );
    await wd.click(await wd.waitFor("//button[@aria-label='Close details']"));
    return { cited: cited.split('\n').slice(0, 3).join(' / ') };
  });

  const resultButton = (label) =>
    `(//li[@data-testid='search-hit']//button[starts-with(@aria-label, ${lit(label)})])[1]`;

  await step('a bookmark is saved', async () => {
    await wd.click(await wd.waitFor(button('Search')));
    await wd.waitFor(heading('Search'));
    await wd.click(await wd.waitFor("//label[normalize-space(.)='Find sources']"));
    const box = await wd.waitFor("//input[@id='search-input']");
    await wd.clear(box);
    // \uE007 is WebDriver's Enter key.
    await wd.type(box, 'Engineering\uE007');
    await wd.click(await wd.waitFor(resultButton('Bookmark: '), { timeout: 30_000 }));
    await wd.waitFor(
      "//*[starts-with(@data-testid, 'save-status-bookmark-hit-')][normalize-space(.)='Bookmarked']",
    );
    await wd.click(await wd.waitFor(button('Trails')));
    await wd.waitFor(heading('Trails'));
    await wd.click(await wd.waitFor("//label[normalize-space(.)='Saved']"));
    const item = await wd.waitFor("//li[@data-testid='saved-item'][@data-kind='bookmark']");
    return { saved: (await wd.text(item)).split('\n').slice(0, 2).join(' / ') };
  });

  await step('the page view shows a page', async () => {
    await wd.click(await wd.waitFor(button('Search')));
    await wd.waitFor(heading('Search'));
    const show = await wd.waitFor(resultButton('Show the page: '), { timeout: 30_000 });
    const label = await wd.execute(
      "return document.evaluate(arguments[0], document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue?.getAttribute('aria-label') ?? '';",
      [resultButton('Show the page: ')],
    );
    await wd.click(show);
    await wd.waitFor("//*[@data-testid='page-image']//img", { timeout: 60_000 });
    const width = await wd.until('the page image decoded', async () => {
      const w = await wd.execute(
        'return document.querySelector("[data-testid=\'page-image\'] img")?.naturalWidth ?? 0;',
      );
      return w > 0 && w;
    });
    await wd.click(await wd.waitFor("//button[@aria-label='Close the page']"));
    return { page: label, width };
  });

  await step('a trail is exported to a scratch folder', async () => {
    await openNewestTrail();
    await wd.click(await wd.waitFor(button('Export…')));
    await wd.click(await wd.waitFor(button('Choose a folder and export…'), { enabled: true }));
    const result = await wd.text(
      await wd.waitFor("//*[@data-testid='export-result']", { timeout: 30_000 }),
    );
    const trails = path.join(exportDir, 'Atlas', 'Trails');
    const notes = fs.existsSync(trails)
      ? fs.readdirSync(trails).filter((n) => n.endsWith('.md'))
      : [];
    if (notes.length !== 1) throw new Error(`notes written: ${JSON.stringify(notes)}`);
    const text = fs.readFileSync(path.join(trails, notes[0]), 'utf8');
    if (!/^---\natlas_id: "[0-9a-f-]{36}"\natlas_kind: "trail"\n/.test(text)) {
      throw new Error(`front matter: ${text.slice(0, 200)}`);
    }
    if (!text.includes(VAGUE_QUESTION)) throw new Error('the note lacks the question');
    await wd.click(await wd.waitFor(button('Done')));
    return { result, note: notes[0] };
  });
}

/** Opens Trails and its newest trail's timeline. */
async function openNewestTrail() {
  await wd.click(await wd.waitFor(button('Trails')));
  await wd.waitFor(heading('Trails'));
  const tabs = await wd.all("//label[normalize-space(.)='Trails']");
  if (tabs.length > 0) await wd.click(tabs[0]);
  // A trail opened earlier is still showing: back to the list first.
  const back = await wd.all(button('Back to trails'));
  if (back.length > 0) await wd.click(back[0]);
  await wd.click(
    await wd.waitFor(
      "(//li[@data-testid='trail-row']//button[contains(@class, 'name-button')])[1]",
    ),
  );
  await wd.waitFor("//li[@data-testid='trail-step']", { timeout: 30_000 });
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
  await fake.close();
  process.exit(0);
} catch (err) {
  await wd.quit();
  fail(err.message);
}
