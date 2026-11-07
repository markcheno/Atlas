<script lang="ts">
  import LibraryFoldersPanel from '../lib/components/LibraryFoldersPanel.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
</script>

<div class="view">
  <ViewHeading>What are you curious about?</ViewHeading>

  <div class="search">
    <label for="home-search">Search filenames</label>
    <input
      id="home-search"
      type="search"
      placeholder="Filename search is coming soon"
      disabled
      aria-describedby="home-search-help"
    />
    <p id="home-search-help" class="muted small">
      Phase 0 matches file names and paths only. This is not AI search.
    </p>
  </div>

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

  input:disabled {
    background: var(--surface-muted);
    cursor: not-allowed;
  }

  section {
    display: grid;
    gap: var(--space-3);
  }

  .small {
    font-size: 13px;
  }
</style>
