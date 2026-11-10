#!/usr/bin/env node
// A scripted OpenAI-compatible model service for the smoke test
// (phase2-spec §14): a models list, embeddings as a hashed bag of words,
// and a canned streamed answer that cites [1]. It listens on 127.0.0.1
// only, needs no key, and records what it was asked.
//
//   node e2e/fake-ai.mjs [--port 8765] [--dims 256]   (or `just fake-ai`)
//
// The embedding follows src-tauri/src/services/embedding/fake.rs: words
// split at anything not a letter or digit, common words dropped, each
// word hashed to a bucket with a sign, then normalised. It hashes with
// SHA-256 (Node has no BLAKE3), so its vectors differ from the Rust fake's
// but behave the same: texts sharing words are close.

import crypto from 'node:crypto';
import http from 'node:http';
import { fileURLToPath } from 'node:url';

export const EMBEDDING_MODEL = 'atlas-hashed-bag-of-words';
export const CHAT_MODEL = 'atlas-scripted-chat';

// src-tauri/src/services/search/query.rs STOPWORDS.
const STOPWORDS = new Set(
  (
    'a about an and are as at be been but by can could did do does for from had has have he her ' +
    'his how i if in into is it its of on or our she should so than that the their them then ' +
    'there they this to was we were what when where which who why will with would you your au ' +
    'aux ce dans de des du en est et il la le les un une pour que qui sur'
  ).split(' '),
);

/** The hashed bag-of-words vector of `text`, unit length. */
export function embed(text, dims) {
  const v = new Array(dims).fill(0);
  for (const word of text.split(/[^\p{L}\p{N}]+/u)) {
    if (!word || STOPWORDS.has(word.toLowerCase())) continue;
    const h = crypto.createHash('sha256').update(word.toLowerCase()).digest();
    const bucket = Number(h.readBigUInt64LE(0) % BigInt(dims));
    v[bucket] += (h[8] & 1) === 0 ? 1 : -1;
  }
  const norm = Math.hypot(...v);
  if (!Number.isFinite(norm) || norm === 0) {
    v.fill(0);
    v[0] = 1;
    return v;
  }
  return v.map((x) => x / norm);
}

/** The canned answer, in the pieces it streams. */
export const ANSWER_PIECES = [
  'From your library: ',
  'the lighthouse keeper ',
  'logged each passing schooner ',
  'in the fog [1].',
];

function json(res, status, body) {
  const text = JSON.stringify(body);
  res.writeHead(status, {
    'content-type': 'application/json',
    'content-length': Buffer.byteLength(text),
  });
  res.end(text);
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let data = '';
    req.setEncoding('utf8');
    req.on('data', (c) => (data += c));
    req.on('end', () => {
      try {
        resolve(data ? JSON.parse(data) : {});
      } catch (err) {
        reject(err);
      }
    });
    req.on('error', reject);
  });
}

async function stream(res, pieces) {
  res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' });
  for (const piece of pieces) {
    res.write(`data: ${JSON.stringify({ choices: [{ delta: { content: piece } }] })}\n\n`);
    await new Promise((r) => setTimeout(r, 30));
  }
  res.write('data: [DONE]\n\n');
  res.end();
}

/**
 * Starts the service. Resolves to `{ url, requests, close }`: `url` ends in
 * `/v1`; `requests` lists `{ method, path, body }` for every request.
 */
export function startFakeAi({ port = 0, dims = 256 } = {}) {
  const requests = [];
  const server = http.createServer(async (req, res) => {
    const path = new URL(req.url, 'http://localhost').pathname;
    let body = {};
    try {
      if (req.method === 'POST') body = await readBody(req);
    } catch {
      return json(res, 400, { error: { message: 'not JSON' } });
    }
    requests.push({ method: req.method, path, body });
    if (req.method === 'GET' && path === '/v1/models') {
      return json(res, 200, {
        object: 'list',
        data: [EMBEDDING_MODEL, CHAT_MODEL].map((id) => ({ id, object: 'model' })),
      });
    }
    if (req.method === 'POST' && path === '/v1/embeddings') {
      if (body.model !== EMBEDDING_MODEL) {
        return json(res, 404, { error: { message: `model '${body.model}' not found` } });
      }
      const inputs = Array.isArray(body.input) ? body.input : [body.input ?? ''];
      return json(res, 200, {
        object: 'list',
        model: EMBEDDING_MODEL,
        data: inputs.map((text, index) => ({
          object: 'embedding',
          index,
          embedding: embed(String(text), dims),
        })),
      });
    }
    if (req.method === 'POST' && path === '/v1/chat/completions') {
      if (body.model !== CHAT_MODEL) {
        return json(res, 404, { error: { message: `model '${body.model}' not found` } });
      }
      const last = body.messages?.at(-1)?.content ?? '';
      return stream(res, last.includes('Reply with the word OK') ? ['OK'] : ANSWER_PIECES);
    }
    return json(res, 404, { error: { message: `no route ${req.method} ${path}` } });
  });
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, '127.0.0.1', () => {
      const { port: bound } = server.address();
      resolve({
        url: `http://127.0.0.1:${bound}/v1`,
        requests,
        close: () => new Promise((r) => server.close(() => r())),
      });
    });
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const opt = (name, fallback) => {
    const i = args.indexOf(`--${name}`);
    return i === -1 ? fallback : Number(args[i + 1]);
  };
  const fake = await startFakeAi({ port: opt('port', 8765), dims: opt('dims', 256) });
  console.log(`fake AI service at ${fake.url}`);
  console.log(`  embedding model: ${EMBEDDING_MODEL}`);
  console.log(`  chat model:      ${CHAT_MODEL}`);
  console.log('Ctrl+C stops it.');
}
