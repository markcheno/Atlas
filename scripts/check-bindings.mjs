#!/usr/bin/env node
// Fails when generated TypeScript bindings differ from what is committed.
// Run after `cargo test`, which regenerates src/lib/api/bindings via ts-rs.
import { execFileSync } from 'node:child_process';

const dir = 'src/lib/api/bindings';
const git = (...args) => execFileSync('git', args, { encoding: 'utf8' }).trim();

const changed = git('status', '--porcelain', '--', dir);
if (changed) {
  console.error(`Generated bindings are out of date:\n${changed}`);
  console.error('Run `pnpm check:rust` (or `cargo test`) and commit the updated bindings.');
  process.exit(1);
}
console.log('Bindings are up to date.');
