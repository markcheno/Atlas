/**
 * Full-text search state (phase1-spec §5.2). A search runs when the user
 * submits, not while typing, so the catalog only works on intent. While
 * text reading continues, the shown results refresh (throttled).
 */
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { FileType } from '../api/bindings/FileType';
import type { SearchPage } from '../api/bindings/SearchPage';
import type { SearchQuery } from '../api/bindings/SearchQuery';
import { hasWords } from '../searchCopy';

export const SEARCH_PAGE_SIZE = 20;
export const SEARCH_REFRESH_MS = 2000;

/** The Search screen's two modes (phase2-spec §5.3, §5.5). */
export type SearchMode = 'find' | 'ask';

class SearchState {
  /** Find sources (results) or Ask Atlas (an answer); filters are shared. */
  mode = $state<SearchMode>('find');
  /** What is in the box. */
  draft = $state('');
  /** What the shown results are for; empty before the first search. */
  text = $state('');
  sourceRootId = $state<string | null>(null);
  fileType = $state<FileType | null>(null);
  page = $state(0);

  result = $state<SearchPage | null>(null);
  loading = $state(false);
  error = $state<AtlasApiError | null>(null);

  #seq = 0;
  /** Filters changed in Ask mode: Find sources searches again on return. */
  #filtersMoved = false;
  #lastLive = 0;
  #liveTimer: ReturnType<typeof setTimeout> | null = null;

  /** Submitted, but with nothing to search for. */
  get noWords(): boolean {
    return this.text.trim() !== '' && !hasWords(this.text);
  }

  query(): SearchQuery {
    return {
      text: this.text,
      sourceRootId: this.sourceRootId,
      fileType: this.fileType,
      includeMissing: false,
      page: this.page,
      pageSize: SEARCH_PAGE_SIZE,
    };
  }

  async load() {
    const seq = ++this.#seq;
    if (this.text.trim() === '' || this.noWords) {
      this.result = null;
      this.error = null;
      this.loading = false;
      return;
    }
    this.loading = true;
    try {
      const result = await atlas.searchDocuments(this.query());
      if (seq === this.#seq) {
        this.result = result;
        this.error = null;
      }
    } catch (err) {
      if (seq === this.#seq) {
        this.error = err instanceof AtlasApiError ? err : toApiError('search_documents', err);
      }
    } finally {
      if (seq === this.#seq) this.loading = false;
    }
  }

  submit(text = this.draft) {
    this.draft = text;
    this.text = text.trim();
    this.page = 0;
    return this.load();
  }

  setFilters(change: Partial<Pick<SearchState, 'sourceRootId' | 'fileType'>>) {
    Object.assign(this, change);
    this.page = 0;
    if (this.mode === 'ask') {
      // The next question uses them; asking again is the user's choice.
      this.#filtersMoved = true;
      return Promise.resolve();
    }
    return this.load();
  }

  setMode(mode: SearchMode) {
    this.mode = mode;
    if (mode === 'find' && this.#filtersMoved) {
      this.#filtersMoved = false;
      if (this.result) return this.load();
    }
    return Promise.resolve();
  }

  goTo(page: number) {
    this.page = Math.max(0, page);
    return this.load();
  }

  /** On reading progress: reloads the shown results, at most every 2 s. */
  refreshLive() {
    if (!this.result || this.#liveTimer) return;
    const wait = SEARCH_REFRESH_MS - (Date.now() - this.#lastLive);
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
    this.#filtersMoved = false;
    this.mode = 'find';
    this.draft = '';
    this.text = '';
    this.sourceRootId = null;
    this.fileType = null;
    this.page = 0;
    this.result = null;
    this.loading = false;
    this.error = null;
  }
}

export const search = new SearchState();
