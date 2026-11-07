import { describe, expect, it } from 'vitest';
import { formatBytes } from './format';

describe('formatBytes', () => {
  it('uses decimal units with sensible precision', () => {
    expect(formatBytes(0)).toBe('0 bytes');
    expect(formatBytes(999)).toBe('999 bytes');
    expect(formatBytes(1_500_000)).toBe('1.5 MB');
    expect(formatBytes(245_100_000_000)).toBe('245 GB');
    expect(formatBytes(2_000_000_000_000)).toBe('2.0 TB');
  });

  it('reports unknown values honestly', () => {
    expect(formatBytes(null)).toBe('Unknown');
    expect(formatBytes(undefined)).toBe('Unknown');
  });
});
