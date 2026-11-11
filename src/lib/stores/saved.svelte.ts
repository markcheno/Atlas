/**
 * Bookmarks and saved findings (Slice 3.5, phase3-spec §5.3, §7.3): saving
 * from wherever a place, a passage or an answer is shown, and the Saved
 * tab's list. The renderer names what to keep; Rust builds it.
 */
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { Bookmark } from '../api/bindings/Bookmark';
import type { FindingSource } from '../api/bindings/FindingSource';
import type { SaveTarget } from '../api/bindings/SaveTarget';
import type { SavedKind } from '../api/bindings/SavedKind';
import type { SavedPage } from '../api/bindings/SavedPage';
import { BOOKMARKED, SAVED_FINDING } from '../savedCopy';

export const SAVED_PAGE_SIZE = 25;
const FILTER_DEBOUNCE_MS = 200;

const asApiError = (command: string, err: unknown) =>
  err instanceof AtlasApiError ? err : toApiError(command, err);

/** What a Save button shows after it was pressed. */
export interface SaveState {
  done?: string;
  error?: AtlasApiError;
}

class SavedState {
  // The Saved tab.
  kind = $state<SavedKind | null>(null);
  text = $state('');
  page = $state(0);
  result = $state<SavedPage | null>(null);
  loading = $state(false);
  error = $state<AtlasApiError | null>(null);
  /** A change from the list that failed. */
  actionError = $state<AtlasApiError | null>(null);

  /** Per button: what happened when it was pressed. */
  states = $state<Record<string, SaveState>>({});

  #seq = 0;
  #timer: ReturnType<typeof setTimeout> | null = null;

  async load() {
    const seq = ++this.#seq;
    this.loading = true;
    try {
      const result = await atlas.listSaved({
        kind: this.kind,
        filter: this.text,
        documentId: null,
        page: this.page,
        pageSize: SAVED_PAGE_SIZE,
      });
      if (seq === this.#seq) {
        this.result = result;
        this.error = null;
      }
    } catch (err) {
      if (seq === this.#seq) this.error = asApiError('list_saved', err);
    } finally {
      if (seq === this.#seq) this.loading = false;
    }
  }

  setKind(kind: SavedKind | null) {
    this.kind = kind;
    this.page = 0;
    return this.load();
  }

  setText(text: string, immediate = false) {
    this.text = text;
    this.page = 0;
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = null;
    if (immediate) {
      void this.load();
      return;
    }
    this.#timer = setTimeout(() => {
      this.#timer = null;
      void this.load();
    }, FILTER_DEBOUNCE_MS);
  }

  goTo(page: number) {
    this.page = Math.max(0, page);
    return this.load();
  }

  /** Bookmarks a place; `key` names the button that asked. */
  async bookmark(target: SaveTarget, key: string) {
    try {
      await atlas.saveBookmark(target);
      this.states = { ...this.states, [key]: { done: BOOKMARKED } };
    } catch (err) {
      this.states = { ...this.states, [key]: { error: asApiError('save_bookmark', err) } };
    }
  }

  async saveFinding(source: FindingSource, key: string) {
    try {
      await atlas.saveFinding(source);
      this.states = { ...this.states, [key]: { done: SAVED_FINDING } };
    } catch (err) {
      this.states = { ...this.states, [key]: { error: asApiError('save_finding', err) } };
    }
  }

  async setNote(kind: SavedKind, id: string, note: string): Promise<boolean> {
    this.actionError = null;
    try {
      await atlas.updateSavedNote(kind, id, note);
      await this.load();
      return true;
    } catch (err) {
      this.actionError = asApiError('update_saved_note', err);
      return false;
    }
  }

  async remove(kind: SavedKind, id: string) {
    this.actionError = null;
    try {
      await atlas.deleteSaved(kind, id);
      await this.load();
    } catch (err) {
      this.actionError = asApiError('delete_saved', err);
    }
  }

  /** A document's bookmarks, for the details panel. */
  async forDocument(documentId: string): Promise<Bookmark[]> {
    try {
      const page = await atlas.listSaved({
        kind: 'bookmark',
        filter: '',
        documentId,
        page: 0,
        pageSize: 100,
      });
      return page.items.flatMap((i) => (i.item === 'bookmark' ? [i] : []));
    } catch {
      return [];
    }
  }

  reset() {
    this.#seq++;
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = null;
    this.kind = null;
    this.text = '';
    this.page = 0;
    this.result = null;
    this.loading = false;
    this.error = null;
    this.actionError = null;
    this.states = {};
  }
}

export const saved = new SavedState();
