/**
 * Research trails (Slice 3.3, phase3-spec §5.2, §7.1): the status bar's
 * indicator, New trail, Home's one-time note, and Delete all trails.
 * Recording happens in Rust; this store only says what the user did and
 * shows what Rust reports through `trail-changed`.
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { isTauri } from '@tauri-apps/api/core';
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { OpenAction } from '../api/bindings/OpenAction';
import type { OpenVia } from '../api/bindings/OpenVia';
import type { RecordingStatus } from '../api/bindings/RecordingStatus';
import { settings } from './settings.svelte';

export const TRAIL_EVENT = 'trail-changed';

/**
 * Tells Rust a source was opened, after it was. Never fails the open: a
 * problem shows in the indicator. No `via` (the Library) records nothing.
 */
export function recordOpen(
  fileId: string,
  via: OpenVia | undefined,
  action: OpenAction,
  pageNo?: number,
) {
  if (!via || via.kind === 'library') return;
  const place = pageNo === undefined ? undefined : { pageNo };
  void atlas.recordOpen(fileId, via, action, place).catch(() => {});
}

const asApiError = (command: string, err: unknown) =>
  err instanceof AtlasApiError ? err : toApiError(command, err);

class TrailsState {
  status = $state<RecordingStatus | null>(null);
  /** New trail was chosen; cleared by the next step. */
  starting = $state(false);
  busy = $state(false);
  error = $state<AtlasApiError | null>(null);
  #unlisten: UnlistenFn | null = null;

  get recording(): boolean {
    return this.status?.state === 'recording';
  }

  async subscribe() {
    if (this.#unlisten || !isTauri()) return;
    this.#unlisten = await listen<RecordingStatus>(TRAIL_EVENT, (event) => {
      this.#apply(event.payload);
    });
  }

  #apply(status: RecordingStatus) {
    if (status.currentTrailId) this.starting = false;
    this.status = status;
  }

  async refresh() {
    try {
      this.#apply(await atlas.getRecordingStatus());
    } catch {
      this.status = null;
    }
  }

  /** Pauses or resumes recording, through Settings › Privacy. */
  async toggle() {
    const on = !this.recording;
    await this.setRecording(on);
  }

  async setRecording(on: boolean) {
    this.error = null;
    try {
      // Never save defaults over choices that weren't read.
      if (!settings.loaded && !(await settings.load())) {
        throw toApiError('get_settings', null);
      }
      await settings.update((s) => ({ ...s, privacy: { ...s.privacy, recordTrails: on } }));
      await this.refresh();
    } catch (err) {
      this.error = asApiError('update_settings', err);
    }
  }

  async startTrail() {
    this.error = null;
    try {
      this.#apply(await atlas.startTrail());
      this.starting = true;
    } catch (err) {
      this.error = asApiError('start_trail', err);
    }
  }

  async dismissNote() {
    try {
      this.#apply(await atlas.dismissTrailNote());
    } catch (err) {
      this.error = asApiError('dismiss_trail_note', err);
    }
  }

  /** Deletes every trail; resolves to how many went. */
  async deleteAll(): Promise<number | null> {
    this.busy = true;
    this.error = null;
    try {
      const n = await atlas.deleteAllTrails();
      await this.refresh();
      return n;
    } catch (err) {
      this.error = asApiError('delete_all_trails', err);
      return null;
    } finally {
      this.busy = false;
    }
  }

  reset() {
    this.status = null;
    this.starting = false;
    this.busy = false;
    this.error = null;
  }
}

export const trails = new TrailsState();
