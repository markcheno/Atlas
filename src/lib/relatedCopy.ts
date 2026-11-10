/**
 * Words for related documents (phase2-spec §5.4, Slice 2.5).
 */
import type { RelatedStatus } from './api/bindings/RelatedStatus';

export const NOT_INDEXED = 'Not indexed for meaning yet';

/** The line under a related document's name. */
export function relatedBy(location: string | null): string {
  return location ? `Related by meaning: ${location}` : 'Related by meaning';
}

/** What the Related section says instead of a list, or null for none. */
export function relatedNote(status: RelatedStatus, found: number): string | null {
  switch (status) {
    case 'off':
      return null;
    case 'ready':
      return found === 0 ? 'Nothing else in your library is close in meaning.' : null;
    case 'not_indexed':
      return `${NOT_INDEXED}.`;
    case 'no_text':
      return 'Atlas found no text in this file to compare.';
    case 'loading':
      return 'Finding related documents…';
    case 'failed':
      return "Atlas couldn't find related documents right now.";
  }
}

/** Whether the section should ask again when the meaning index moves on. */
export function relatedPending(status: RelatedStatus): boolean {
  return status === 'not_indexed' || status === 'loading';
}
