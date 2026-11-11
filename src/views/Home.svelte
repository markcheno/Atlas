<script lang="ts">
  import { atlas } from '../lib/api/client';
  import type { TrailSummary } from '../lib/api/bindings/TrailSummary';
  import LibraryFoldersPanel from '../lib/components/LibraryFoldersPanel.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { homeReadingNote, SEARCH_HELP } from '../lib/searchCopy';
  import { extraction } from '../lib/stores/extraction.svelte';
  import { embedding } from '../lib/stores/embedding.svelte';
  import { homeMeaningNote } from '../lib/embeddingCopy';
  import { search } from '../lib/stores/search.svelte';
  import { trails } from '../lib/stores/trails.svelte';
  import { trailView } from '../lib/stores/trailView.svelte';
  import { relativeTime } from '../lib/inventoryCopy';
  import { FIRST_STEP_NOTE, PICK_UP_TITLE, stepsText } from '../lib/trailCopy';
  import { view } from '../lib/stores/view.svelte';

  let text = $state('');
  /** The three most recent trails (phase3-spec §5.6). */
  let recent = $state<TrailSummary[]>([]);

  async function loadRecent() {
    try {
      recent = (await atlas.listTrails({ filter: '', page: 0, pageSize: 3 })).trails;
    } catch {
      recent = [];
    }
  }

  // Loaded at first, then whenever a step is recorded or trails change.
  $effect(() => {
    void trails.status;
    void loadRecent();
  });

  async function pickUp(id: string) {
    view.go('trails');
    await trailView.open(id);
  }
  const note = $derived(homeReadingNote(extraction.overview));
  const meaningNote = $derived(homeMeaningNote(embedding.overview));

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void search.submit(text);
    view.go('search');
  }
</script>

<div class="view">
  <ViewHeading>What are you curious about?</ViewHeading>

  <form class="search" role="search" onsubmit={submit}>
    <label for="home-search">Search your library</label>
    <input
      id="home-search"
      type="search"
      placeholder="For example: meteor, “harmonic drive”, café"
      bind:value={text}
      aria-describedby="home-search-help"
    />
    <p id="home-search-help" class="muted small">{SEARCH_HELP}</p>
    {#if note}
      <p class="small" data-testid="reading-note">{note}</p>
    {/if}
    {#if meaningNote}
      <p class="small" data-testid="meaning-note">{meaningNote}</p>
    {/if}
  </form>

  {#if trails.status?.firstStepNote}
    <aside class="note" aria-label="About trails" data-testid="trail-note">
      <p class="small">{FIRST_STEP_NOTE}</p>
      <button type="button" class="button quiet" onclick={() => void trails.dismissNote()}>
        Got it
      </button>
    </aside>
  {/if}

  {#if recent.length > 0}
    <section aria-labelledby="home-trails">
      <h2 id="home-trails">{PICK_UP_TITLE}</h2>
      <ul class="recent">
        {#each recent as t (t.id)}
          <li data-testid="recent-trail">
            <button type="button" class="name-button" onclick={() => void pickUp(t.id)}>
              {t.label}
            </button>
            <span class="muted small">{relativeTime(t.lastStepAt)} · {stepsText(t.stepCount)}</span>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <section aria-labelledby="home-library">
    <h2 id="home-library">Your library</h2>
    <LibraryFoldersPanel />
  </section>
</div>

<style>
  .view {
    display: grid;
    gap: var(--space-6);
    max-width: 720px;
    margin: 0 auto;
  }

  .search {
    display: grid;
    gap: var(--space-2);
  }

  label {
    font-weight: 500;
  }

  input {
    min-height: 52px;
    padding: 0 var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    font-size: 17px;
  }

  section {
    display: grid;
    gap: var(--space-3);
  }

  .small {
    font-size: 13px;
  }

  .recent {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .recent li {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-1) var(--space-3);
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
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

  .note {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .note p {
    margin: 0;
  }
</style>
