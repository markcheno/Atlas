/**
 * Shared fake of the Tauri core module, mirroring the Rust contract.
 * Tests override individual commands with `setHandler` and call
 * `resetHandlers` in `beforeEach`. Unhandled commands reject with a
 * not_implemented ErrorPayload, like the real stubs.
 */
import { vi } from 'vitest';
import type { AiRole } from '../lib/api/bindings/AiRole';
import type { AppStatus } from '../lib/api/bindings/AppStatus';
import type { CapabilityResult } from '../lib/api/bindings/CapabilityResult';
import type { ModelRole } from '../lib/api/bindings/ModelRole';
import type { ModelRoles } from '../lib/api/bindings/ModelRoles';
import type { Recommendations } from '../lib/api/bindings/Recommendations';
import type { DataRootCandidate } from '../lib/api/bindings/DataRootCandidate';
import type { DataRootStatus } from '../lib/api/bindings/DataRootStatus';
import type { DeviceCheck } from '../lib/api/bindings/DeviceCheck';
import type { DeviceSnapshot } from '../lib/api/bindings/DeviceSnapshot';
import type { DocumentDetail } from '../lib/api/bindings/DocumentDetail';
import type { DocumentPage } from '../lib/api/bindings/DocumentPage';
import type { DuplicateGroup } from '../lib/api/bindings/DuplicateGroup';
import type { DuplicatePage } from '../lib/api/bindings/DuplicatePage';
import type { ExtractionCounts } from '../lib/api/bindings/ExtractionCounts';
import type { ExtractionOverview } from '../lib/api/bindings/ExtractionOverview';
import type { EmbeddingCounts } from '../lib/api/bindings/EmbeddingCounts';
import type { EmbeddingOverview } from '../lib/api/bindings/EmbeddingOverview';
import type { EmbeddingStatus } from '../lib/api/bindings/EmbeddingStatus';
import type { ExtractionStatus } from '../lib/api/bindings/ExtractionStatus';
import type { FileInstance } from '../lib/api/bindings/FileInstance';
import type { ErrorPayload } from '../lib/api/bindings/ErrorPayload';
import type { InventoryOverview } from '../lib/api/bindings/InventoryOverview';
import type { InventoryStatus } from '../lib/api/bindings/InventoryStatus';
import type { ConnectionResult } from '../lib/api/bindings/ConnectionResult';
import type { ProviderProfile } from '../lib/api/bindings/ProviderProfile';
import type { ProviderProfileInput } from '../lib/api/bindings/ProviderProfileInput';
import type { AddSourceRootsResult } from '../lib/api/bindings/AddSourceRootsResult';
import type { SearchHit } from '../lib/api/bindings/SearchHit';
import type { SearchPage } from '../lib/api/bindings/SearchPage';
import type { Settings } from '../lib/api/bindings/Settings';
import type { SourceRoot } from '../lib/api/bindings/SourceRoot';
import type { AskReply } from '../lib/api/bindings/AskReply';
import type { AskStarted } from '../lib/api/bindings/AskStarted';
import type { SourceUsed } from '../lib/api/bindings/SourceUsed';
import type { RelatedHit } from '../lib/api/bindings/RelatedHit';
import type { RelatedPage } from '../lib/api/bindings/RelatedPage';
import type { StorageUsage } from '../lib/api/bindings/StorageUsage';

export const DEFAULT_ROOT = '/Users/test/Library/Application Support/com.altazlabs.atlas/Library';

export const fakeStatus: AppStatus = {
  appVersion: '0.3.0',
  os: 'macos',
  arch: 'aarch64',
  buildProfile: 'debug',
  slice: '3.1',
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
    documentId: null,
    extraction: { state: 'pending', reason: null },
    ...overrides,
  };
}

export function extractionCounts(overrides: Partial<ExtractionCounts> = {}): ExtractionCounts {
  return {
    done: 0,
    pending: 0,
    needsOcr: 0,
    encrypted: 0,
    cloudOnly: 0,
    unreadable: 0,
    total: 0,
    updating: 0,
    ...overrides,
  };
}

export function extractionStatus(overrides: Partial<ExtractionStatus> = {}): ExtractionStatus {
  return {
    jobId: 'extract-1',
    sourceRootId: 'root-books',
    sourceRootDisplayPath: '/Users/test/Books',
    state: 'running',
    pauseReason: null,
    throttleReason: null,
    processedFiles: 0,
    pendingFiles: 0,
    currentFileId: null,
    lastError: null,
    ...overrides,
  };
}

export function extractionOverview(
  overrides: Partial<ExtractionOverview> = {},
): ExtractionOverview {
  return {
    active: null,
    queuedJobs: 0,
    counts: extractionCounts(),
    roots: [],
    scanning: false,
    ...overrides,
  };
}

export function embeddingCounts(overrides: Partial<EmbeddingCounts> = {}): EmbeddingCounts {
  return { indexed: 0, waiting: 0, failed: 0, total: 0, ...overrides };
}

export function embeddingStatus(overrides: Partial<EmbeddingStatus> = {}): EmbeddingStatus {
  return {
    jobId: 'embed-1',
    sourceRootId: 'root-books',
    sourceRootDisplayPath: '/Users/me/Books',
    state: 'running',
    pauseReason: null,
    throttleReason: null,
    lastError: null,
    embeddedChunks: 0,
    sentChars: 0,
    serviceMs: 0,
    ...overrides,
  };
}

export function embeddingOverview(overrides: Partial<EmbeddingOverview> = {}): EmbeddingOverview {
  return {
    active: null,
    queuedJobs: 0,
    counts: embeddingCounts(),
    model: null,
    rebuilding: false,
    needsModel: true,
    hasAiService: false,
    needsPermission: false,
    roots: [],
    reading: false,
    ...overrides,
  };
}

export function documentPage(overrides: Partial<DocumentPage> = {}): DocumentPage {
  return { items: [], page: 0, pageSize: 50, matchedSoFar: 0, scanning: false, ...overrides };
}

export function searchHit(overrides: Partial<SearchHit> = {}): SearchHit {
  return {
    documentId: 'doc-1',
    fileInstanceId: 'file-1',
    title: 'Harmonic Drives',
    titleSource: 'metadata',
    displayName: 'harmonic.pdf',
    displayPath: '/Users/test/Books/harmonic.pdf',
    fileType: 'pdf',
    sourceRootId: 'root-books',
    availability: 'available',
    storage: 'local',
    location: 'Page 12',
    pageNo: 12,
    snippet: [
      { text: '… a ', hit: false },
      { text: 'harmonic drive', hit: true },
      { text: ' has no backlash …', hit: false },
    ],
    why: 'text',
    allWords: true,
    otherLocations: 0,
    ...overrides,
  };
}

export function searchPage(overrides: Partial<SearchPage> = {}): SearchPage {
  return {
    hits: [],
    page: 0,
    pageSize: 20,
    matchedDocuments: 0,
    capped: false,
    readingText: false,
    textReadFiles: 10,
    totalFiles: 10,
    semantic: { state: 'off' },
    ...overrides,
  };
}

export function documentDetail(overrides: Partial<DocumentDetail> = {}): DocumentDetail {
  return {
    file: fileInstance({ extraction: { state: 'done', reason: null }, documentId: 'doc-1' }),
    documentId: 'doc-1',
    title: 'Harmonic Drives',
    titleSource: 'metadata',
    authors: ['Ada Lin'],
    language: 'en',
    pageCount: 312,
    charCount: 640_000,
    alsoAt: [],
    ...overrides,
  };
}

export function relatedHit(overrides: Partial<RelatedHit> = {}): RelatedHit {
  return {
    documentId: 'doc-2',
    fileInstanceId: 'file-2',
    title: 'Strain Wave Gearing',
    titleSource: 'metadata',
    displayName: 'strain-wave.pdf',
    displayPath: '/Users/test/Books/strain-wave.pdf',
    fileType: 'pdf',
    sourceRootId: 'root-books',
    availability: 'available',
    storage: 'local',
    location: 'Page 40',
    pageNo: 40,
    ...overrides,
  };
}

/** A passage sent to the model for an Ask Atlas answer. */
export function sourceUsed(overrides: Partial<SourceUsed> = {}): SourceUsed {
  return {
    n: 1,
    documentId: 'doc-1',
    fileInstanceId: 'file-1',
    title: 'Harmonic Drives',
    displayName: 'harmonic.pdf',
    fileType: 'pdf',
    location: 'Page 12',
    pageNo: 12,
    excerpt: 'A harmonic drive uses a flexible spline to reach high reduction ratios.',
    ...overrides,
  };
}

/** `ask_atlas`'s reply when the answer started: two sources by default. */
export function askStarted(overrides: Partial<AskStarted> = {}): AskReply {
  return {
    kind: 'started',
    answerId: 'answer-1',
    sources: [
      sourceUsed(),
      sourceUsed({
        n: 2,
        documentId: 'doc-2',
        fileInstanceId: 'file-2',
        title: 'Strain Wave Gearing',
        displayName: 'strain-wave.pdf',
        location: null,
        pageNo: null,
        excerpt: 'Strain wave gearing was invented in 1957.',
      }),
    ],
    weak: false,
    semantic: { state: 'ready', coveragePercent: 100 },
    ...overrides,
  };
}

export function relatedPage(overrides: Partial<RelatedPage> = {}): RelatedPage {
  return { status: 'off', hits: [], ...overrides };
}

export function storageUsage(overrides: Partial<StorageUsage> = {}): StorageUsage {
  return {
    catalogBytes: 1_400_000_000,
    textBytes: 620_000_000,
    indexBytes: 250_000_000,
    otherCatalogBytes: 530_000_000,
    cacheBytes: 12_000_000,
    dataVolumeFreeBytes: 245_000_000_000,
    estimate: {
      kind: 'range',
      remainingFiles: 17_600,
      lowBytes: 1_500_000_000,
      highBytes: 2_500_000_000,
    },
    meaningCatalogBytes: 0,
    meaningFileBytes: 0,
    meaningEstimate: null,
    measuredAt: '2026-11-09T00:00:00Z',
    ...overrides,
  };
}

export function duplicateGroup(overrides: Partial<DuplicateGroup> = {}): DuplicateGroup {
  return {
    documentId: 'doc-1',
    title: 'Harmonic Drives',
    titleSource: 'metadata',
    sizeBytes: 4_200_000,
    extraBytes: 8_400_000,
    copies: [
      fileInstance({
        id: 'c1',
        displayName: 'harmonic.pdf',
        displayPath: '/Users/test/Books/harmonic.pdf',
      }),
      fileInstance({
        id: 'c2',
        displayName: 'harmonic copy.pdf',
        displayPath: '/Users/test/Books/old/harmonic copy.pdf',
      }),
      fileInstance({
        id: 'c3',
        displayName: 'harmonic.pdf',
        displayPath: '/Users/test/Books/backup/harmonic.pdf',
      }),
    ],
    ...overrides,
  };
}

export function duplicatePage(overrides: Partial<DuplicatePage> = {}): DuplicatePage {
  return { groups: [], page: 0, pageSize: 20, totalGroups: 0, extraBytes: 0, ...overrides };
}

export function connectionResult(overrides: Partial<ConnectionResult> = {}): ConnectionResult {
  return {
    outcome: 'reachable',
    modelIds: ['qwen3-embedding:0.6b', 'qwen3.5:9b'],
    message: 'The service listed 2 models.',
    serverVersion: null,
    testedAt: '2026-11-07T00:00:00Z',
    ...overrides,
  };
}

export function capabilityResult(overrides: Partial<CapabilityResult> = {}): CapabilityResult {
  const role = overrides.role ?? 'embedding';
  return {
    role,
    modelId: role === 'embedding' ? 'qwen3-embedding:0.6b' : 'qwen3.5:9b',
    outcome: 'works',
    dimensions: role === 'embedding' ? 1024 : null,
    message: 'The model answered.',
    elapsedMs: 420,
    testedAt: '2026-11-09T12:00:00Z',
    ...overrides,
  };
}

export function recommendations(overrides: Partial<Recommendations> = {}): Recommendations {
  const steps = (runtime: 'omlx' | 'ollama' | 'lmstudio', modelId: string, verified: boolean) => ({
    runtime,
    modelId,
    command: runtime === 'ollama' ? `ollama pull ${modelId}` : null,
    steps: [`Install ${modelId}.`, 'Back in Atlas, choose it and press Test.'],
    verified,
  });
  return {
    catalogVersion: 1,
    totalMemoryGb: 16,
    embedding: {
      role: 'embedding',
      models: [
        {
          id: 'qwen3-embedding-0.6b',
          label: 'Qwen3-Embedding 0.6B',
          kind: 'recommended',
          sizeBytes: 639_000_000,
          minMemoryGb: 8,
          dimensions: 1024,
          why: 'Small and fast.',
          runtimes: [
            steps('omlx', 'mlx-community/Qwen3-Embedding-0.6B-8bit', false),
            steps('ollama', 'qwen3-embedding:0.6b', true),
            steps('lmstudio', 'text-embedding-qwen3-embedding-0.6b', false),
          ],
          installedAs: null,
        },
      ],
    },
    chat: {
      role: 'chat',
      models: [
        {
          id: 'qwen3.5-9b',
          label: 'Qwen3.5 9B',
          kind: 'recommended',
          sizeBytes: 6_600_000_000,
          minMemoryGb: 16,
          dimensions: null,
          why: 'Good answers on 16 GB.',
          runtimes: [
            steps('omlx', 'Qwen3.5-9B', false),
            steps('ollama', 'qwen3.5:9b', true),
            steps('lmstudio', 'qwen3.5-9b', false),
          ],
          installedAs: null,
        },
      ],
    },
    ...overrides,
  };
}

/** Folders the fake picker returns, in order, on each call. */
export const pickerQueue: AddSourceRootsResult[] = [];

export function apiError(code: ErrorPayload['code'], message: string, recovery?: string) {
  const payload: ErrorPayload = { code, message, recovery };
  return payload;
}

type Handler = (args: Record<string, unknown> | undefined) => unknown;

const plannedSlices: Record<string, string> = {};

function defaults(): Record<string, Handler> {
  let settings: Settings = {
    appearance: { theme: 'system' },
    privacy: { allowRemoteEmbedding: false, disableExternalAi: false, acknowledged: [] },
  };
  let roots: SourceRoot[] = [];
  let profiles: ProviderProfile[] = [];
  let modelRoles: ModelRoles = { chat: null, embedding: null };
  const withoutProfile = (id: unknown): ModelRoles => ({
    chat: modelRoles.chat?.profileId === id ? null : modelRoles.chat,
    embedding: modelRoles.embedding?.profileId === id ? null : modelRoles.embedding,
  });
  return {
    list_provider_profiles: () => profiles,
    save_provider_profile: (args) => {
      const input = args?.profile as ProviderProfileInput;
      const id = input.id ?? `profile-${profiles.length + 1}`;
      const previous = profiles.find((p) => p.id === id);
      const hasKey = input.clearApiKey
        ? false
        : Boolean(input.apiKey) || Boolean(previous?.hasCredential);
      const saved: ProviderProfile = {
        id,
        kind: input.kind,
        label: input.label || (input.kind === 'ollama' ? 'Ollama' : 'OpenAI-compatible service'),
        baseUrl: input.baseUrl.replace(/\/+$/, ''),
        hasCredential: hasKey,
        credentialStorage: hasKey ? 'keychain' : 'none',
        insecureHttpAcknowledged: input.insecureHttpAcknowledged,
        location: /\/\/(localhost|127\.)/.test(input.baseUrl)
          ? 'this_computer'
          : /\/\/(192\.168\.|10\.)/.test(input.baseUrl)
            ? 'another_computer'
            : 'internet',
        createdAt: 't',
        updatedAt: 't',
      };
      profiles = [...profiles.filter((p) => p.id !== id), saved];
      return saved;
    },
    delete_provider_profile: (args) => {
      profiles = profiles.filter((p) => p.id !== args?.id);
      modelRoles = withoutProfile(args?.id);
      return null;
    },
    test_model_capability: (args) =>
      capabilityResult({ role: args?.role as AiRole, modelId: String(args?.modelId ?? '') }),
    get_model_roles: () => modelRoles,
    set_model_role: (args) => {
      const role = args?.role as AiRole;
      const result = args?.result as CapabilityResult;
      const listed = (handlers.list_provider_profiles?.(undefined) ??
        profiles) as ProviderProfile[];
      const profile = listed.find((p) => p.id === args?.profileId);
      const assigned: ModelRole = {
        role,
        profileId: String(args?.profileId),
        profileLabel: profile?.label ?? 'Service',
        location: profile?.location ?? 'this_computer',
        modelId: String(args?.modelId),
        dimensions: result.dimensions,
        testedAt: result.testedAt,
      };
      modelRoles = { ...modelRoles, [role]: assigned };
      return modelRoles;
    },
    clear_model_role: (args) => {
      modelRoles = { ...modelRoles, [args?.role as AiRole]: null };
      return modelRoles;
    },
    get_model_recommendations: () => recommendations(),
    test_provider_connection: () => connectionResult(),
    detect_local_providers: () => [],
    get_device_check: () => deviceCheck(),
    get_inventory_status: () => overview(),
    get_extraction_status: () => extractionOverview(),
    start_extraction: () => 'extract-2',
    get_embedding_status: () => embeddingOverview(),
    start_embedding: () => 'embed-2',
    list_documents: () => documentPage(),
    search_documents: () => searchPage(),
    list_duplicates: () => duplicatePage(),
    get_related_documents: () => relatedPage(),
    get_storage_usage: () => storageUsage(),
    clean_caches: () => ({ freedBytes: 0, usage: storageUsage() }),
    get_document_detail: (args) =>
      documentDetail({ file: fileInstance({ id: String(args?.fileId) }) }),
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
    // Ask Atlas (Slice 2.6): nothing in the library unless a test says so.
    ask_atlas: (): AskReply => ({ kind: 'nothing_found', semantic: { state: 'off' } }),
    cancel_answer: () => null,
    acknowledge_disclosure: () => settings,
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
      message: `${command} is not available yet; planned for slice ${slice}`,
      command,
      plannedSlice: slice,
    };
    throw payload;
  },
);

export const isTauri = vi.fn(() => true);
