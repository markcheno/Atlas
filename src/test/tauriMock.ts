/**
 * Shared fake of the Tauri core module, mirroring the Rust contract.
 * Tests override individual commands with `setHandler` and call
 * `resetHandlers` in `beforeEach`. Unhandled commands reject with a
 * not_implemented ErrorPayload, like the real stubs.
 */
import { vi } from 'vitest';
import type { AppStatus } from '../lib/api/bindings/AppStatus';
import type { DataRootCandidate } from '../lib/api/bindings/DataRootCandidate';
import type { DataRootStatus } from '../lib/api/bindings/DataRootStatus';
import type { ErrorPayload } from '../lib/api/bindings/ErrorPayload';
import type { Settings } from '../lib/api/bindings/Settings';

export const DEFAULT_ROOT = '/Users/test/Library/Application Support/com.altazlabs.atlas/Library';

export const fakeStatus: AppStatus = {
  appVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  buildProfile: 'debug',
  slice: '0.2',
  features: [],
};

export function openStatus(path = DEFAULT_ROOT): DataRootStatus {
  return {
    state: 'open',
    displayPath: path,
    rootId: '8a3e2a39-6c39-4f5c-8f55-7f3f0bdfa001',
    problem: null,
  };
}

export function candidate(overrides: Partial<DataRootCandidate> = {}): DataRootCandidate {
  return {
    displayPath: DEFAULT_ROOT,
    kind: 'new_folder',
    problems: [],
    canUse: true,
    freeBytes: 245_100_000_000,
    totalBytes: 994_700_000_000,
    requiredFreeBytes: 10_737_418_240,
    volumeKind: 'local',
    suggestedSubfolder: null,
    rootId: null,
    ...overrides,
  };
}

export function apiError(code: ErrorPayload['code'], message: string, recovery?: string) {
  const payload: ErrorPayload = { code, message, recovery };
  return payload;
}

type Handler = (args: Record<string, unknown> | undefined) => unknown;

const plannedSlices: Record<string, string> = {
  list_documents: '0.6',
  list_source_roots: '0.3',
};

function defaults(): Record<string, Handler> {
  let settings: Settings = { appearance: { theme: 'system' } };
  return {
    get_app_status: () => fakeStatus,
    get_data_root_status: (): DataRootStatus => ({
      state: 'not_configured',
      displayPath: null,
      rootId: null,
      problem: null,
    }),
    get_default_data_root: () => DEFAULT_ROOT,
    pick_folder: () => null,
    inspect_data_root: (args) => candidate({ displayPath: String(args?.path) }),
    choose_data_root: (args) => openStatus(String(args?.path)),
    open_data_root: (args) => openStatus(String(args?.path)),
    get_settings: () => settings,
    update_settings: (args) => {
      settings = args?.settings as Settings;
      return settings;
    },
  };
}

let handlers = defaults();

export function resetHandlers() {
  handlers = defaults();
  invoke.mockClear();
}

export function setHandler(command: string, handler: Handler) {
  handlers[command] = handler;
}

export const invoke = vi.fn(
  async (command: string, args?: Record<string, unknown>): Promise<unknown> => {
    const handler = handlers[command];
    if (handler) return handler(args);
    const slice = plannedSlices[command] ?? '0.x';
    const payload: ErrorPayload = {
      code: 'not_implemented',
      message: `${command} is not available yet; planned for Phase 0 slice ${slice}`,
      command,
      plannedSlice: slice,
    };
    throw payload;
  },
);

export const isTauri = vi.fn(() => true);
