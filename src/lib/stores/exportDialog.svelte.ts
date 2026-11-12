/**
 * The Export dialog (Slice 3.8, phase3-spec §5.8): what to export (the item
 * it was opened from, or everything), the format and the link style. Rust
 * shows the folder dialog; the renderer never sends a path. Focus goes
 * back to the button that opened it.
 */
import { tick } from 'svelte';
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { ExportFormat } from '../api/bindings/ExportFormat';
import type { ExportReport } from '../api/bindings/ExportReport';
import type { ExportScope } from '../api/bindings/ExportScope';
import type { LinkStyle } from '../api/bindings/LinkStyle';
import type { ExportWhat } from '../exportCopy';

export interface ExportItem {
  what: ExportWhat;
  id: string;
  /** Its name, for the dialog's title. */
  name: string;
}

class ExportDialogState {
  open = $state(false);
  /** The item it was opened from; none from Settings. */
  item = $state<ExportItem | null>(null);
  everything = $state(false);
  format = $state<ExportFormat>('markdown');
  linkStyle = $state<LinkStyle>('obsidian');
  running = $state(false);
  report = $state<ExportReport | null>(null);
  error = $state<AtlasApiError | null>(null);

  #trigger: HTMLElement | null = null;

  show(item: ExportItem | null, trigger: HTMLElement | null = null) {
    this.#trigger = trigger;
    this.item = item;
    this.everything = item === null;
    this.report = null;
    this.error = null;
    this.running = false;
    this.open = true;
  }

  get scope(): ExportScope {
    if (this.everything || !this.item) return { kind: 'everything' };
    return { kind: this.item.what, id: this.item.id };
  }

  /** Shows the folder dialog, then exports. A cancelled dialog says nothing. */
  async run() {
    if (this.running) return;
    this.running = true;
    this.error = null;
    this.report = null;
    try {
      this.report = await atlas.exportKnowledge(this.scope, this.format, this.linkStyle);
    } catch (err) {
      this.error = err instanceof AtlasApiError ? err : toApiError('export_knowledge', err);
    } finally {
      this.running = false;
    }
  }

  /** Closes; the format and link style stay for the next export. */
  async close() {
    const trigger = this.#trigger;
    this.#clear();
    await tick();
    if (trigger?.isConnected) trigger.focus();
  }

  /** A library opened: the choices go back to their defaults too. */
  reset() {
    this.#clear();
    this.format = 'markdown';
    this.linkStyle = 'obsidian';
  }

  #clear() {
    this.#trigger = null;
    this.open = false;
    this.item = null;
    this.everything = false;
    this.running = false;
    this.report = null;
    this.error = null;
  }
}

export const exportDialog = new ExportDialogState();
