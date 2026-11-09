<script lang="ts">
  // Open and Reveal for one original file (Library rows, search results,
  // the detail panel).
  import { revealLabel } from '../libraryCopy';
  import { appInfo } from '../stores/app.svelte';
  import { originals, type Original } from '../stores/originals.svelte';

  let { item }: { item: Original } = $props();
  const reveal = $derived(revealLabel(appInfo.status?.os));
</script>

<span class="actions">
  <button
    type="button"
    class="row-button"
    disabled={item.availability !== 'available'}
    aria-label={`Open ${item.displayName}`}
    onclick={() => originals.open(item)}
  >
    Open
  </button>
  <button
    type="button"
    class="row-button"
    disabled={item.availability === 'unreachable'}
    aria-label={`${reveal}: ${item.displayName}`}
    onclick={() => originals.reveal(item)}
  >
    {reveal}
  </button>
</span>

<style>
  .actions {
    display: inline-flex;
    gap: var(--space-2);
    white-space: nowrap;
  }
</style>
