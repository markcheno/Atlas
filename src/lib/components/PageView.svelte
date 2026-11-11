<script lang="ts">
  // The page view (Slice 3.7, phase3-spec §5.5): a PDF page as Atlas
  // rendered it, the passage outlined over the image when found (drawn by
  // the app, so it follows the theme), and the page's text below it for
  // reading aloud and copying. Escape closes and returns focus.
  import { tick } from 'svelte';
  import {
    ACTUAL_SIZE,
    BOOKMARK_PAGE,
    CLOUD_ONLY,
    COULD_NOT_SHOW,
    DOWNLOAD_AND_SHOW,
    FIT_WIDTH,
    NOT_FOUND_ON_PAGE,
    OPEN_DEFAULT,
    PAGE_TEXT,
    pageOf,
    showing,
  } from '../readerCopy';
  import { originals } from '../stores/originals.svelte';
  import { pageView } from '../stores/pageView.svelte';
  import SaveAction from './SaveAction.svelte';

  let dialog = $state<HTMLElement>();
  let closeButton = $state<HTMLButtonElement>();
  // What is typed in the page field; follows the page shown.
  let pageField = $derived(String(pageView.page));

  const req = $derived(pageView.request);
  const image = $derived(pageView.image);
  const count = $derived(image?.pageCount ?? null);

  $effect(() => {
    if (pageView.open) void tick().then(() => closeButton?.focus());
  });

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      void pageView.close();
      return;
    }
    if (event.key !== 'Tab' || !dialog) return;
    const focusable = [
      ...dialog.querySelectorAll<HTMLElement>('button:not(:disabled), input, [tabindex="0"]'),
    ];
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  function goToField(event?: Event) {
    event?.preventDefault();
    const n = Number.parseInt(pageField, 10);
    if (Number.isFinite(n)) void pageView.go(n);
  }

  function openDefault(allowDownload = false) {
    if (!req) return;
    void originals.open(
      { id: req.fileId, displayName: req.title, availability: 'available', via: req.via },
      allowDownload,
    );
  }
</script>

{#if req}
  <div class="backdrop" role="presentation" onclick={() => void pageView.close()}></div>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="page-view-title"
    bind:this={dialog}
    onkeydown={onKeydown}
    tabindex="-1"
  >
    <header class="head">
      <h2 id="page-view-title">{req.title}</h2>
      <button
        type="button"
        class="button quiet"
        aria-label="Close the page"
        bind:this={closeButton}
        onclick={() => void pageView.close()}
      >
        Close
      </button>
    </header>

    <div class="controls">
      <button
        type="button"
        class="button"
        disabled={pageView.page <= 1 || pageView.loading}
        onclick={() => void pageView.go(pageView.page - 1)}
      >
        Previous page
      </button>
      <form class="page-field" onsubmit={goToField}>
        <label for="page-view-number" class="visually-hidden">Page number</label>
        <span aria-hidden="true">Page</span>
        <input
          id="page-view-number"
          type="number"
          min="1"
          max={count ?? undefined}
          value={pageField}
          oninput={(e) => (pageField = e.currentTarget.value)}
          onkeydown={(e) => {
            if (e.key === 'Enter') goToField(e);
          }}
        />
        {#if count}<span>of {count.toLocaleString('en-US')}</span>{/if}
      </form>
      <button
        type="button"
        class="button"
        disabled={(count !== null && pageView.page >= count) || pageView.loading}
        onclick={() => void pageView.go(pageView.page + 1)}
      >
        Next page
      </button>
      <fieldset class="fit">
        <legend class="visually-hidden">Size</legend>
        <label>
          <input
            type="radio"
            name="page-fit"
            checked={pageView.fit === 'width'}
            onchange={() => (pageView.fit = 'width')}
          />
          {FIT_WIDTH}
        </label>
        <label>
          <input
            type="radio"
            name="page-fit"
            checked={pageView.fit === 'actual'}
            onchange={() => (pageView.fit = 'actual')}
          />
          {ACTUAL_SIZE}
        </label>
      </fieldset>
      <SaveAction
        target={{
          kind: 'file',
          fileId: req.fileId,
          place: { kind: 'page', pageNo: pageView.page },
        }}
        key={`bookmark-page-${req.fileId}-${pageView.page}`}
        label={BOOKMARK_PAGE}
      />
      <button type="button" class="button quiet" onclick={() => openDefault()}>
        {OPEN_DEFAULT}
      </button>
    </div>

    <div class="body">
      {#if pageView.loading}
        <p class="muted" role="status" data-testid="page-status">{showing(pageView.page)}</p>
      {/if}
      {#if pageView.needsDownload}
        <div class="notice" role="alert">
          <p>{CLOUD_ONLY}</p>
          <button
            type="button"
            class="button primary"
            onclick={() => void pageView.go(pageView.page, true)}
          >
            {DOWNLOAD_AND_SHOW}
          </button>
        </div>
      {:else if pageView.error}
        <div class="notice" role="alert" data-testid="page-error">
          <p><strong>{COULD_NOT_SHOW}</strong></p>
          <p class="small">{pageView.error.message}</p>
          <button type="button" class="button" onclick={() => openDefault()}>{OPEN_DEFAULT}</button>
        </div>
      {:else if image && !pageView.loading}
        <figure class="page" class:actual={pageView.fit === 'actual'} data-testid="page-image">
          <div class="frame" style:aspect-ratio={`${image.width} / ${image.height}`}>
            <img
              src={`data:image/png;base64,${image.png}`}
              alt={pageOf(image.pageNo, image.pageCount)}
              width={image.width}
              height={image.height}
              draggable="false"
            />
            {#each image.rects as r, i (i)}
              <span
                class="outline"
                data-testid="page-outline"
                style:left={`${(r.x / image.width) * 100}%`}
                style:top={`${(r.y / image.height) * 100}%`}
                style:width={`${(r.width / image.width) * 100}%`}
                style:height={`${(r.height / image.height) * 100}%`}
              ></span>
            {/each}
          </div>
          <figcaption class="muted small">
            {pageOf(image.pageNo, image.pageCount)}
            {#if req.find && image.pageNo === req.pageNo && !image.found}
              · {NOT_FOUND_ON_PAGE}
            {/if}
          </figcaption>
        </figure>
        {#if image.text}
          <section aria-labelledby="page-text-title" class="text">
            <h3 id="page-text-title">{PAGE_TEXT}</h3>
            <p>{image.text}</p>
          </section>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: rgb(0 0 0 / 0.45);
  }

  .dialog {
    position: fixed;
    inset: var(--space-4);
    z-index: 31;
    display: grid;
    grid-template-rows: auto auto 1fr;
    gap: var(--space-3);
    max-width: 1000px;
    margin: 0 auto;
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    overflow: hidden;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  h2 {
    margin: 0;
    font-size: 17px;
    overflow-wrap: anywhere;
  }

  h3 {
    margin: 0 0 var(--space-2);
    font-size: 14px;
  }

  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .page-field {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .page-field input {
    width: 5em;
    min-height: var(--control-height, 36px);
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .fit {
    display: inline-flex;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    border: 0;
  }

  .body {
    display: grid;
    gap: var(--space-4);
    align-content: start;
    overflow: auto;
  }

  .page {
    margin: 0;
  }

  .frame {
    position: relative;
    width: 100%;
    background: white;
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.2);
  }

  .page.actual .frame {
    width: auto;
    max-width: none;
  }

  .page.actual .frame img {
    width: auto;
  }

  img {
    display: block;
    width: 100%;
    height: auto;
    user-select: none;
  }

  .outline {
    position: absolute;
    border: 2px solid var(--accent);
    border-radius: 2px;
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    pointer-events: none;
  }

  .notice {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--notice-border);
    border-radius: var(--radius);
    background: var(--notice-bg);
  }

  p {
    margin: 0;
  }

  .text p {
    white-space: pre-line;
    overflow-wrap: anywhere;
    line-height: 1.5;
  }

  .small {
    font-size: 13px;
  }
</style>
