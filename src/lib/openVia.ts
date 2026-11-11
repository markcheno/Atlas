/**
 * How the user reached a source, for their trail (Slice 3.3). The Library
 * and the duplicates list pass none: opening from them is not a step.
 */
import type { OpenVia } from './api/bindings/OpenVia';
import type { SaveTarget } from './api/bindings/SaveTarget';
import type { SourceUsed } from './api/bindings/SourceUsed';
import { answer } from './stores/answer.svelte';

export const FROM_RESULT: OpenVia = { kind: 'result' };
export const FROM_RELATED: OpenVia = { kind: 'related' };
export const FROM_TRAIL: OpenVia = { kind: 'trail' };
export const FROM_BOOKMARK: OpenVia = { kind: 'bookmark' };
export const FROM_SUGGESTION: OpenVia = { kind: 'suggestion' };

/** Source `n` of the answer on screen. */
export function fromCitation(answerId: string | null, n: number): OpenVia | undefined {
  return answerId ? { kind: 'citation', answerId, n } : undefined;
}

/** What bookmarking a source of the answer on screen keeps: as it was sent. */
export function citationTarget(s: SourceUsed): SaveTarget | undefined {
  return answer.answerId
    ? {
        kind: 'file',
        fileId: s.fileInstanceId,
        place: { kind: 'citation', answerId: answer.answerId, n: s.n },
      }
    : undefined;
}
