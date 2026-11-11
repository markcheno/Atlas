/**
 * Opening and revealing original files, shared by the Library, Search and
 * the detail panel. A cloud-only file asks before it is downloaded; errors
 * are shown next to the file they belong to.
 */
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { Availability } from '../api/bindings/Availability';
import type { FileInstance } from '../api/bindings/FileInstance';
import type { OpenVia } from '../api/bindings/OpenVia';
import type { SearchHit } from '../api/bindings/SearchHit';
import { recordOpen } from './trails.svelte';

/** What the actions need to know about a file. */
export interface Original {
  id: string;
  displayName: string;
  availability: Availability;
  sizeBytes?: number | null;
  /** How the user got here, for their trail; none from the Library. */
  via?: OpenVia;
}

export const fromFile = (f: FileInstance, via?: OpenVia): Original => ({
  id: f.id,
  displayName: f.displayName,
  availability: f.availability,
  sizeBytes: f.sizeBytes,
  via,
});

/** A search result or a related document. */
export const fromHit = (
  h: Pick<SearchHit, 'fileInstanceId' | 'displayName' | 'availability'>,
  via?: OpenVia,
): Original => ({
  id: h.fileInstanceId,
  displayName: h.displayName,
  availability: h.availability,
  via,
});

const asApiError = (command: string, err: unknown) =>
  err instanceof AtlasApiError ? err : toApiError(command, err);

class OriginalsState {
  /** The file waiting for "Download and open". */
  confirmId = $state<string | null>(null);
  problem = $state<{ id: string; error: AtlasApiError } | null>(null);

  async open(item: Original, allowDownload = false) {
    this.problem = null;
    try {
      await atlas.openOriginal(item.id, allowDownload);
      this.confirmId = null;
      recordOpen(item.id, item.via, 'open');
    } catch (err) {
      const error = asApiError('open_original', err);
      if (error.code === 'file_cloud_only') {
        this.confirmId = item.id;
      } else {
        this.confirmId = null;
        this.problem = { id: item.id, error };
      }
    }
  }

  async reveal(item: Original) {
    this.problem = null;
    try {
      await atlas.revealOriginal(item.id);
      recordOpen(item.id, item.via, 'reveal');
    } catch (err) {
      this.problem = { id: item.id, error: asApiError('reveal_original', err) };
    }
  }

  cancel() {
    this.confirmId = null;
  }

  /** Whether something should show under this file. */
  hasNotice(id: string): boolean {
    return this.confirmId === id || this.problem?.id === id;
  }

  clear() {
    this.confirmId = null;
    this.problem = null;
  }
}

export const originals = new OriginalsState();
