/**
 * Library browsing state: query, current page, and per-row actions.
 * Searches are debounced; live refreshes during cataloging are throttled.
 */
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { DocumentPage } from '../api/bindings/DocumentPage';
import type { DocumentQuery } from '../api/bindings/DocumentQuery';
import type { DocumentSort } from '../api/bindings/DocumentSort';
import type { FileInstance } from '../api/bindings/FileInstance';
import type { FileType } from '../api/bindings/FileType';

export const PAGE_SIZE = 50;
export const SEARCH_DEBOUNCE_MS = 200;
export const LIVE_REFRESH_MS = 1000;

function asApiError(command: string, err: unknown): AtlasApiError {
  return err instanceof AtlasApiError ? err : toApiError(command, err);
}

class LibraryState {
  text = $state('');
  sourceRootId = $state<string | null>(null);
  fileType = $state<FileType | null>(null);
  includeMissing = $state(false);
  sort = $state<DocumentSort>('name_asc');
  page = $state(0);

  result = $state<DocumentPage | null>(null);
  loading = $state(false);
  error = $state<AtlasApiError | null>(null);
  /** A per-row problem from Open or Reveal. */
  rowError = $state<{ id: string; error: AtlasApiError } | null>(null);
  /** Row awaiting consent to download from the cloud. */
  confirmDownloadId = $state<string | null>(null);

  #seq = 0;
  #debounce: ReturnType<typeof setTimeout> | null = null;
  #lastLive = 0;
  #liveTimer: ReturnType<typeof setTimeout> | null = null;

  query(): DocumentQuery {
    return {
      text: this.text.trim() === '' ? null : this.text,
      sourceRootId: this.sourceRootId,
      fileType: this.fileType,
      sort: this.sort,
      page: this.page,
      pageSize: PAGE_SIZE,
      includeMissing: this.includeMissing,
    };
  }

  async load() {
    const seq = ++this.#seq;
    this.loading = true;
    try {
      const result = await atlas.listDocuments(this.query());
      if (seq === this.#seq) {
        this.result = result;
        this.error = null;
      }
    } catch (err) {
      if (seq === this.#seq) this.error = asApiError('list_documents', err);
    } finally {
      if (seq === this.#seq) this.loading = false;
    }
  }

  /** Updates the search text and reloads after a short pause in typing. */
  setText(text: string, immediate = false) {
    this.text = text;
    this.page = 0;
    if (this.#debounce) clearTimeout(this.#debounce);
    if (immediate) {
      void this.load();
    } else {
      this.#debounce = setTimeout(() => void this.load(), SEARCH_DEBOUNCE_MS);
    }
  }

  setFilters(change: Partial<Pick<LibraryState, 'sourceRootId' | 'fileType' | 'includeMissing'>>) {
    Object.assign(this, change);
    this.page = 0;
    void this.load();
  }

  toggleSort(column: 'name' | 'modified') {
    if (column === 'name') {
      this.sort = this.sort === 'name_asc' ? 'name_desc' : 'name_asc';
    } else {
      this.sort = this.sort === 'modified_desc' ? 'modified_asc' : 'modified_desc';
    }
    this.page = 0;
    void this.load();
  }

  goTo(page: number) {
    this.page = Math.max(0, page);
    void this.load();
  }

  /** Called on cataloging progress; reloads at most once per second. */
  refreshLive() {
    const wait = LIVE_REFRESH_MS - (Date.now() - this.#lastLive);
    if (this.#liveTimer) return;
    this.#liveTimer = setTimeout(
      () => {
        this.#liveTimer = null;
        this.#lastLive = Date.now();
        void this.load();
      },
      Math.max(0, wait),
    );
  }

  async open(item: FileInstance, allowDownload = false) {
    this.rowError = null;
    try {
      await atlas.openOriginal(item.id, allowDownload);
      this.confirmDownloadId = null;
    } catch (err) {
      const error = asApiError('open_original', err);
      if (error.code === 'file_cloud_only') {
        this.confirmDownloadId = item.id;
      } else {
        this.confirmDownloadId = null;
        this.rowError = { id: item.id, error };
      }
    }
  }

  async reveal(item: FileInstance) {
    this.rowError = null;
    try {
      await atlas.revealOriginal(item.id);
    } catch (err) {
      this.rowError = { id: item.id, error: asApiError('reveal_original', err) };
    }
  }

  reset() {
    this.text = '';
    this.sourceRootId = null;
    this.fileType = null;
    this.includeMissing = false;
    this.sort = 'name_asc';
    this.page = 0;
    this.result = null;
    this.error = null;
    this.rowError = null;
    this.confirmDownloadId = null;
  }
}

export const library = new LibraryState();
