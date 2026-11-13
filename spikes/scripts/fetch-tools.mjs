#!/usr/bin/env node
// Downloads the pinned spike tools (spikes/tools.lock.json) into
// spikes/.tools/, verifying each SHA-256 before unpacking. Nothing is
// installed system-wide. Already-present tools are kept.
//
//   node spikes/scripts/fetch-tools.mjs [pdfium] [protoc] [typst] [onnxruntime] [cmake]
//                                        [ppocr-models] [tessdata]
//
// A tool with `files` (models) is the same on every platform: each file is
// verified and stored under its name, not unpacked.
//
// Writes spikes/.tools/paths.json, which the other spike scripts read.

import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const spikes = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const toolsDir = path.join(spikes, '.tools');
const lock = JSON.parse(fs.readFileSync(path.join(spikes, 'tools.lock.json'), 'utf8'));
// PDFium moved to the app's lock file (Slice 1.2); the spike uses the same pin.
const appLock = JSON.parse(fs.readFileSync(path.join(spikes, '..', 'tools.lock.json'), 'utf8'));
lock.tools = { pdfium: appLock.tools.pdfium, ...lock.tools };

export const platformKey = () => {
  const key = `${process.platform}-${process.arch}`;
  if (!['darwin-arm64', 'linux-x64', 'win32-x64'].includes(key)) {
    throw new Error(`spike tools are pinned for darwin-arm64, linux-x64 and win32-x64, not ${key}`);
  }
  return key;
};

const exe = (name) => (process.platform === 'win32' ? `${name}.exe` : name);

// Where each tool's useful file ends up inside its unpacked folder.
const layout = {
  pdfium: (dir) => ({
    // pdfium-render's bind_to_library wants the library file itself.
    library: path.join(
      dir,
      process.platform === 'win32' ? 'bin' : 'lib',
      { darwin: 'libpdfium.dylib', linux: 'libpdfium.so', win32: 'pdfium.dll' }[process.platform],
    ),
    // PDFium's BSD-3-Clause licence; the archive's root LICENSE is the MIT
    // licence of bblanchon's build scripts.
    license: path.join(dir, 'licenses', 'pdfium.txt'),
  }),
  protoc: (dir) => ({
    bin: path.join(dir, 'bin', exe('protoc')),
    include: path.join(dir, 'include'),
  }),
  typst: (dir) => {
    const inner = fs.readdirSync(dir).find((d) => d.startsWith('typst-'));
    return { bin: path.join(dir, inner ?? '', exe('typst')) };
  },
  // The OCR spike (Slice 4.2).
  onnxruntime: (dir) => {
    const inner = fs.readdirSync(dir).find((d) => d.startsWith('onnxruntime-')) ?? '';
    const lib = path.join(dir, inner, 'lib');
    const name = fs.existsSync(lib)
      ? fs
          .readdirSync(lib)
          .find((f) =>
            process.platform === 'win32'
              ? f === 'onnxruntime.dll'
              : process.platform === 'darwin'
                ? /^libonnxruntime\.\d+\.\d+\.\d+\.dylib$/.test(f)
                : /^libonnxruntime\.so\.\d+\.\d+\.\d+$/.test(f),
          )
      : undefined;
    return {
      library: path.join(lib, name ?? 'missing'),
      license: path.join(dir, inner, 'LICENSE'),
      notices: path.join(dir, inner, 'ThirdPartyNotices.txt'),
    };
  },
  cmake: (dir) => {
    const inner = fs.readdirSync(dir).find((d) => d.startsWith('cmake-')) ?? '';
    return {
      bin:
        process.platform === 'darwin'
          ? path.join(dir, inner, 'CMake.app', 'Contents', 'bin', 'cmake')
          : path.join(dir, inner, 'bin', exe('cmake')),
    };
  },
  'ppocr-models': (dir) => ({ dir }),
  tessdata: (dir) => ({ dir }),
};

async function download(name, url, sha256) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${name}: HTTP ${res.status} for ${url}`);
  const bytes = Buffer.from(await res.arrayBuffer());
  const digest = createHash('sha256').update(bytes).digest('hex');
  if (digest !== sha256) {
    throw new Error(`${name}: SHA-256 mismatch for ${url} (expected ${sha256}, got ${digest})`);
  }
  return bytes;
}

/** A tool of single files: each verified, stored under its name. */
async function fetchFiles(name, tool, dir) {
  // The marker is one hash over every file's pin, so a changed pin fetches again.
  const pin = createHash('sha256').update(JSON.stringify(tool.files)).digest('hex');
  const marker = path.join(dir, '.sha256');
  if (fs.existsSync(marker) && fs.readFileSync(marker, 'utf8') === pin) {
    return { name, version: tool.version, dir, ...layout[name](dir) };
  }
  process.stdout.write(`spike-tools: ${name} ${tool.version}… `);
  fs.rmSync(dir, { recursive: true, force: true });
  let total = 0;
  for (const [file, { url, sha256 }] of Object.entries(tool.files)) {
    const bytes = await download(name, url, sha256);
    const to = path.join(dir, file);
    fs.mkdirSync(path.dirname(to), { recursive: true });
    fs.writeFileSync(to, bytes);
    total += bytes.length;
  }
  fs.writeFileSync(marker, pin);
  console.log(`ok (${Object.keys(tool.files).length} files, ${(total / 1048576).toFixed(1)} MiB)`);
  return { name, version: tool.version, dir, ...layout[name](dir) };
}

async function fetchTool(name) {
  const tool = lock.tools[name];
  if (!tool) throw new Error(`unknown tool ${name}`);
  const dir = path.join(toolsDir, `${name}-${tool.version.replace(/[^\w.-]/g, '_')}`);
  if (tool.files) return fetchFiles(name, tool, dir);
  const asset = tool.assets[platformKey()];
  const marker = path.join(dir, '.sha256');
  if (fs.existsSync(marker) && fs.readFileSync(marker, 'utf8') === asset.sha256) {
    return { name, version: tool.version, dir, ...layout[name](dir) };
  }

  process.stdout.write(`spike-tools: ${name} ${tool.version}… `);
  const bytes = await download(name, asset.url, asset.sha256);

  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  const archive = path.join(toolsDir, path.basename(new URL(asset.url).pathname));
  fs.writeFileSync(archive, bytes);
  if (archive.endsWith('.zip') && process.platform === 'linux') {
    execFileSync('unzip', ['-q', archive, '-d', dir]);
  } else {
    // bsdtar (macOS, Windows) reads zip, tgz and tar.xz; GNU tar reads tgz and tar.xz.
    execFileSync('tar', ['-xf', archive, '-C', dir]);
  }
  fs.rmSync(archive);
  fs.writeFileSync(marker, asset.sha256);
  console.log(`ok (${(bytes.length / 1048576).toFixed(1)} MiB)`);
  return { name, version: tool.version, dir, ...layout[name](dir) };
}

export function readPaths() {
  const file = path.join(toolsDir, 'paths.json');
  if (!fs.existsSync(file)) throw new Error('run `just spike-tools` first');
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const wanted = process.argv.slice(2).filter((a) => !a.startsWith('--'));
  const names = wanted.length ? wanted : Object.keys(lock.tools);
  fs.mkdirSync(toolsDir, { recursive: true });
  const file = path.join(toolsDir, 'paths.json');
  const paths = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, 'utf8')) : {};
  for (const name of names) {
    const got = await fetchTool(name);
    for (const p of Object.values(got)) {
      if (typeof p === 'string' && p.startsWith(toolsDir) && !fs.existsSync(p)) {
        throw new Error(`${name}: expected ${p} after unpacking`);
      }
    }
    paths[name] = got;
  }
  fs.writeFileSync(file, JSON.stringify(paths, null, 2));
  console.log(`spike-tools: ${names.join(', ')} ready in ${toolsDir}`);
}
