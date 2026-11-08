// A stable hash of a generated fixture tree: every folder and regular file
// (relative path, and SHA-256 of the bytes), sorted. The symlinks and the
// locked folder are left out because they are not created on every OS (and
// the locked folder can't be read); the generator's summary covers them.
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

const SKIP = new Set(['locked', 'loop-to-root', 'link-outside']);

export function treeLines(root) {
  const lines = [];
  const walk = (dir, rel) => {
    for (const name of fs.readdirSync(dir).sort()) {
      if (rel === '' && SKIP.has(name)) continue;
      const abs = path.join(dir, name);
      const r = rel === '' ? name : `${rel}/${name}`;
      const st = fs.lstatSync(abs);
      if (st.isSymbolicLink()) continue;
      if (st.isDirectory()) {
        lines.push(`D ${r}`);
        walk(abs, r);
      } else if (st.isFile()) {
        lines.push(`F ${r} ${createHash('sha256').update(fs.readFileSync(abs)).digest('hex')}`);
      }
    }
  };
  walk(root, '');
  return lines;
}

export function treeHash(root) {
  return createHash('sha256').update(treeLines(root).join('\n')).digest('hex');
}
