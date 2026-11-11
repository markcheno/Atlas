<script lang="ts">
  // Full-text search (phase1-spec §5.2): results grouped by document, a
  // highlighted snippet built from typed spans (never markup), why each one
  // matched, and Open, Reveal and Details on every result; More like this
  // when meaning search is set up (Slice 2.5). Ask Atlas (Slice 2.7) is the
  // screen's other mode: a question, an answer with citations, and the
  // sources used, under the same folder and type filters.
  import { untrack } from 'svelte';
  import type { SearchHit } from '../lib/api/bindings/SearchHit';
  import AnswerView from '../lib/components/AnswerView.svelte';
  import AskForm from '../lib/components/AskForm.svelte';
  import OriginalActions from '../lib/components/OriginalActions.svelte';
  import OriginalNotice from '../lib/components/OriginalNotice.svelte';
  import ProblemNotice from '../lib/components/ProblemNotice.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { statusLabel } from '../lib/libraryCopy';
  import {
    alsoIn,
    readingNote,
    readSoFar,
    resultRange,
    SEARCH_HELP,
    WHY_LABEL,
    foundByWords,
    meaningNote,
    meaningPending,
  } from '../lib/searchCopy';
  import { FROM_RESULT } from '../lib/openVia';
  import { NEW_TRAIL, NEW_TRAIL_DONE } from '../lib/trailCopy';
  import { details } from '../lib/stores/details.svelte';
  import { embedding } from '../lib/stores/embedding.svelte';
  import { extraction } from '../lib/stores/extraction.svelte';
  import { fromHit, originals } from '../lib/stores/originals.svelte';
  import { search, SEARCH_PAGE_SIZE, type SearchMode } from '../lib/stores/search.svelte';
  import { folderName, sources } from '../lib/stores/sources.svelte';
  import { trails } from '../lib/stores/trails.svelte';

  const scannableRoots = $derived(sources.roots.filter((r) => !r.nestedInRootId));
  const page = $derived(search.result);
  const lastPage = $derived(
    page ? Math.max(0, Math.ceil(page.matchedDocuments / SEARCH_PAGE_SIZE) - 1) : 0,
  );

  function rootName(hit: SearchHit): string {
    const root = sources.roots.find((r) => r.id === hit.sourceRootId);
    return root ? folderName(root.displayPath) : '';
  }

  const asking = $derived(search.mode === 'ask');

  function setMode(mode: SearchMode) {
    void search.setMode(mode);
  }

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void search.submit();
  }

  // While Atlas is still reading text, refresh the shown results as it
  // goes (throttled in the store).
  $effect(() => {
    if (extraction.overview) {
      untrack(() => {
        if (search.result?.readingText) search.refreshLive();
      });
    }
  });

  // Meaning search was loading its index or the service was slow: the
  // next embedding-status event is a good moment to search again.
  $effect(() => {
    if (embedding.overview) {
      untrack(() => {
        if (search.result && meaningPending(search.result.semantic)) search.refreshLive();
      });
    }
  });
</script>

<div class="view">
  <ViewHeading>Search</ViewHeading>

  <div class="mode-row">
    <fieldset class="mode">
      <legend class="visually-hidden">Search mode</legend>
      <label>
        <input
          type="radio"
          name="search-mode"
          value="find"
          checked={!asking}
          onchange={() => setMode('find')}
        />
        Find sources
      </label>
      <label>
        <input
          type="radio"
          name="search-mode"
          value="ask"
          checked={asking}
          onchange={() => setMode('ask')}
        />
        Ask Atlas
      </label>
    </fieldset>
    {#if trails.status && trails.status.state !== 'unavailable'}
      <button
        type="button"
        class="button quiet"
        disabled={trails.status.state === 'paused'}
        onclick={() => void trails.startTrail()}
      >
        {NEW_TRAIL}
      </button>
      <span role="status" class="muted small" data-testid="new-trail-status">
        {trails.starting ? NEW_TRAIL_DONE : ''}
      </span>
    {/if}
  </div>

  {#if search.notice}
    <p class="muted small" role="status" data-testid="search-notice">{search.notice}</p>
  {/if}

  {#if asking}
    <AskForm />
  {:else}
    <form class="box" role="search" onsubmit={submit}>
      <label for="search-input">Search your library</label>
      <div class="row">
        <input
          id="search-input"
          type="search"
          placeholder="For example: meteor, “harmonic drive”, café"
          bind:value={search.draft}
          aria-describedby="search-help"
        />
        <button type="submit" class="button primary">Search</button>
      </div>
      <p id="search-help" class="muted small">{SEARCH_HELP}</p>
    </form>
  {/if}

  {#if sources.roots.length > 0}
    <div class="filters">
      <label>
        <span class="muted small">Folder</span>
        <select
          value={search.sourceRootId ?? ''}
          onchange={(e) => search.setFilters({ sourceRootId: e.currentTarget.value || null })}
        >
          <option value="">All folders</option>
          {#each scannableRoots as root (root.id)}
            <option value={root.id}>{folderName(root.displayPath)}</option>
          {/each}
        </select>
      </label>
      <label>
        <span class="muted small">Type</span>
        <select
          value={search.fileType ?? ''}
          onchange={(e) =>
            search.setFilters({
              fileType: (e.currentTarget.value || null) as 'pdf' | 'epub' | null,
            })}
        >
          <option value="">All types</option>
          <option value="pdf">PDF</option>
          <option value="epub">EPUB</option>
        </select>
      </label>
    </div>
  {/if}

  {#if asking}
    <AnswerView />
  {:else if search.noWords}
    <div class="empty"><p>Type a word to search for.</p></div>
  {:else if search.error}
    <ProblemNotice
      title="Atlas couldn't search your library"
      message={search.error.message}
      recovery={search.error.recovery}
    />
  {:else if page}
    {#if page.readingText && page.textReadFiles < page.totalFiles}
      <p class="note" data-testid="reading-note">
        {readingNote(page.textReadFiles, page.totalFiles)}
      </p>
    {/if}
    {#if meaningNote(page.semantic)}
      <p class="note" data-testid="meaning-note">{meaningNote(page.semantic)}</p>
    {/if}
    {#if page.hits.length === 0}
      <div class="empty">
        {#if page.textReadFiles === 0}
          <p>Atlas hasn't read any text yet. It searches file names meanwhile.</p>
          <p class="muted">No file names match “{search.text}”.</p>
        {:else}
          <p>Nothing in your library mentions “{search.text}”.</p>
          <p class="muted">{readSoFar(page.textReadFiles, page.totalFiles)}</p>
        {/if}
      </div>
    {:else}
      <h2 class="visually-hidden">Results</h2>
      <ol class="results" aria-busy={search.loading}>
        {#each page.hits as hit (hit.fileInstanceId)}
          {@const also = alsoIn(hit.otherLocations)}
          <li data-testid="search-hit">
            <h3>{hit.title}</h3>
            <p class="meta muted small">
              <span>{hit.fileType.toUpperCase()}</span>
              <span>{rootName(hit)}</span>
              {#if hit.location}<span>{hit.location}</span>{/if}
              {#if hit.titleSource !== 'file_name'}<span>{hit.displayName}</span>{/if}
            </p>
            {#if hit.snippet.length > 0}
              <p class="snippet" data-testid="snippet">
                {#each hit.snippet as span, i (i)}{#if span.hit}<mark>{span.text}</mark
                    >{:else}{span.text}{/if}{/each}
              </p>
            {/if}
            <p class="why small">
              <span>{WHY_LABEL[hit.why]}</span>
              {#if !hit.allWords && foundByWords(hit.why)}<span>Some of your words</span>{/if}
              {#if also}<span>{also}</span>{/if}
              {#if hit.availability !== 'available' || hit.storage === 'cloud_only'}
                <span class="badge" data-status={hit.availability}>{statusLabel(hit)}</span>
              {/if}
            </p>
            <div class="actions">
              <button
                type="button"
                class="row-button"
                aria-label={`Details for ${hit.displayName}`}
                onclick={(e) =>
                  details.open(hit.fileInstanceId, e.currentTarget, { via: FROM_RESULT })}
              >
                Details
              </button>
              {#if hit.documentId && page.semantic.state !== 'off'}
                <button
                  type="button"
                  class="row-button"
                  aria-label={`More like this: ${hit.displayName}`}
                  onclick={(e) =>
                    details.open(hit.fileInstanceId, e.currentTarget, {
                      focusRelated: true,
                      via: FROM_RESULT,
                    })}
                >
                  More like this
                </button>
              {/if}
              <OriginalActions item={fromHit(hit, FROM_RESULT)} />
            </div>
            {#if originals.hasNotice(hit.fileInstanceId)}
              <OriginalNotice item={fromHit(hit, FROM_RESULT)} />
            {/if}
          </li>
        {/each}
      </ol>

      <nav class="pager" aria-label="Result pages">
        <span class="muted" role="status" data-testid="search-range">
          {resultRange(
            page.page,
            SEARCH_PAGE_SIZE,
            page.hits.length,
            page.matchedDocuments,
            page.capped,
          )}
        </span>
        <div>
          <button
            type="button"
            class="button"
            disabled={search.page === 0}
            onclick={() => search.goTo(search.page - 1)}
          >
            Previous
          </button>
          <button
            type="button"
            class="button"
            disabled={search.page >= lastPage}
            onclick={() => search.goTo(search.page + 1)}
          >
            Next
          </button>
        </div>
      </nav>
    {/if}
  {:else if search.loading}
    <p class="muted" role="status">Searching…</p>
  {/if}
</div>

<style>
  .view {
    display: grid;
    gap: var(--space-5);
    max-width: 900px;
    margin: 0 auto;
  }

  .box {
    display: grid;
    gap: var(--space-2);
  }

  .mode-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .mode {
    display: inline-flex;
    justify-self: start;
    gap: 0;
    margin: 0;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    min-height: 40px;
  }

  .mode label {
    display: inline-flex;
    align-items: center;
    padding: 0 var(--space-3);
    border-radius: calc(var(--radius) - 2px);
    cursor: pointer;
  }

  .mode label:has(input:checked) {
    background: var(--accent-soft);
    font-weight: 500;
  }

  .mode label:has(input:focus-visible) {
    outline: 2px solid var(--focus);
    outline-offset: 1px;
  }

  .mode input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .box label {
    font-weight: 500;
  }

  .row {
    display: flex;
    gap: var(--space-2);
  }

  .row input {
    flex: 1;
    min-height: 48px;
    padding: 0 var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    font-size: 16px;
  }

  .row .button {
    min-height: 48px;
  }

  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-4);
  }

  .filters label {
    display: grid;
    gap: var(--space-1);
  }

  .filters select {
    min-height: 40px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .note {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--notice-border);
    border-radius: var(--radius);
    background: var(--notice-bg);
  }

  .empty {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-6);
    border: 1px dashed var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  .empty p {
    margin: 0;
  }

  .results {
    display: grid;
    gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .results li {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  h3 {
    margin: 0;
    font-size: 16px;
    overflow-wrap: anywhere;
  }

  .results p {
    margin: 0;
  }

  .meta,
  .why {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-3);
  }

  .snippet {
    line-height: 1.5;
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .pager {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-4);
  }

  .pager div {
    display: flex;
    gap: var(--space-2);
  }

  .small {
    font-size: 13px;
  }
</style>
