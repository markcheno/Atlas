/**
 * How the user reached a source, for their trail (Slice 3.3). The Library
 * and the duplicates list pass none: opening from them is not a step.
 */
import type { OpenVia } from './api/bindings/OpenVia';

export const FROM_RESULT: OpenVia = { kind: 'result' };
export const FROM_RELATED: OpenVia = { kind: 'related' };

/** Source `n` of the answer on screen. */
export function fromCitation(answerId: string | null, n: number): OpenVia | undefined {
  return answerId ? { kind: 'citation', answerId, n } : undefined;
}
