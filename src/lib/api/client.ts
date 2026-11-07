/**
 * Typed facade over Atlas's Tauri IPC commands.
 *
 * This is the frontend's only route to native functionality. DTO types are
 * generated from Rust by ts-rs (see ./bindings); do not hand-edit them.
 * Command names are checked against the Rust registry by
 * src-tauri/tests/ipc_contract.rs, so keep each one as a single-quoted
 * literal below.
 */
import { invoke, isTauri } from '@tauri-apps/api/core';
import type { AppStatus } from './bindings/AppStatus';
import type { ConnectionResult } from './bindings/ConnectionResult';
import type { ConnectionTestTarget } from './bindings/ConnectionTestTarget';
import type { DeviceSnapshot } from './bindings/DeviceSnapshot';
import type { DocumentPage } from './bindings/DocumentPage';
import type { DocumentQuery } from './bindings/DocumentQuery';
import type { ErrorCode } from './bindings/ErrorCode';
import type { ErrorPayload } from './bindings/ErrorPayload';
import type { InventoryStatus } from './bindings/InventoryStatus';
import type { ProviderProfile } from './bindings/ProviderProfile';
import type { ProviderProfileInput } from './bindings/ProviderProfileInput';
import type { SourceRoot } from './bindings/SourceRoot';

/** Server codes plus two the client itself can produce. */
export type ApiErrorCode = ErrorCode | 'ipc_unavailable' | 'unexpected';

export class AtlasApiError extends Error {
  readonly code: ApiErrorCode;
  readonly command: string;
  readonly plannedSlice?: string;
  readonly recovery?: string;

  constructor(
    code: ApiErrorCode,
    message: string,
    command: string,
    extra: { plannedSlice?: string; recovery?: string } = {},
  ) {
    super(message);
    this.name = 'AtlasApiError';
    this.code = code;
    this.command = command;
    this.plannedSlice = extra.plannedSlice;
    this.recovery = extra.recovery;
  }

  get isNotImplemented(): boolean {
    return this.code === 'not_implemented';
  }
}

function isErrorPayload(value: unknown): value is ErrorPayload {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { code?: unknown }).code === 'string' &&
    typeof (value as { message?: unknown }).message === 'string'
  );
}

export function toApiError(command: string, raw: unknown): AtlasApiError {
  if (isErrorPayload(raw)) {
    return new AtlasApiError(raw.code, raw.message, command, {
      plannedSlice: raw.plannedSlice,
      recovery: raw.recovery,
    });
  }
  return new AtlasApiError(
    'unexpected',
    'Atlas hit an unexpected problem talking to its core.',
    command,
  );
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    throw new AtlasApiError(
      'ipc_unavailable',
      'This view is running outside the Atlas desktop app, so native features are unavailable.',
      command,
      { recovery: 'Start Atlas with `pnpm tauri dev`.' },
    );
  }
  try {
    return await invoke<T>(command, args);
  } catch (raw) {
    throw toApiError(command, raw);
  }
}

export const atlas = {
  getAppStatus: () => call<AppStatus>('get_app_status'),
  getDeviceSnapshot: () => call<DeviceSnapshot>('get_device_snapshot'),

  chooseDataRoot: (path: string) => call<void>('choose_data_root', { path }),
  openDataRoot: (path: string) => call<void>('open_data_root', { path }),

  listSourceRoots: () => call<SourceRoot[]>('list_source_roots'),
  addSourceRoot: (path: string) => call<SourceRoot>('add_source_root', { path }),
  removeSourceRoot: (id: string) => call<void>('remove_source_root', { id }),

  startInventory: (sourceRootId: string) => call<string>('start_inventory', { sourceRootId }),
  pauseJob: (jobId: string) => call<void>('pause_job', { jobId }),
  resumeJob: (jobId: string) => call<void>('resume_job', { jobId }),
  cancelJob: (jobId: string) => call<void>('cancel_job', { jobId }),
  getInventoryStatus: () => call<InventoryStatus>('get_inventory_status'),

  listDocuments: (query: DocumentQuery) => call<DocumentPage>('list_documents', { query }),
  openOriginal: (fileId: string) => call<void>('open_original', { fileId }),
  revealOriginal: (fileId: string) => call<void>('reveal_original', { fileId }),

  listProviderProfiles: () => call<ProviderProfile[]>('list_provider_profiles'),
  saveProviderProfile: (profile: ProviderProfileInput) =>
    call<ProviderProfile>('save_provider_profile', { profile }),
  testProviderConnection: (target: ConnectionTestTarget) =>
    call<ConnectionResult>('test_provider_connection', { target }),
} as const;

export type AtlasApi = typeof atlas;
