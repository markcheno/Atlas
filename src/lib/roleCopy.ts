/** Wording for model roles, capability tests and recommendations (Slice 2.1). */
import type { AiRole } from './api/bindings/AiRole';
import type { AiRuntime } from './api/bindings/AiRuntime';
import type { CapabilityResult } from './api/bindings/CapabilityResult';
import type { RecommendationKind } from './api/bindings/RecommendationKind';
import { formatBytes } from './format';

export const ROLES: readonly AiRole[] = ['embedding', 'chat'];

export function roleTitle(role: AiRole): string {
  return role === 'embedding' ? 'Embeddings' : 'Chat';
}

export function roleWhat(role: AiRole): string {
  return role === 'embedding'
    ? 'Finds books and papers by meaning, and related documents.'
    : 'Answers questions from your library, with sources.';
}

export const TESTING_NOTE = 'Checking the model. Loading it can take a minute the first time…';

function answeredIn(ms: number): string {
  return ms < 100
    ? 'under 0.1 s'
    : `${(ms / 1000).toLocaleString('en-US', { maximumFractionDigits: 1 })} s`;
}

export function capabilityText(r: CapabilityResult): string {
  const seconds = answeredIn(r.elapsedMs);
  switch (r.outcome) {
    case 'works':
      return r.role === 'embedding'
        ? `Works for embeddings · ${(r.dimensions ?? 0).toLocaleString('en-US')} dimensions · answered in ${seconds}`
        : `Works for chat · answered in ${seconds}`;
    case 'needs_model_id':
      return 'Choose or type a model first.';
    case 'model_not_found':
      return `The service doesn't have a model called ${r.modelId}. Check the name, or install it first.`;
    case 'wrong_role':
      return r.role === 'embedding'
        ? "This model can't make embeddings. Choose an embedding model."
        : "This model can't chat. Choose a chat model.";
    case 'authentication_failed':
      return 'The service refused the key. Check it in the service settings above.';
    case 'protocol_mismatch':
      return 'The service answered, but not the way Atlas expected. It may not support this kind of model.';
    case 'network_error':
      return 'Nothing answered at this address. Is the service running?';
    case 'timeout':
      return 'The service took too long to answer.';
    case 'insecure_url_rejected':
      return 'Unencrypted HTTP to another computer needs your confirmation first.';
  }
}

export function runtimeLabel(runtime: AiRuntime): string {
  switch (runtime) {
    case 'omlx':
      return 'oMLX';
    case 'ollama':
      return 'Ollama';
    case 'lmstudio':
      return 'LM Studio';
  }
}

export function kindLabelOf(kind: RecommendationKind): string {
  switch (kind) {
    case 'recommended':
      return 'Recommended';
    case 'faster':
      return 'Faster';
    case 'higher_quality':
      return 'Higher quality';
  }
}

export function sizeText(bytes: number): string {
  return `${formatBytes(bytes)} download`;
}

export const NEVER_DOWNLOADS = 'Atlas never downloads models itself.';

export function clearQuestion(role: AiRole): string {
  return `Stop using this model for ${role === 'embedding' ? 'embeddings' : 'chat'}?`;
}

export function clearedText(role: AiRole): string {
  return role === 'embedding'
    ? 'Atlas no longer uses a model for embeddings.'
    : 'Atlas no longer uses a model for chat.';
}

/** "9 Nov" from an RFC 3339 time. */
export function testedOn(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? ''
    : d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' });
}

/** Before Use replaces an embedding model that has an index (Slice 2.3). */
export const NEW_INDEX_NOTE =
  'Atlas will build a new meaning index with this model. The current one keeps working until the new one is ready.';

/** While Use is saved for an embedding model. */
export const PREPARING_NOTE =
  'Saving… The first time, Atlas may prepare your library for meaning search, which can take a minute.';
