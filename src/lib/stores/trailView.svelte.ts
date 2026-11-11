/**
 * The Trails view (Slice 3.4, phase3-spec §5.1, §7.2): the list of trails,
 * the open trail's timeline, the changes made from it, and picking it up
 * again: Continue, Search again, Ask again.
 *
 * A recorded answer is shown from the store, never asked again. Search
 * again and Ask again are ordinary actions: they continue the trail, then
 * submit on the Search screen, so the new step joins it and the old one
 * stays as it was.
 */
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { AskStep } from '../api/bindings/AskStep';
import type { RecordingStatus } from '../api/bindings/RecordingStatus';
import type { SearchStep } from '../api/bindings/SearchStep';
import type { StepFilters } from '../api/bindings/StepFilters';
import type { TrailDetail } from '../api/bindings/TrailDetail';
import type { TrailPage } from '../api/bindings/TrailPage';
import { folderGone } from '../trailCopy';
import { answer } from './answer.svelte';
import { search } from './search.svelte';
import { sources } from './sources.svelte';
import { view } from './view.svelte';

export const TRAILS_PAGE_SIZE = 25;
const FILTER_DEBOUNCE_MS = 200;

const asApiError = (command: string, err: unknown) =>
  err instanceof AtlasApiError ? err : toApiError(command, err);

/** The recorded filters that still apply, and what was dropped. */
function usable(f: StepFilters) {
  const gone = f.sourceRootId !== null && !sources.roots.some((r) => r.id === f.sourceRootId);
  return {
    filters: { sourceRootId: gone ? null : f.sourceRootId, fileType: f.fileType },
    notice: gone ? folderGone(f.folderName) : null,
  };
}

class TrailViewState {
  // The list.
  text = $state('');
  page = $state(0);
  result = $state<TrailPage | null>(null);
  loading = $state(false);
  error = $state<AtlasApiError | null>(null);

  // The open trail.
  selectedId = $state<string | null>(null);
  detail = $state<TrailDetail | null>(null);
  detailLoading = $state(false);
  detailError = $state<AtlasApiError | null>(null);
  /** A change made from the timeline that failed. */
  actionError = $state<AtlasApiError | null>(null);
  /** Continue this trail was chosen. */
  continued = $state(false);

  #seq = 0;
  #detailSeq = 0;
  #timer: ReturnType<typeof setTimeout> | null = null;

  async load() {
    const seq = ++this.#seq;
    this.loading = true;
    try {
      const result = await atlas.listTrails({
        filter: this.text,
        page: this.page,
        pageSize: TRAILS_PAGE_SIZE,
      });
      if (seq === this.#seq) {
        this.result = result;
        this.error = null;
      }
    } catch (err) {
      if (seq === this.#seq) this.error = asApiError('list_trails', err);
    } finally {
      if (seq === this.#seq) this.loading = false;
    }
  }

  /** The filter field: searches titles, questions and searches. */
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

  /** Shows a trail's timeline. */
  async open(trailId: string) {
    this.selectedId = trailId;
    this.detail = null;
    this.actionError = null;
    this.continued = false;
    await this.#loadDetail();
  }

  /** Back to the list. */
  close() {
    this.#detailSeq++;
    this.selectedId = null;
    this.detail = null;
    this.detailError = null;
    this.actionError = null;
    this.continued = false;
    return this.load();
  }

  async #loadDetail() {
    const id = this.selectedId;
    if (!id) return;
    const seq = ++this.#detailSeq;
    this.detailLoading = true;
    try {
      const detail = await atlas.getTrail(id);
      if (seq === this.#detailSeq) {
        this.detail = detail;
        this.detailError = null;
      }
    } catch (err) {
      if (seq === this.#detailSeq) this.detailError = asApiError('get_trail', err);
    } finally {
      if (seq === this.#detailSeq) this.detailLoading = false;
    }
  }

  /** On `trail-changed`: an open timeline that gained a step reloads. */
  onStatus(status: RecordingStatus | null) {
    const shown = this.detail;
    if (!status || !shown || status.currentTrailId !== shown.trail.id) return;
    if (status.stepCount !== shown.trail.stepCount) void this.#loadDetail();
  }

  /** Runs a change to the open trail, then shows it again. */
  async #change(command: string, run: () => Promise<unknown>, reload = true): Promise<boolean> {
    this.actionError = null;
    try {
      await run();
      if (reload) await this.#loadDetail();
      return true;
    } catch (err) {
      this.actionError = asApiError(command, err);
      return false;
    }
  }

  async continueTrail() {
    const id = this.selectedId;
    if (!id) return;
    this.continued = await this.#change('continue_trail', () => atlas.continueTrail(id), false);
  }

  rename(title: string) {
    const id = this.selectedId;
    if (!id) return Promise.resolve(false);
    return this.#change('rename_trail', () => atlas.renameTrail(id, title));
  }

  pin(pinned: boolean) {
    const id = this.selectedId;
    if (!id) return Promise.resolve(false);
    return this.#change('pin_trail', () => atlas.pinTrail(id, pinned));
  }

  /** Pins from the list. */
  async pinInList(trailId: string, pinned: boolean) {
    try {
      await atlas.pinTrail(trailId, pinned);
      await this.load();
    } catch (err) {
      this.error = asApiError('pin_trail', err);
    }
  }

  async remove() {
    const id = this.selectedId;
    if (!id) return;
    if (await this.#change('delete_trail', () => atlas.deleteTrail(id), false)) {
      await this.close();
    }
  }

  async removeStep(stepId: string) {
    const last = this.detail?.steps.length === 1;
    const ok = await this.#change('delete_trail_step', () => atlas.deleteTrailStep(stepId), !last);
    // The trail goes with its last step.
    if (ok && last) await this.close();
  }

  /** A note on a step, or (no step) a note of its own at the end. */
  note(text: string, stepId?: string) {
    const id = this.selectedId;
    if (!id) return Promise.resolve(false);
    return this.#change('add_trail_note', () => atlas.addTrailNote(id, text, stepId));
  }

  /** Search again: the recorded words and filters, as a new step here. */
  async searchAgain(step: SearchStep) {
    const id = this.selectedId;
    if (!id) return;
    if (!(await this.#change('continue_trail', () => atlas.continueTrail(id), false))) return;
    const { filters, notice } = usable(step.filters);
    void search.apply(step.words, filters, notice);
    view.go('search');
  }

  /** Ask again: the recorded question and filters, asked now. */
  async askAgain(step: AskStep) {
    const id = this.selectedId;
    if (!id) return;
    if (!(await this.#change('continue_trail', () => atlas.continueTrail(id), false))) return;
    const { filters, notice } = usable(step.filters);
    search.mode = 'ask';
    search.sourceRootId = filters.sourceRootId;
    search.fileType = filters.fileType;
    answer.draft = step.question;
    void answer.ask(step.question);
    search.notice = notice;
    view.go('search');
  }

  reset() {
    this.#seq++;
    this.#detailSeq++;
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = null;
    this.text = '';
    this.page = 0;
    this.result = null;
    this.loading = false;
    this.error = null;
    this.selectedId = null;
    this.detail = null;
    this.detailLoading = false;
    this.detailError = null;
    this.actionError = null;
    this.continued = false;
  }
}

export const trailView = new TrailViewState();
