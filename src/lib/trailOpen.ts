/**
 * Opening what a trail recorded (Slice 3.4): an anchor resolved against the
 * catalog now. A source that is still there opens in the details panel,
 * at its place now, recorded as reached from a trail; a question's source
 * shows the excerpt that was sent as the cited passage.
 */
import type { Anchor } from './api/bindings/Anchor';
import type { OpenVia } from './api/bindings/OpenVia';
import type { SaveTarget } from './api/bindings/SaveTarget';
import type { ResolvedAnchor } from './api/bindings/ResolvedAnchor';
import type { SourceUsed } from './api/bindings/SourceUsed';
import { FROM_TRAIL } from './openVia';
import { details } from './stores/details.svelte';
import { citedOn } from './trailCopy';

/** The file to open now, if there is one. */
export function openableFile(r: ResolvedAnchor | undefined): string | null {
  if (!r || (r.state !== 'found' && r.state !== 'found_elsewhere')) return null;
  return r.fileInstanceId;
}

/** A recorded source as the panel's cited passage. */
function asCited(n: number, a: Anchor, r: ResolvedAnchor, fileId: string): SourceUsed {
  return {
    n,
    documentId: r.documentId ?? a.documentId,
    fileInstanceId: fileId,
    title: a.title,
    displayName: a.title,
    fileType: a.fileType,
    location: r.location ?? a.location ?? null,
    pageNo: r.pageNo ?? a.pageNo ?? null,
    excerpt: a.excerpt ?? '',
  };
}

/**
 * Opens a recorded or saved anchor's document in the details panel. A
 * question's source (`n`, with its excerpt) shows that excerpt as the
 * cited passage; `saveAs` is what the panel's Bookmark keeps. False when
 * there is nothing to open.
 */
export function openRecorded(
  a: Anchor,
  r: ResolvedAnchor | undefined,
  trigger: HTMLElement | null,
  options: { question?: { n: number; at: string }; via?: OpenVia; saveAs?: SaveTarget } = {},
): boolean {
  const fileId = openableFile(r);
  if (!fileId || !r) return false;
  const { question } = options;
  const cited = question && a.excerpt ? asCited(question.n, a, r, fileId) : undefined;
  void details.open(fileId, trigger, {
    via: options.via ?? FROM_TRAIL,
    cited,
    citedNote: cited && question ? citedOn(question.at) : undefined,
    saveAs: options.saveAs,
  });
  return true;
}
