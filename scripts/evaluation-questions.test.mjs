// The committed retrieval evaluation sets (fixtures/evaluation/*.json,
// phase1-spec §10 and phase2-spec §10) stay well formed and answerable: every
// relevant id is a corpus document that extracts to text. node --test.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const manifest = JSON.parse(
  fs.readFileSync(path.join(repo, 'spikes/extraction-bench/corpus/manifest.json'), 'utf8'),
);
const readable = new Set(manifest.documents.filter((d) => d.expect === 'text').map((d) => d.id));

const sets = [
  { file: 'questions.json', kinds: ['exact', 'phrase', 'paraphrase'], count: 40 },
  { file: 'questions-vague.json', kinds: ['vague'], count: 40 },
  // Questions the corpus has nothing on (just eval-ask, Slice 2.6.2).
  { file: 'questions-off.json', kinds: ['off'], count: 12, off: true },
];

for (const { file, kinds, count, off } of sets) {
  test(`${file} is well formed and every relevant document has text`, () => {
    const set = JSON.parse(fs.readFileSync(path.join(repo, 'fixtures/evaluation', file), 'utf8'));
    assert.equal(set.version, 1);
    assert.equal(set.questions.length, count);
    const ids = new Set();
    for (const q of set.questions) {
      assert.match(q.id, /^[a-z]\d{2}$/, `${file}: id ${q.id}`);
      assert.ok(!ids.has(q.id), `${file}: duplicate id ${q.id}`);
      ids.add(q.id);
      assert.ok(kinds.includes(q.kind), `${file} ${q.id}: kind ${q.kind}`);
      assert.ok(q.question.trim().length >= 8, `${file} ${q.id}: question too short`);
      assert.ok(Array.isArray(q.relevant), `${file} ${q.id}: no relevant list`);
      if (off) {
        assert.equal(q.relevant.length, 0, `${file} ${q.id}: an off question has no document`);
      } else {
        assert.ok(q.relevant.length > 0, `${file} ${q.id}: no relevant`);
      }
      for (const r of q.relevant) {
        assert.ok(readable.has(r), `${file} ${q.id}: ${r} is not a readable corpus document`);
      }
    }
  });
}

test('every readable corpus document is asked about in the vague set', () => {
  const set = JSON.parse(
    fs.readFileSync(path.join(repo, 'fixtures/evaluation/questions-vague.json'), 'utf8'),
  );
  const asked = new Set(set.questions.flatMap((q) => q.relevant));
  const missing = [...readable].filter((id) => !asked.has(id));
  assert.deepEqual(missing, [], `readable documents never asked about: ${missing.join(', ')}`);
});
