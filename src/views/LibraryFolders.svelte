<script lang="ts">
  import LibraryFoldersPanel from '../lib/components/LibraryFoldersPanel.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { sources } from '../lib/stores/sources.svelte';
  import { view } from '../lib/stores/view.svelte';
</script>

<main id="main" class="screen">
  <div class="card">
    <ViewHeading>Add the folders where you keep your books and papers</ViewHeading>
    <p class="lede">
      Atlas only reads these folders. It never moves, renames, or changes your files, and you don't
      need to tag or reorganize anything.
    </p>

    <LibraryFoldersPanel removable primary={sources.roots.length === 0} />

    <div class="actions">
      <button
        type="button"
        class="button"
        class:primary={sources.roots.length > 0}
        disabled={sources.roots.length === 0}
        onclick={() => view.go('device-check')}
      >
        Continue
      </button>
      <button type="button" class="button quiet" onclick={() => view.go('device-check')}>
        Skip for now
      </button>
    </div>
  </div>
</main>

<style>
  .screen {
    display: grid;
    place-items: center;
    min-height: 100%;
    padding: var(--space-6);
  }

  .card {
    display: grid;
    gap: var(--space-5);
    width: min(680px, 100%);
  }

  .lede {
    font-size: 17px;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }
</style>
