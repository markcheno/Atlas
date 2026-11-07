/**
 * Shared fake of the Tauri core module. It mirrors the Rust Slice 0.1
 * contract: get_app_status returns facts and every other command rejects
 * with a not_implemented ErrorPayload.
 */
import { vi } from 'vitest';
import type { AppStatus } from '../lib/api/bindings/AppStatus';
import type { ErrorPayload } from '../lib/api/bindings/ErrorPayload';

export const fakeStatus: AppStatus = {
  appVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  buildProfile: 'debug',
  slice: '0.1',
  features: [],
};

const plannedSlices: Record<string, string> = {
  list_documents: '0.6',
  list_source_roots: '0.3',
};

export const invoke = vi.fn(async (command: string): Promise<unknown> => {
  if (command === 'get_app_status') return fakeStatus;
  const slice = plannedSlices[command] ?? '0.x';
  const payload: ErrorPayload = {
    code: 'not_implemented',
    message: `${command} is not available yet; planned for Phase 0 slice ${slice}`,
    command,
    plannedSlice: slice,
  };
  throw payload;
});

export const isTauri = vi.fn(() => true);
