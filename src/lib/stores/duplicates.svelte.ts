/**
 * Library › Duplicates (Slice 1.6): groups of files with identical
 * content, most extra space first. Refreshes while cataloging or reading
 * makes progress (throttled), like the file list.
 */
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { DuplicatePage } from '../api/bindings/DuplicatePage';
import type { DuplicateQuery } from '../api/bindings/DuplicateQuery';
import type { FileType } from '../api/bindings/FileType';

export const DUPLICATES_PAGE_SIZE = 20;
const LIVE_REFRESH_MS = 2000;

class DuplicatesState {
  sourceRootId = $state<string | null>(null);
  fileType = $state<FileType | null>(null);
  includeMissing = $state(false);
  page = $state(0);

  result = $state<DuplicatePage | null>(null);
  loading = $state(false);
  error = $state<AtlasApiError | null>(null);

  #seq = 0;
  #lastLive = 0;
  #liveTimer: ReturnType<typeof setTimeout> | null = null;

  query(): DuplicateQuery {
    return {
      sourceRootId: this.sourceRootId,
      fileType: this.fileType,
      includeMissing: this.includeMissing,
      page: this.page,
      pageSize: DUPLICATES_PAGE_SIZE,
    };
  }

  async load() {
    const seq = ++this.#seq;
    this.loading = true;
    try {
      const result = await atlas.listDuplicates(this.query());
      if (seq === this.#seq) {
        this.result = result;
        this.error = null;
      }
    } catch (err) {
      if (seq === this.#seq) {
        this.error = err instanceof AtlasApiError ? err : toApiError('list_duplicates', err);
      }
    } finally {
      if (seq === this.#seq) this.loading = false;
    }
  }

  setFilters(
    change: Partial<Pick<DuplicatesState, 'sourceRootId' | 'fileType' | 'includeMissing'>>,
  ) {
    Object.assign(this, change);
    this.page = 0;
    return this.load();
  }

  goTo(page: number) {
    this.page = Math.max(0, page);
    return this.load();
  }

  refreshLive() {
    if (!this.result || this.#liveTimer) return;
    const wait = LIVE_REFRESH_MS - (Date.now() - this.#lastLive);
    this.#liveTimer = setTimeout(
      () => {
        this.#liveTimer = null;
        this.#lastLive = Date.now();
        void this.load();
      },
      Math.max(0, wait),
    );
  }

  reset() {
    this.#seq++;
    if (this.#liveTimer) clearTimeout(this.#liveTimer);
    this.#liveTimer = null;
    this.sourceRootId = null;
    this.fileType = null;
    this.includeMissing = false;
    this.page = 0;
    this.result = null;
    this.loading = false;
    this.error = null;
  }
}

export const duplicates = new DuplicatesState();
