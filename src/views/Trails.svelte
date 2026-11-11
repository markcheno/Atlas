<script lang="ts">
  // Trails (Slice 3.4, phase3-spec §5.1): every trail by last activity,
  // newest first, with a filter over titles, questions and searches, and
  // the open trail's timeline.
  import { onMount } from 'svelte';
  import ProblemNotice from '../lib/components/ProblemNotice.svelte';
  import SavedList from '../lib/components/SavedList.svelte';
  import CollectionList from '../lib/components/CollectionList.svelte';
  import { COLLECTIONS_TAB } from '../lib/collectionCopy';
  import { SAVED_TAB, TRAILS_TAB } from '../lib/savedCopy';
  import TrailTimeline from '../lib/components/TrailTimeline.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { count, relativeTime } from '../lib/inventoryCopy';
  import { trails } from '../lib/stores/trails.svelte';
  import { trailView, TRAILS_PAGE_SIZE } from '../lib/stores/trailView.svelte';
  import { noTrailsMatch, stepsText, trailDates, TRAILS_EMPTY } from '../lib/trailCopy';

  const page = $derived(trailView.result);
  const lastPage = $derived(page ? Math.max(0, Math.ceil(page.total / TRAILS_PAGE_SIZE) - 1) : 0);
  const range = $derived.by(() => {
    if (!page || page.total === 0) return '';
    const first = page.page * TRAILS_PAGE_SIZE + 1;
    const last = first + page.trails.length - 1;
    return `Showing ${first}–${last} of ${count(page.total, 'trail')}`;
  });

  onMount(() => {
    if (!trailView.selectedId) void trailView.load();
  });

  // A new trail, or a step on one, shows in the list.
  $effect(() => {
    if (trails.status && !trailView.selectedId) void trailView.load();
  });
</script>

<div class="view">
  <ViewHeading>Trails</ViewHeading>

  <fieldset class="tabs">
    <legend class="visually-hidden">Show</legend>
    <label>
      <input
        type="radio"
        name="trails-tab"
        value="trails"
        checked={trailView.tab === 'trails'}
        onchange={() => (trailView.tab = 'trails')}
      />
      {TRAILS_TAB}
    </label>
    <label>
      <input
        type="radio"
        name="trails-tab"
        value="saved"
        checked={trailView.tab === 'saved'}
        onchange={() => (trailView.tab = 'saved')}
      />
      {SAVED_TAB}
    </label>
    <label>
      <input
        type="radio"
        name="trails-tab"
        value="collections"
        checked={trailView.tab === 'collections'}
        onchange={() => (trailView.tab = 'collections')}
      />
      {COLLECTIONS_TAB}
    </label>
  </fieldset>

  {#if trailView.tab === 'saved'}
    <SavedList />
  {:else if trailView.tab === 'collections'}
    <CollectionList />
  {:else if trailView.selectedId}
    <TrailTimeline />
  {:else}
    {#if page && (page.total > 0 || trailView.text)}
      <div class="toolbar">
        <label class="filter">
          <span class="visually-hidden">Filter trails</span>
          <input
            type="search"
            placeholder="Filter trails"
            value={trailView.text}
            oninput={(e) => trailView.setText(e.currentTarget.value)}
            onkeydown={(e) => {
              if (e.key === 'Escape') trailView.setText('', true);
            }}
          />
        </label>
      </div>
    {/if}

    {#if trailView.error}
      <ProblemNotice
        title="Atlas couldn't list your trails"
        message={trailView.error.message}
        recovery={trailView.error.recovery}
      />
    {:else if !page}
      <p class="muted" role="status">Loading trails…</p>
    {:else if page.total === 0}
      <div class="empty">
        <p>{trailView.text ? noTrailsMatch(trailView.text) : TRAILS_EMPTY}</p>
      </div>
    {:else}
      <ol class="list" aria-label="Trails">
        {#each page.trails as t (t.id)}
          <li data-testid="trail-row">
            <div class="main">
              <button type="button" class="name-button" onclick={() => void trailView.open(t.id)}>
                {t.label}
              </button>
              <span class="muted small">
                {trailDates(t.startedAt, t.lastStepAt)} · {relativeTime(t.lastStepAt)} · {stepsText(
                  t.stepCount,
                )}
              </span>
            </div>
            <button
              type="button"
              class="button quiet"
              aria-pressed={t.pinned}
              aria-label={t.pinned ? `Pinned: ${t.label}` : `Pin ${t.label}`}
              onclick={() => void trailView.pinInList(t.id, !t.pinned)}
            >
              {t.pinned ? 'Pinned' : 'Pin'}
            </button>
          </li>
        {/each}
      </ol>
      {#if page.total > TRAILS_PAGE_SIZE}
        <nav class="pager" aria-label="Pages">
          <span class="muted" role="status" data-testid="trail-range">{range}</span>
          <div>
            <button
              type="button"
              class="button"
              disabled={trailView.page === 0}
              onclick={() => void trailView.goTo(trailView.page - 1)}
            >
              Previous
            </button>
            <button
              type="button"
              class="button"
              disabled={trailView.page >= lastPage}
              onclick={() => void trailView.goTo(trailView.page + 1)}
            >
              Next
            </button>
          </div>
        </nav>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .tabs {
    display: inline-flex;
    justify-self: start;
    margin: 0;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    min-height: 40px;
  }

  .tabs label {
    display: inline-flex;
    align-items: center;
    padding: 0 var(--space-3);
    border-radius: calc(var(--radius) - 2px);
    cursor: pointer;
  }

  .tabs label:has(input:checked) {
    background: var(--accent-soft);
    font-weight: 500;
  }

  .tabs label:has(input:focus-visible) {
    outline: 2px solid var(--focus);
    outline-offset: 1px;
  }

  .tabs input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .view {
    display: grid;
    gap: var(--space-4);
    max-width: 880px;
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-4);
  }

  .filter input {
    min-width: min(24rem, 100%);
    min-height: var(--control-height, 36px);
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .list {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .main {
    display: grid;
    gap: var(--space-1);
    min-width: 0;
  }

  .name-button {
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    font-weight: 500;
    text-align: left;
    overflow-wrap: anywhere;
    cursor: pointer;
  }

  .name-button:hover {
    text-decoration: underline;
  }

  .empty {
    padding: var(--space-6);
    border: 1px dashed var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  .empty p {
    margin: 0;
  }

  .pager {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .pager div {
    display: flex;
    gap: var(--space-2);
  }

  .small {
    font-size: 13px;
  }
</style>
