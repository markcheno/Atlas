// The app's CSP blocks inline styles: Tauri adds a nonce to style-src, and
// WebKit then ignores 'unsafe-inline' (Slice 3.9, found by the release DMG
// smoke: the page view's outline was never drawn). jsdom doesn't apply a
// CSP, so this check stands in for it: no component sets a style attribute.
import { describe, expect, it } from 'vitest';

const sources = import.meta.glob('../**/*.svelte', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

describe('the CSP', () => {
  it('no component sets an inline style, which the app would block', () => {
    const files = Object.keys(sources);
    expect(files.length).toBeGreaterThan(20);
    const offenders = Object.entries(sources)
      .filter(([, text]) =>
        /\sstyle(=|:[a-z-]+=)/.test(text.replace(/<style[\s\S]*?<\/style>/g, '')),
      )
      .map(([file]) => file);
    expect(offenders).toEqual([]);
  });
});
