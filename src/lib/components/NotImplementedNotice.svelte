<script lang="ts">
  import type { AtlasApiError } from '../api/client';

  // Shows honestly that a capability has not shipped yet, using the slice
  // reported by the Rust core when available.
  let {
    title,
    error = null,
    plannedSlice = undefined,
  }: { title: string; error?: AtlasApiError | null; plannedSlice?: string } = $props();

  const slice = $derived(error?.plannedSlice ?? plannedSlice);
</script>

<div class="notice" role="note">
  <p class="title">{title}</p>
  {#if error && !error.isNotImplemented}
    <p class="muted">{error.message}</p>
    {#if error.recovery}<p class="muted">{error.recovery}</p>{/if}
  {:else}
    <p class="muted">
      Not available in this build yet{slice ? `; planned for slice ${slice}` : ''}.
    </p>
  {/if}
</div>

<style>
  .notice {
    display: grid;
    gap: var(--space-1);
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--notice-border);
    border-radius: var(--radius);
    background: var(--notice-bg);
  }

  .title {
    font-weight: 600;
  }
</style>
