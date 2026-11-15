/**
 * Explore (Slice 4.6.1): the map's status and, once drawn, the map itself,
 * kept current by the Rust watch's `explore-status` events. The selection
 * and the books a search found live here too, so the list and the map
 * agree.
 */
import { isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { ExploreMap } from '../api/bindings/ExploreMap';
import type { ExploreStatus } from '../api/bindings/ExploreStatus';

export const EXPLORE_EVENT = 'explore-status';

/** How many results "Find on the map" asks for. */
export const FIND_LIMIT = 200;

function asApiError(command: string, err: unknown): AtlasApiError {
  return err instanceof AtlasApiError ? err : toApiError(command, err);
}

class ExploreStore {
  status = $state<ExploreStatus | null>(null);
  map = $state<ExploreMap | null>(null);
  error = $state<AtlasApiError | null>(null);
  /** The chosen area or sub-area's id, lit on the map. */
  selected = $state<number | null>(null);
  /** What "Find on the map" looked for, and the documents it found. */
  findText = $state('');
  found = $state<Set<string> | null>(null);
  foundCount = $state(0);
  finding = $state(false);
  #unlisten: UnlistenFn | null = null;
  #seq = 0;

  reset() {
    this.status = null;
    this.map = null;
    this.error = null;
    this.selected = null;
    this.clearFound();
  }

  /** Loads the status, and the map when it has a new generation. */
  async refresh() {
    try {
      const status = await atlas.getExploreStatus();
      this.status = status;
      if (status.state === 'ready' && status.generation !== this.map?.status.generation) {
        await this.loadMap();
      } else if (status.state !== 'ready') {
        this.map = null;
      }
      this.error = null;
    } catch (err) {
      this.error = asApiError('get_explore_status', err);
    }
  }

  async loadMap() {
    const seq = ++this.#seq;
    try {
      const map = await atlas.getExploreMap();
      if (seq !== this.#seq) return;
      this.map = map;
      this.status = map.status;
      // A choice that no longer exists is let go.
      const ids = new Set(map.areas.flatMap((a) => [a.id, ...a.subs.map((s) => s.id)]));
      if (this.selected !== null && !ids.has(this.selected)) this.selected = null;
    } catch (err) {
      if (seq === this.#seq) this.error = asApiError('get_explore_map', err);
    }
  }

  async subscribe() {
    if (this.#unlisten || !isTauri()) return;
    this.#unlisten = await listen<ExploreStatus>(EXPLORE_EVENT, (event) => {
      this.status = event.payload;
      if (
        event.payload.state === 'ready' &&
        !event.payload.building &&
        event.payload.generation !== this.map?.status.generation
      ) {
        void this.loadMap();
      }
    });
  }

  /** Draw the map again (`fresh`: from scratch). */
  async redraw(fresh = false) {
    try {
      this.status = await atlas.refreshExploreMap(fresh);
      this.error = null;
    } catch (err) {
      this.error = asApiError('refresh_explore_map', err);
    }
  }

  select(id: number | null) {
    this.selected = this.selected === id ? null : id;
  }

  /** Names an area; null or blank goes back to its words. Throws. */
  async rename(areaId: number, name: string | null) {
    await atlas.renameArea(areaId, name && name.trim() ? name.trim() : null);
    await this.loadMap();
  }

  /** Lights up the books a search finds. */
  async find(text: string) {
    this.findText = text;
    if (!text.trim()) {
      this.clearFound();
      return;
    }
    this.finding = true;
    try {
      // Up to FIND_LIMIT results, a page at a time (search pages hold 50).
      // A refresh, not a submit: lighting books up isn't a trail step.
      const hits = [];
      for (let page = 0; hits.length < FIND_LIMIT; page++) {
        const result = await atlas.searchDocuments(
          {
            text,
            sourceRootId: null,
            fileType: null,
            includeMissing: false,
            page,
            pageSize: 50,
          },
          'refresh',
        );
        hits.push(...result.hits);
        if (result.hits.length < 50) break;
      }
      const docs = new Set(
        hits.map((h) => h.documentId).filter((d): d is string => typeof d === 'string'),
      );
      this.found = docs;
      this.foundCount = hits.length;
      this.error = null;
    } catch (err) {
      this.error = asApiError('search_documents', err);
    } finally {
      this.finding = false;
    }
  }

  clearFound() {
    this.findText = '';
    this.found = null;
    this.foundCount = 0;
  }
}

export const explore = new ExploreStore();
