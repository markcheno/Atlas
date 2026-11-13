/**
 * Plain-language meaning-index status (Slice 2.3, phase2-spec §5.2): the
 * status bar's third line, the per-folder line and Home's note. Counts are
 * files with text; files without text are never waiting for meaning.
 */
import type { EmbeddingOverview } from './api/bindings/EmbeddingOverview';
import type { PauseReason } from './api/bindings/PauseReason';
import { count } from './inventoryCopy';

/** What the status line offers besides Pause, Resume and Cancel. */
export type MeaningAction = 'retry' | 'choose_model' | 'allow_remote' | 'index_now' | null;

export interface MeaningLine {
  text: string;
  action: MeaningAction;
}

function pausedText(reason: PauseReason | null): string {
  switch (reason) {
    case 'on_battery':
      return 'Paused indexing meaning on battery · Resumes when plugged in';
    case 'battery_low':
      return 'Paused indexing meaning · Resumes at 25% or when plugged in';
    case 'low_disk_space':
      return 'Paused indexing meaning · Not enough free space';
    case 'data_root_missing':
      return "Paused indexing meaning · The library location isn't available";
    case 'memory_pressure':
      return 'Paused indexing meaning · The computer is low on memory';
    case 'thermal_pressure':
      return 'Paused indexing meaning · The computer is too hot';
    default:
      return 'Paused indexing meaning';
  }
}

const ERRORS: Record<string, string> = {
  provider_unreachable: "Can't index meaning · AI service not reachable",
  provider_auth: "Can't index meaning · The AI service refused the key",
  model_not_found: "Can't index meaning · The AI service no longer has the model",
  model_changed: "Can't index meaning · The model changed; test it again in Settings",
  address_rejected: "Can't index meaning · The AI service's address isn't allowed",
  index_unavailable: "Can't index meaning · Atlas couldn't open its meaning index",
};

/** Failures only Settings can mend: the key, the model or the address. */
const MENDED_IN_SETTINGS = [
  'provider_auth',
  'model_not_found',
  'model_changed',
  'address_rejected',
];

/**
 * Folders whose last meaning job stopped for good with files still
 * waiting (Slice 4.1.1). Such a job isn't active, so without this the
 * line read "Meaning waiting" and gave no reason.
 */
export function stoppedFolders(ov: EmbeddingOverview | null): string[] {
  return (ov?.roots ?? [])
    .filter((r) => r.failedBecause !== null && r.counts.waiting > 0)
    .map((r) => r.sourceRootId);
}

/** The status bar's third line, or null when there's nothing to say. */
export function meaningLine(ov: EmbeddingOverview | null): MeaningLine | null {
  if (!ov) return null;
  if (ov.needsModel) {
    // Atlas works without AI: only someone who added an AI service hears
    // that it needs a model, and only once there is text to index.
    if (!ov.hasAiService || ov.counts.total === 0) return null;
    return { text: 'Indexing meaning needs an embedding model', action: 'choose_model' };
  }
  if (ov.needsPermission) {
    return { text: 'Indexing meaning needs your permission', action: 'allow_remote' };
  }
  if (ov.counts.total === 0) return null;
  const done = ov.counts.indexed;
  const progress = ov.reading
    ? `${count(done, 'file')} so far`
    : `${done.toLocaleString('en-US')} of ${count(ov.counts.total, 'file')}`;
  const doing = ov.rebuilding ? 'Rebuilding meaning index' : 'Indexing meaning';
  const after = ov.rebuilding ? ' · search uses the previous index' : '';
  const a = ov.active;
  if (!a) {
    const stopped = ov.roots.find((r) => r.failedBecause !== null && r.counts.waiting > 0);
    if (stopped) {
      const code = stopped.failedBecause ?? '';
      return {
        text: `${ERRORS[code] ?? "Can't index meaning"} · ${progress} indexed`,
        action: MENDED_IN_SETTINGS.includes(code) ? 'choose_model' : 'index_now',
      };
    }
    if (ov.counts.waiting > 0)
      return { text: `Meaning waiting · ${progress} indexed`, action: null };
    return {
      text: `Meaning up to date · ${count(ov.counts.indexed, 'file')}`,
      action: null,
    };
  }
  switch (a.state) {
    case 'running':
      if (a.throttleReason)
        return { text: `${doing} · ${progress} · Going slowly${after}`, action: null };
      return { text: `${doing} · ${progress}${after}`, action: null };
    case 'queued':
      return { text: `Waiting to index meaning · ${progress}`, action: null };
    case 'paused_policy':
      return { text: pausedText(a.pauseReason), action: null };
    case 'paused_user':
      return { text: `Paused indexing meaning by you · ${progress}`, action: null };
    case 'failed_retryable':
      return {
        text: ERRORS[a.lastError ?? ''] ?? "Can't index meaning · AI service not reachable",
        action: 'retry',
      };
    default:
      return { text: `${doing} · ${progress}`, action: null };
  }
}

/** The last job's lasting failure, for Settings and the folder line. */
export function meaningFailure(code: string | null | undefined): string | null {
  return code ? (ERRORS[code] ?? null) : null;
}

/** A short meaning summary for a folder line, or null with no model. */
export function rootMeaningLine(ov: EmbeddingOverview | null, sourceRootId: string): string | null {
  if (!ov?.model) return null;
  const root = ov.roots.find((r) => r.sourceRootId === sourceRootId);
  if (!root || root.counts.total === 0) return null;
  if (root.activeJobId) {
    const job = ov.active?.jobId === root.activeJobId ? ov.active : null;
    return job?.state.startsWith('paused') ? 'Indexing meaning paused' : 'Indexing meaning…';
  }
  const c = root.counts;
  const failure = c.waiting > 0 ? meaningFailure(root.failedBecause) : null;
  if (failure) return failure;
  const parts = [`Meaning: ${c.indexed.toLocaleString('en-US')} indexed`];
  if (c.waiting) parts.push(`${c.waiting.toLocaleString('en-US')} waiting`);
  if (c.failed) parts.push(`${c.failed.toLocaleString('en-US')} refused by the model`);
  return parts.join(' · ');
}

/** Home's note while meaning indexing is going on; null otherwise. */
export function homeMeaningNote(ov: EmbeddingOverview | null): string | null {
  if (!ov?.active || ov.counts.waiting === 0 || !ov.model) return null;
  return `Atlas is still indexing meaning: ${ov.counts.indexed.toLocaleString('en-US')} of ${count(ov.counts.total, 'file')}. Meaning search covers what's been indexed.`;
}
