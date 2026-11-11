/** Wording for interests, projects and suggestions (Slice 3.6, phase3-spec §5.4, §5.6). */
import type { CollectionCounts } from './api/bindings/CollectionCounts';
import type { CollectionKind } from './api/bindings/CollectionKind';
import type { CollectionStatus } from './api/bindings/CollectionStatus';
import type { SuggestionReason } from './api/bindings/SuggestionReason';
import { count } from './inventoryCopy';

export const COLLECTIONS_TAB = 'Interests and projects';
export const COLLECTIONS_HOME = 'Interests and projects';
export const ALL_COLLECTIONS = 'All interests and projects';
export const FROM_LIBRARY = 'From your library';
export const NOT_FOR_THIS = 'Not for this';
export const NEW_INTEREST = 'New interest…';
export const NEW_PROJECT = 'New project…';
export const ADD_TO = 'Add to…';
export const COLLECTIONS_EMPTY =
  'Group trails, bookmarks, findings and documents under an interest or a project. Nothing needs one.';

export function kindLabel(kind: CollectionKind): string {
  return kind === 'project' ? 'Project' : 'Interest';
}

export function statusLabel(status: CollectionStatus): string {
  return status === 'finished' ? 'Finished' : status === 'archived' ? 'Archived' : 'Active';
}

/** "2 trails · 1 finding", or "Nothing yet". */
export function countsText(c: CollectionCounts): string {
  const parts = [
    c.trails && count(c.trails, 'trail'),
    c.findings && count(c.findings, 'finding'),
    c.bookmarks && count(c.bookmarks, 'bookmark'),
    c.documents && count(c.documents, 'document'),
  ].filter(Boolean);
  return parts.length ? parts.join(' · ') : 'Nothing yet';
}

/** "Related to your interest ‘Stoicism’". */
export function reasonText(r: SuggestionReason): string {
  switch (r.seed) {
    case 'interest':
      return `Related to your interest ‘${r.name}’`;
    case 'project':
      return `Related to your project ‘${r.name}’`;
    case 'recent':
      return 'Related to your recent trails';
  }
}

export function removeFrom(kind: CollectionKind): string {
  return kind === 'project' ? 'Remove from this project' : 'Remove from this interest';
}

export function deleteCollectionQuestion(kind: CollectionKind, name: string): string {
  return `Delete the ${kind} “${name}”? Its trails, bookmarks and findings are kept.`;
}

export const DOCUMENT_NOT_READ = "Atlas hasn't read this document yet.";
export const DOCUMENT_GONE = "This document isn't in your library any more.";
