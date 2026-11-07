<script lang="ts">
  import { onMount } from 'svelte';
  import { atlas } from '../api/client';
  import type { AppStatus } from '../api/bindings/AppStatus';

  let status = $state<AppStatus | null>(null);

  onMount(() => {
    atlas.getAppStatus().then(
      (result) => (status = result),
      () => (status = null),
    );
  });
</script>

<footer class="status" aria-label="Status">
  <span role="status">Not cataloging yet</span>
  {#if status}
    <span class="muted">
      Atlas {status.appVersion} · Phase 0 slice {status.slice}{status.buildProfile === 'debug'
        ? ' · development build'
        : ''}
    </span>
  {/if}
</footer>

<style>
  .status {
    display: flex;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-2) var(--space-5);
    border-top: 1px solid var(--border);
    background: var(--surface);
    font-size: 13px;
  }
</style>
