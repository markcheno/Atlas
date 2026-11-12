#!/usr/bin/env node
// A knowledge store of any size, as a JSON export (phase3-spec §11, Slice
// 3.8): trails of searches, questions, opens and notes, with bookmarks,
// findings, interests and projects in proportion. It is written in the
// import format, so loading it through Import tests import too. Seeded:
// the same arguments give the same file.
//
//   node scripts/generate-knowledge.mjs <out.json> [--trails 10000] [--steps 100000]
//        [--documents docs.json] [--seed 1]
//
// --documents names a JSON array of {documentId, contentHash, title,
// fileType, pages} from a real catalog, so anchors resolve as found; without
// it, anchors name documents no catalog has. The first trail always has 50
// steps (the benchmark opens it), and the first project holds 900 trails
// and 99 findings: with its own page, 1,000 Markdown notes.

import fs from 'node:fs';
import path from 'node:path';

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1];
};
const out = args[0];
if (!out || out.startsWith('--')) {
  console.error('usage: generate-knowledge.mjs <out.json> [--trails N] [--steps N]');
  process.exit(2);
}
const trailCount = Number(opt('trails', '10000'));
const stepCount = Number(opt('steps', '100000'));
if (!Number.isInteger(trailCount) || trailCount < 1) throw new Error('--trails is a whole number');
if (!Number.isInteger(stepCount) || stepCount < trailCount + 49) {
  throw new Error('--steps is at least --trails + 49');
}
let seed = Number(opt('seed', '1')) >>> 0 || 1;

// mulberry32: small, fast and the same everywhere.
function random() {
  seed = (seed + 0x6d2b79f5) >>> 0;
  let t = seed;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
const pick = (list) => list[Math.floor(random() * list.length)];
const hex = (n) => Array.from({ length: n }, () => Math.floor(random() * 16).toString(16)).join('');
const uuid = () =>
  `${hex(8)}-${hex(4)}-4${hex(3)}-${pick(['8', '9', 'a', 'b'])}${hex(3)}-${hex(12)}`;

const WORDS = (
  'orbital harbour lantern gear spline resonance river archive stoic virtue reason ' +
  'tide schooner keeper ledger engine drive bearing truss comet nebula sourdough ' +
  'kitchen ballad sailor chapter margin index letter treaty empire coastline'
).split(' ');
const words = (n) => Array.from({ length: n }, () => pick(WORDS)).join(' ');
const sentence = (n) => {
  const w = words(n);
  return `${w[0].toUpperCase()}${w.slice(1)}.`;
};

const documents = opt('documents', null)
  ? JSON.parse(fs.readFileSync(opt('documents', null), 'utf8'))
  : Array.from({ length: 200 }, (_, i) => ({
      documentId: uuid(),
      contentHash: hex(64),
      title: `Generated document ${i + 1}`,
      fileType: 'pdf',
      pages: 1 + Math.floor(random() * 300),
    }));
if (documents.length === 0) throw new Error('--documents lists no documents');

function anchor(excerptChars) {
  const d = pick(documents);
  const a = {
    documentId: d.documentId,
    contentHash: d.contentHash,
    title: d.title,
    fileType: d.fileType,
  };
  if (d.fileType === 'pdf') a.pageNo = 1 + Math.floor(random() * Math.max(1, d.pages ?? 1));
  else a.location = 'Chapter 1, part 1';
  a.ordinal = 0;
  if (excerptChars > 0) {
    let text = '';
    while (text.length < excerptChars) text += `${sentence(12)} `;
    a.excerpt = text.slice(0, excerptChars).trim();
  }
  return a;
}

const BASE = Date.parse('2026-01-01T00:00:00Z');
const at = (ms) => new Date(ms).toISOString().replace(/\.\d{3}Z$/, 'Z');

function payload(kind) {
  switch (kind) {
    case 'search':
      return {
        kind,
        v: 1,
        words: words(2 + Math.floor(random() * 3)),
        filters: {},
        matched: 5 + Math.floor(random() * 400),
        results: Array.from({ length: 5 }, () => ({ anchor: anchor(300), why: 'text' })),
        semantic: { state: 'off' },
      };
    case 'ask': {
      const sources = Array.from({ length: 5 }, (_, i) => {
        const cited = i < 2;
        return { n: i + 1, cited, anchor: anchor(cited ? 600 : 0) };
      });
      return {
        kind,
        v: 1,
        question: `${words(5)}?`,
        filters: {},
        answer: `${sentence(30)} [1] ${sentence(30)} [2]`,
        ending: { kind: 'done' },
        cited: [1, 2],
        dropped: 0,
        weak: false,
        semantic: { state: 'off' },
        model: 'generated-chat',
        location: 'this_computer',
        sources,
      };
    }
    case 'open':
      return { kind, v: 1, anchor: anchor(0), via: 'result', actions: ['details'] };
    default:
      return { kind: 'note', v: 1, text: sentence(20) };
  }
}

const KINDS = ['search', 'search', 'search', 'search', 'ask', 'ask', 'ask', 'open', 'open', 'note'];

// Steps per trail: the first has 50, the rest share what is left.
const sizes = [50];
const rest = stepCount - 50;
for (let i = 1; i < trailCount; i += 1) {
  sizes.push(
    Math.floor((rest * i) / (trailCount - 1)) - Math.floor((rest * (i - 1)) / (trailCount - 1)),
  );
}
if (trailCount === 1) sizes[0] = stepCount;

const stream = fs.createWriteStream(out);
const write = (s) => (stream.write(s) ? null : new Promise((r) => stream.once('drain', r)));

const exported = at(BASE + trailCount * 3_600_000);
await write(
  `{"format":"atlas-knowledge","version":1,"exported":"${exported}","scope":"everything","trails":[`,
);
const trailIds = [];
const stepIds = [];
for (let t = 0; t < trailCount; t += 1) {
  const id = uuid();
  trailIds.push(id);
  const start = BASE + t * 3_600_000;
  const steps = [];
  for (let s = 0; s < Math.max(1, sizes[t]); s += 1) {
    const when = at(start + s * 60_000);
    const stepId = uuid();
    if (stepIds.length < 2_000) stepIds.push(stepId);
    steps.push({
      id: stepId,
      seq: s + 1,
      at: when,
      updatedAt: when,
      note: random() < 0.05 ? sentence(10) : null,
      payload: payload(s === 0 ? 'search' : pick(KINDS)),
    });
  }
  const last = steps[steps.length - 1].at;
  const trail = {
    id,
    title: random() < 0.3 ? words(3) : null,
    pinned: random() < 0.1,
    startedAt: steps[0].at,
    updatedAt: last,
    steps,
  };
  await write(`${t > 0 ? ',' : ''}${JSON.stringify(trail)}`);
}
await write('],"bookmarks":[');
const bookmarkCount = trailCount;
const bookmarkIds = [];
for (let b = 0; b < bookmarkCount; b += 1) {
  const id = uuid();
  bookmarkIds.push(id);
  const when = at(BASE + b * 3_600_000 + 30_000);
  const a = anchor(400);
  // One bookmark per place, as saving keeps them.
  a.ordinal = b;
  await write(
    `${b > 0 ? ',' : ''}${JSON.stringify({ id, anchor: a, note: random() < 0.3 ? sentence(8) : null, createdAt: when, updatedAt: when })}`,
  );
}
await write('],"findings":[');
const findingCount = Math.max(1, Math.floor(trailCount / 2));
const findingIds = [];
for (let f = 0; f < findingCount; f += 1) {
  const id = uuid();
  findingIds.push(id);
  const when = at(BASE + f * 7_200_000 + 45_000);
  const answer = random() < 0.6;
  const finding = answer
    ? {
        id,
        kind: 'answer',
        question: `${words(5)}?`,
        text: `${sentence(30)} [1]`,
        sources: [{ n: 1, anchor: anchor(600) }],
        note: random() < 0.4 ? sentence(10) : null,
        fromStepId: pick(stepIds),
        createdAt: when,
        updatedAt: when,
      }
    : (() => {
        const a = anchor(500);
        return {
          id,
          kind: 'passage',
          question: null,
          text: a.excerpt,
          sources: [{ n: 1, anchor: a }],
          note: null,
          fromStepId: null,
          createdAt: when,
          updatedAt: when,
        };
      })();
  await write(`${f > 0 ? ',' : ''}${JSON.stringify(finding)}`);
}
await write('],"collections":[');
const collectionCount = Math.max(1, Math.floor(trailCount / 100));
for (let c = 0; c < collectionCount; c += 1) {
  const project = c % 3 === 0;
  const when = at(BASE + c * 86_400_000);
  const items = [];
  const add = (kind, id) => items.push({ kind, id, contentHash: null, addedAt: when });
  if (c === 0) {
    trailIds.slice(0, 900).forEach((id) => add('trail', id));
    findingIds.slice(0, 99).forEach((id) => add('finding', id));
  } else {
    for (let k = 0; k < 10; k += 1) add('trail', pick(trailIds));
    for (let k = 0; k < 5; k += 1) add('bookmark', pick(bookmarkIds));
    for (let k = 0; k < 3; k += 1) add('finding', pick(findingIds));
    const d = pick(documents);
    items.push({ kind: 'document', id: d.documentId, contentHash: d.contentHash, addedAt: when });
  }
  const seen = new Set();
  const unique = items.filter(
    (i) => !seen.has(`${i.kind}:${i.id}`) && seen.add(`${i.kind}:${i.id}`),
  );
  const collection = {
    id: uuid(),
    kind: project ? 'project' : 'interest',
    name: c === 0 ? 'The thousand notes' : `${words(2)} ${c}`,
    description: random() < 0.5 ? sentence(15) : null,
    goal: project ? sentence(8) : null,
    status: 'active',
    createdAt: when,
    updatedAt: when,
    items: unique,
  };
  await write(`${c > 0 ? ',' : ''}${JSON.stringify(collection)}`);
}
await write('],"dismissedSuggestions":[]}\n');
await new Promise((resolve) => stream.end(resolve));
const bytes = fs.statSync(out).size;
console.log(
  JSON.stringify({
    file: path.resolve(out),
    trails: trailCount,
    steps: Math.max(stepCount, trailCount),
    bookmarks: bookmarkCount,
    findings: findingCount,
    collections: collectionCount,
    bytes,
  }),
);
