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
import type { DeviceCheck } from '../lib/api/bindings/DeviceCheck';
import type { DeviceSnapshot } from '../lib/api/bindings/DeviceSnapshot';
import type { DocumentPage } from '../lib/api/bindings/DocumentPage';
import type { FileInstance } from '../lib/api/bindings/FileInstance';
import type { ErrorPayload } from '../lib/api/bindings/ErrorPayload';
import type { InventoryOverview } from '../lib/api/bindings/InventoryOverview';
import type { InventoryStatus } from '../lib/api/bindings/InventoryStatus';
import type { AddSourceRootsResult } from '../lib/api/bindings/AddSourceRootsResult';
import type { Settings } from '../lib/api/bindings/Settings';
import type { SourceRoot } from '../lib/api/bindings/SourceRoot';

export const DEFAULT_ROOT = '/Users/test/Library/Application Support/com.altazlabs.atlas/Library';

export const fakeStatus: AppStatus = {
  appVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  buildProfile: 'debug',
  slice: '0.6',
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

export function sourceRoot(overrides: Partial<SourceRoot> = {}): SourceRoot {
  return {
    id: 'root-books',
    displayPath: '/Users/test/Books',
    status: 'available',
    containsDataRoot: false,
    nestedInRootId: null,
    createdAt: '2026-11-07T00:00:00Z',
    lastScanAt: null,
    ...overrides,
  };
}

export function deviceSnapshot(overrides: Partial<DeviceSnapshot> = {}): DeviceSnapshot {
  return {
    schemaVersion: 1,
    os: 'macos',
    arch: 'aarch64',
    logicalCores: 18,
    totalMemoryBytes: 64 * 2 ** 30,
    availableMemoryBytes: 30 * 2 ** 30,
    dataVolumeReachable: true,
    dataVolumeFreeBytes: 712_000_000_000,
    dataVolumeTotalBytes: 2_000_000_000_000,
    dataVolumeKind: 'local',
    requiredFreeBytes: 10_737_418_240,
    power: 'battery',
    batteryPresent: true,
    batteryPercent: 75,
    charging: false,
    memoryPressure: 'normal',
    thermal: 'nominal',
    sampledAt: '2026-11-07T00:00:00Z',
    ...overrides,
  };
}

export function deviceCheck(overrides: Partial<DeviceCheck> = {}): DeviceCheck {
  return {
    snapshot: deviceSnapshot(),
    assessment: { library: 'comfortable', localAi: 'likely', notes: ['on_battery'] },
    policy: {
      mode: 'slow',
      pauseReason: null,
      throttleReason: 'on_battery',
      maxMetadataOpsPerSecond: 20,
    },
    ...overrides,
  };
}

export function inventoryStatus(overrides: Partial<InventoryStatus> = {}): InventoryStatus {
  return {
    jobId: 'job-1',
    sourceRootId: 'root-books',
    sourceRootDisplayPath: '/Users/test/Books',
    state: 'running',
    pauseReason: null,
    throttleReason: null,
    discoveredFiles: 1204,
    completedDirectories: 37,
    failures: 0,
    skippedSymlinks: 2,
    skippedEntries: 14,
    elapsedMs: 83_000,
    startedAt: '2026-11-07T00:00:00Z',
    finishedAt: null,
    lastError: null,
    ...overrides,
  };
}

export function overview(overrides: Partial<InventoryOverview> = {}): InventoryOverview {
  return {
    active: null,
    queuedSessions: 0,
    catalogedFiles: 0,
    missingFiles: 0,
    cloudOnlyFiles: 0,
    roots: [],
    ...overrides,
  };
}

export function fileInstance(overrides: Partial<FileInstance> = {}): FileInstance {
  return {
    id: 'file-1',
    sourceRootId: 'root-books',
    displayName: 'book.pdf',
    displayPath: '/Users/test/Books/book.pdf',
    extension: 'pdf',
    fileType: 'pdf',
    sizeBytes: 1_200_000,
    modifiedAt: '2026-10-01T00:00:00Z',
    availability: 'available',
    storage: 'local',
    ...overrides,
  };
}

export function documentPage(overrides: Partial<DocumentPage> = {}): DocumentPage {
  return { items: [], page: 0, pageSize: 50, matchedSoFar: 0, scanning: false, ...overrides };
}

/** Folders the fake picker returns, in order, on each call. */
export const pickerQueue: AddSourceRootsResult[] = [];

export function apiError(code: ErrorPayload['code'], message: string, recovery?: string) {
  const payload: ErrorPayload = { code, message, recovery };
  return payload;
}

type Handler = (args: Record<string, unknown> | undefined) => unknown;

const plannedSlices: Record<string, string> = {
  list_provider_profiles: '0.7',
};

function defaults(): Record<string, Handler> {
  let settings: Settings = { appearance: { theme: 'system' } };
  let roots: SourceRoot[] = [];
  return {
    get_device_check: () => deviceCheck(),
    get_inventory_status: () => overview(),
    list_documents: () => documentPage(),
    open_original: () => null,
    reveal_original: () => null,
    start_inventory: () => 'job-2',
    pause_job: () => null,
    resume_job: () => null,
    cancel_job: () => null,
    list_source_roots: () => roots,
    pick_and_add_source_roots: () => {
      const result = pickerQueue.shift() ?? { added: [], rejected: [] };
      roots = [...roots, ...result.added];
      return result;
    },
    remove_source_root: (args) => {
      roots = roots.filter((r) => r.id !== args?.id);
      return null;
    },
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
  pickerQueue.length = 0;
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
