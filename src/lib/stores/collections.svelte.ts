/**
 * Interests and projects (Slice 3.6, phase3-spec §5.4, §5.6): the lists,
 * the open collection's page, Add to…, and Home's suggestions.
 */
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { Collection } from '../api/bindings/Collection';
import type { CollectionDetail } from '../api/bindings/CollectionDetail';
import type { CollectionInput } from '../api/bindings/CollectionInput';
import type { CollectionItemRef } from '../api/bindings/CollectionItemRef';
import type { CollectionStatus } from '../api/bindings/CollectionStatus';
import type { Suggestion } from '../api/bindings/Suggestion';

const asApiError = (command: string, err: unknown) =>
  err instanceof AtlasApiError ? err : toApiError(command, err);

class CollectionsState {
  /** Every collection, for the tab. */
  all = $state<Collection[] | null>(null);
  /** The active ones, for Add to… and Home. */
  active = $state<Collection[]>([]);
  error = $state<AtlasApiError | null>(null);

  selectedId = $state<string | null>(null);
  detail = $state<CollectionDetail | null>(null);
  detailError = $state<AtlasApiError | null>(null);
  actionError = $state<AtlasApiError | null>(null);

  suggestions = $state<Suggestion[]>([]);

  async load() {
    try {
      const all = await atlas.listCollections(null);
      this.all = all;
      this.active = all.filter((c) => c.status === 'active');
      this.error = null;
    } catch (err) {
      this.error = asApiError('list_collections', err);
    }
  }

  async open(id: string) {
    this.selectedId = id;
    this.detail = null;
    this.actionError = null;
    await this.#loadDetail();
  }

  close() {
    this.selectedId = null;
    this.detail = null;
    this.detailError = null;
    this.actionError = null;
    return this.load();
  }

  async #loadDetail() {
    const id = this.selectedId;
    if (!id) return;
    try {
      const detail = await atlas.getCollection(id);
      if (this.selectedId === id) {
        this.detail = detail;
        this.detailError = null;
      }
    } catch (err) {
      if (this.selectedId === id) this.detailError = asApiError('get_collection', err);
    }
  }

  /** Creates or changes one; resolves to it, or null with `actionError`. */
  async save(input: CollectionInput): Promise<Collection | null> {
    this.actionError = null;
    try {
      const saved = await atlas.saveCollection(input);
      await this.load();
      if (this.selectedId === saved.id) await this.#loadDetail();
      return saved;
    } catch (err) {
      this.actionError = asApiError('save_collection', err);
      return null;
    }
  }

  setStatus(c: Collection, status: CollectionStatus) {
    return this.save({
      id: c.id,
      kind: c.kind,
      name: c.name,
      description: c.description,
      goal: c.goal,
      status,
    });
  }

  async remove(id: string) {
    this.actionError = null;
    try {
      await atlas.deleteCollection(id);
      await this.close();
    } catch (err) {
      this.actionError = asApiError('delete_collection', err);
    }
  }

  async add(collectionId: string, item: CollectionItemRef): Promise<boolean> {
    this.actionError = null;
    try {
      await atlas.addToCollection(collectionId, item);
      await this.load();
      if (this.selectedId === collectionId) await this.#loadDetail();
      return true;
    } catch (err) {
      this.actionError = asApiError('add_to_collection', err);
      return false;
    }
  }

  async take(collectionId: string, item: CollectionItemRef): Promise<boolean> {
    this.actionError = null;
    try {
      await atlas.removeFromCollection(collectionId, item);
      await this.load();
      if (this.selectedId === collectionId) await this.#loadDetail();
      return true;
    } catch (err) {
      this.actionError = asApiError('remove_from_collection', err);
      return false;
    }
  }

  async membership(item: CollectionItemRef): Promise<string[]> {
    try {
      return (await atlas.getCollectionMembership(item)).collectionIds;
    } catch {
      return [];
    }
  }

  async loadSuggestions(limit = 3) {
    try {
      this.suggestions = await atlas.getSuggestions(limit);
    } catch {
      this.suggestions = [];
    }
  }

  async dismiss(s: Suggestion) {
    const documentId = s.hit.documentId;
    if (!documentId) return;
    try {
      await atlas.dismissSuggestion(s.seedId, documentId);
    } catch {
      // Shown again next time; nothing else changes.
    }
    await this.loadSuggestions(3);
  }

  reset() {
    this.all = null;
    this.active = [];
    this.error = null;
    this.selectedId = null;
    this.detail = null;
    this.detailError = null;
    this.actionError = null;
    this.suggestions = [];
  }
}

export const collections = new CollectionsState();
