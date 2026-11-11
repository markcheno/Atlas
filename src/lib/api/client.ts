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
import type { AddSourceRootsResult } from './bindings/AddSourceRootsResult';
import type { AiRole } from './bindings/AiRole';
import type { CapabilityResult } from './bindings/CapabilityResult';
import type { ModelRoles } from './bindings/ModelRoles';
import type { Recommendations } from './bindings/Recommendations';
import type { RelatedPage } from './bindings/RelatedPage';
import type { AppStatus } from './bindings/AppStatus';
import type { ConnectionResult } from './bindings/ConnectionResult';
import type { ConnectionTestTarget } from './bindings/ConnectionTestTarget';
import type { DataRootCandidate } from './bindings/DataRootCandidate';
import type { DeviceCheck } from './bindings/DeviceCheck';
import type { DataRootStatus } from './bindings/DataRootStatus';
import type { DeviceSnapshot } from './bindings/DeviceSnapshot';
import type { DocumentDetail } from './bindings/DocumentDetail';
import type { CleanCachesResult } from './bindings/CleanCachesResult';
import type { DuplicatePage } from './bindings/DuplicatePage';
import type { DuplicateQuery } from './bindings/DuplicateQuery';
import type { StorageUsage } from './bindings/StorageUsage';
import type { KnowledgeStatus } from './bindings/KnowledgeStatus';
import type { DocumentPage } from './bindings/DocumentPage';
import type { ExtractionOverview } from './bindings/ExtractionOverview';
import type { EmbeddingOverview } from './bindings/EmbeddingOverview';
import type { DocumentQuery } from './bindings/DocumentQuery';
import type { ErrorCode } from './bindings/ErrorCode';
import type { ErrorPayload } from './bindings/ErrorPayload';
import type { InventoryOverview } from './bindings/InventoryOverview';
import type { LocalProviderSuggestion } from './bindings/LocalProviderSuggestion';
import type { ProviderProfile } from './bindings/ProviderProfile';
import type { ProviderProfileInput } from './bindings/ProviderProfileInput';
import type { AskQuery } from './bindings/AskQuery';
import type { AskReply } from './bindings/AskReply';
import type { Settings } from './bindings/Settings';
import type { SourceRoot } from './bindings/SourceRoot';
import type { SearchPage } from './bindings/SearchPage';
import type { SearchQuery } from './bindings/SearchQuery';
import type { SearchIntent } from './bindings/SearchIntent';
import type { OpenAction } from './bindings/OpenAction';
import type { OpenPlace } from './bindings/OpenPlace';
import type { OpenVia } from './bindings/OpenVia';
import type { RecordingStatus } from './bindings/RecordingStatus';
import type { TrailDetail } from './bindings/TrailDetail';
import type { TrailPage } from './bindings/TrailPage';
import type { TrailQuery } from './bindings/TrailQuery';
import type { TrailSummary } from './bindings/TrailSummary';

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
  getDeviceCheck: () => call<DeviceCheck>('get_device_check'),

  getDataRootStatus: () => call<DataRootStatus>('get_data_root_status'),
  getDefaultDataRoot: () => call<string>('get_default_data_root'),
  /** Native folder picker; resolves to null when the user cancels. */
  pickFolder: () => call<string | null>('pick_folder'),
  inspectDataRoot: (path: string) => call<DataRootCandidate>('inspect_data_root', { path }),
  chooseDataRoot: (path: string) => call<DataRootStatus>('choose_data_root', { path }),
  openDataRoot: (path: string) => call<DataRootStatus>('open_data_root', { path }),

  getSettings: () => call<Settings>('get_settings'),
  updateSettings: (settings: Settings) => call<Settings>('update_settings', { settings }),

  listSourceRoots: () => call<SourceRoot[]>('list_source_roots'),
  addSourceRoot: (path: string) => call<SourceRoot>('add_source_root', { path }),
  removeSourceRoot: (id: string) => call<void>('remove_source_root', { id }),
  /** Native multi-folder picker; adds each chosen folder. Empty when cancelled. */
  pickAndAddSourceRoots: () => call<AddSourceRootsResult>('pick_and_add_source_roots'),

  startInventory: (sourceRootId: string) => call<string>('start_inventory', { sourceRootId }),
  pauseJob: (jobId: string) => call<void>('pause_job', { jobId }),
  resumeJob: (jobId: string) => call<void>('resume_job', { jobId }),
  cancelJob: (jobId: string) => call<void>('cancel_job', { jobId }),
  getInventoryStatus: () => call<InventoryOverview>('get_inventory_status'),
  startExtraction: (sourceRootId: string) => call<string>('start_extraction', { sourceRootId }),
  getExtractionStatus: () => call<ExtractionOverview>('get_extraction_status'),
  startEmbedding: (sourceRootId: string) => call<string>('start_embedding', { sourceRootId }),
  getEmbeddingStatus: () => call<EmbeddingOverview>('get_embedding_status'),
  /**
   * `intent` says why it ran (Slice 3.3): only `submit` (the user asked)
   * is recorded into the trail; paging, refreshes and filter changes are
   * `refresh`.
   */
  searchDocuments: (query: SearchQuery, intent: SearchIntent = 'refresh') =>
    call<SearchPage>('search_documents', { query, intent }),
  getDocumentDetail: (fileId: string) => call<DocumentDetail>('get_document_detail', { fileId }),
  listDuplicates: (query: DuplicateQuery) => call<DuplicatePage>('list_duplicates', { query }),
  getRelatedDocuments: (documentId: string, limit = 10) =>
    call<RelatedPage>('get_related_documents', { documentId, limit }),
  getStorageUsage: () => call<StorageUsage>('get_storage_usage'),
  cleanCaches: () => call<CleanCachesResult>('clean_caches'),

  listDocuments: (query: DocumentQuery) => call<DocumentPage>('list_documents', { query }),
  /** `allowDownload` must be true to open a file stored only in the cloud. */
  openOriginal: (fileId: string, allowDownload = false) =>
    call<void>('open_original', { fileId, allowDownload }),
  revealOriginal: (fileId: string) => call<void>('reveal_original', { fileId }),

  listProviderProfiles: () => call<ProviderProfile[]>('list_provider_profiles'),
  saveProviderProfile: (profile: ProviderProfileInput) =>
    call<ProviderProfile>('save_provider_profile', { profile }),
  deleteProviderProfile: (id: string) => call<void>('delete_provider_profile', { id }),
  /** Asks the service for its model list only; nothing from the library is sent. */
  testProviderConnection: (target: ConnectionTestTarget) =>
    call<ConnectionResult>('test_provider_connection', { target }),
  /** Checks well-known ports on this computer; only when the user asks. */
  detectLocalProviders: () => call<LocalProviderSuggestion[]>('detect_local_providers'),
  /** Sends one fixed request to the model; nothing from the library. */
  testModelCapability: (target: ConnectionTestTarget, role: AiRole, modelId: string) =>
    call<CapabilityResult>('test_model_capability', { target, role, modelId }),
  getModelRoles: () => call<ModelRoles>('get_model_roles'),
  /** `result` must be a recent passing test of that model for that role. */
  setModelRole: (role: AiRole, profileId: string, modelId: string, result: CapabilityResult) =>
    call<ModelRoles>('set_model_role', { role, profileId, modelId, result }),
  clearModelRole: (role: AiRole) => call<ModelRoles>('clear_model_role', { role }),
  /** From the bundled catalog and this computer's memory; no network. */
  getModelRecommendations: (listedModelIds: string[]) =>
    call<Recommendations>('get_model_recommendations', { listedModelIds }),

  /**
   * Answers from the library (Slice 2.6): the sources and an answer id
   * whose `answer-stream` events follow, or why not yet. Sends only the
   * passages found, to the chat model, after this explicit action.
   */
  askAtlas: (query: AskQuery) => call<AskReply>('ask_atlas', { query }),
  cancelAnswer: (answerId: string) => call<void>('cancel_answer', { answerId }),
  /** The user saw what Ask Atlas sends to this internet service and agreed. */
  acknowledgeDisclosure: (profileId: string) =>
    call<Settings>('acknowledge_disclosure', { profileId }),
  /** The knowledge store's state, size and snapshots (Slice 3.2). */
  getKnowledgeStatus: () => call<KnowledgeStatus>('get_knowledge_status'),

  /**
   * A source the user opened (Slice 3.3). Only ids and how it was reached
   * are sent; Rust builds the step. From the Library it is not a step.
   */
  recordOpen: (fileId: string, via: OpenVia, action: OpenAction, place?: OpenPlace) =>
    call<void>('record_open', { fileId, via, action, place: place ?? null }),
  getRecordingStatus: () => call<RecordingStatus>('get_recording_status'),
  dismissTrailNote: () => call<RecordingStatus>('dismiss_trail_note'),
  listTrails: (query: TrailQuery) => call<TrailPage>('list_trails', { query }),
  /** A trail's steps with their anchors resolved now (Slice 3.3). */
  getTrail: (trailId: string) => call<TrailDetail>('get_trail', { trailId }),
  /** "New trail": the next step starts one. */
  startTrail: () => call<RecordingStatus>('start_trail'),
  continueTrail: (trailId: string) => call<TrailSummary>('continue_trail', { trailId }),
  renameTrail: (trailId: string, title: string) =>
    call<TrailSummary>('rename_trail', { trailId, title }),
  pinTrail: (trailId: string, pinned: boolean) =>
    call<TrailSummary>('pin_trail', { trailId, pinned }),
  deleteTrail: (trailId: string) => call<void>('delete_trail', { trailId }),
  deleteTrailStep: (stepId: string) => call<void>('delete_trail_step', { stepId }),
  addTrailNote: (trailId: string, text: string, stepId?: string) =>
    call<void>('add_trail_note', { trailId, stepId: stepId ?? null, text }),
  /** Every trail; bookmarks and findings are kept. The number deleted. */
  deleteAllTrails: () => call<number>('delete_all_trails'),
} as const;

export type AtlasApi = typeof atlas;
