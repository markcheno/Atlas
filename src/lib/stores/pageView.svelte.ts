/**
 * The page view (Slice 3.7, phase3-spec §5.5): which PDF page is shown, how,
 * and where focus goes back to when it closes. A way to see a cited page,
 * not a reader: no selection on the image, no search, no scrolling between
 * pages.
 */
import { tick } from 'svelte';
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { OpenVia } from '../api/bindings/OpenVia';
import type { PageImage } from '../api/bindings/PageImage';
import { recordOpen } from './trails.svelte';

/** The width asked for: crisp at 1.5× on a 800-pixel-wide view. */
export const RENDER_WIDTH = 1200;

export interface PageRequest {
  fileId: string;
  pageNo: number;
  /** Text already on screen, outlined when it is on the page. */
  find?: string | null;
  title: string;
  via?: OpenVia;
}

class PageViewState {
  request = $state<PageRequest | null>(null);
  page = $state(1);
  image = $state<PageImage | null>(null);
  loading = $state(false);
  error = $state<AtlasApiError | null>(null);
  /** The file is only in the cloud: asks before downloading it. */
  needsDownload = $state(false);
  fit = $state<'width' | 'actual'>('width');

  #seq = 0;
  #trigger: HTMLElement | null = null;

  get open(): boolean {
    return this.request !== null;
  }

  async show(request: PageRequest, trigger: HTMLElement | null = null) {
    this.#trigger = trigger;
    this.request = request;
    this.fit = 'width';
    await this.go(request.pageNo);
    recordOpen(request.fileId, request.via, 'page', request.pageNo);
  }

  /** Another page of the same document. The passage is looked for only on
   * the page it was cited on. */
  async go(page: number, allowDownload = false) {
    const request = this.request;
    if (!request) return;
    const count = this.image?.pageCount;
    const target = Math.max(1, count ? Math.min(page, count) : page);
    const seq = ++this.#seq;
    this.page = target;
    this.loading = true;
    this.error = null;
    this.needsDownload = false;
    try {
      const image = await atlas.renderPage(
        request.fileId,
        target,
        RENDER_WIDTH,
        target === request.pageNo ? (request.find ?? null) : null,
        allowDownload,
      );
      if (seq === this.#seq) this.image = image;
    } catch (err) {
      if (seq !== this.#seq) return;
      const error = err instanceof AtlasApiError ? err : toApiError('render_page', err);
      if (error.code === 'file_cloud_only') this.needsDownload = true;
      else this.error = error;
      this.image = null;
    } finally {
      if (seq === this.#seq) this.loading = false;
    }
  }

  async close() {
    const trigger = this.#trigger;
    this.reset();
    await tick();
    if (trigger?.isConnected) trigger.focus();
  }

  reset() {
    this.#seq++;
    this.#trigger = null;
    this.request = null;
    this.page = 1;
    this.image = null;
    this.loading = false;
    this.error = null;
    this.needsDownload = false;
    this.fit = 'width';
  }
}

export const pageView = new PageViewState();
