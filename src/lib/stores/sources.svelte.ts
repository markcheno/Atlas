/** Library folders (source roots) of the open data root. */
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { SourceRoot } from '../api/bindings/SourceRoot';
import type { SourceRootRejection } from '../api/bindings/SourceRootRejection';

function asApiError(command: string, err: unknown): AtlasApiError {
  return err instanceof AtlasApiError ? err : toApiError(command, err);
}

class SourcesState {
  roots = $state<SourceRoot[]>([]);
  /** Folders refused by the most recent pick, with reasons. */
  rejected = $state<SourceRootRejection[]>([]);
  error = $state<AtlasApiError | null>(null);
  busy = $state(false);

  clear() {
    this.roots = [];
    this.rejected = [];
    this.error = null;
  }

  async refresh() {
    try {
      this.roots = await atlas.listSourceRoots();
      this.error = null;
    } catch (err) {
      this.roots = [];
      this.error = asApiError('list_source_roots', err);
    }
  }

  /** Opens the folder picker and adds whatever the user chose. */
  async addFromPicker() {
    this.busy = true;
    this.error = null;
    try {
      const result = await atlas.pickAndAddSourceRoots();
      this.rejected = result.rejected;
      if (result.added.length > 0) await this.refresh();
    } catch (err) {
      this.error = asApiError('pick_and_add_source_roots', err);
    } finally {
      this.busy = false;
    }
  }

  async remove(id: string) {
    this.error = null;
    try {
      await atlas.removeSourceRoot(id);
    } catch (err) {
      this.error = asApiError('remove_source_root', err);
    }
    await this.refresh();
  }

  displayName(id: string | null): string | null {
    if (!id) return null;
    return this.roots.find((r) => r.id === id)?.displayPath ?? null;
  }
}

export const sources = new SourcesState();

/** The last path component, for compact display. */
export function folderName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}
