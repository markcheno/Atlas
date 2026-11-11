<script lang="ts">
  import LibraryFoldersPanel from '../lib/components/LibraryFoldersPanel.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { homeReadingNote, SEARCH_HELP } from '../lib/searchCopy';
  import { extraction } from '../lib/stores/extraction.svelte';
  import { embedding } from '../lib/stores/embedding.svelte';
  import { homeMeaningNote } from '../lib/embeddingCopy';
  import { search } from '../lib/stores/search.svelte';
  import { trails } from '../lib/stores/trails.svelte';
  import { FIRST_STEP_NOTE } from '../lib/trailCopy';
  import { view } from '../lib/stores/view.svelte';

  let text = $state('');
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
