<script lang="ts">
  import { onMount } from 'svelte';
  import { atlas } from '../api/client';
  import type { AppStatus } from '../api/bindings/AppStatus';
  import { count, elapsedText, statusLine } from '../inventoryCopy';
  import { inventory } from '../stores/inventory.svelte';

  let status = $state<AppStatus | null>(null);

  const active = $derived(inventory.overview?.active ?? null);
  const pausable = $derived(
    active !== null &&
      ['queued', 'running', 'paused_policy', 'failed_retryable'].includes(active.state),
  );
  const resumable = $derived(active?.state === 'paused_user');

  onMount(() => {
    atlas.getAppStatus().then(
      (result) => (status = result),
      () => (status = null),
    );
  });
</script>

<footer class="status" aria-label="Status">
  <div class="cataloging">
    <span role="status" data-testid="inventory-status">{statusLine(inventory.overview)}</span>
    {#if active}
      <details class="details">
        <summary>Details</summary>
        <dl>
          <div>
            <dt>Files found</dt>
            <dd>{active.discoveredFiles.toLocaleString('en-US')}</dd>
          </div>
          <div>
            <dt>Folders done</dt>
            <dd>{active.completedDirectories.toLocaleString('en-US')}</dd>
          </div>
          <div>
            <dt>Problems</dt>
            <dd>{active.failures.toLocaleString('en-US')}</dd>
          </div>
          <div>
            <dt>Links skipped</dt>
            <dd>{active.skippedSymlinks.toLocaleString('en-US')}</dd>
          </div>
          <div>
            <dt>Time spent</dt>
            <dd>{elapsedText(active.elapsedMs)}</dd>
          </div>
        </dl>
      </details>
      <div class="controls">
        {#if pausable}
          <button type="button" class="small-button" onclick={() => inventory.pause(active.jobId)}>
            Pause
          </button>
        {/if}
        {#if resumable}
          <button type="button" class="small-button" onclick={() => inventory.resume(active.jobId)}>
            Resume
          </button>
        {/if}
        <button
          type="button"
          class="small-button"
          title="Stop cataloging. Files found so far stay in your library."
          onclick={() => inventory.cancel(active.jobId)}
        >
          Cancel
        </button>
      </div>
    {:else if inventory.overview && inventory.overview.missingFiles > 0}
      <span class="muted">{count(inventory.overview.missingFiles, 'file')} missing</span>
    {/if}
  </div>
  {#if status}
    <span class="muted">
      Atlas {status.appVersion} · Phase {status.slice.split('.')[0]} slice {status.slice}{status.buildProfile ===
      'debug'
        ? ' · development build'
        : ''}
    </span>
  {/if}
</footer>

<style>
  .status {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-2) var(--space-5);
    border-top: 1px solid var(--border);
    background: var(--surface);
    font-size: 13px;
  }

  .cataloging {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .controls {
    display: flex;
    gap: var(--space-2);
  }

  .small-button {
    min-height: 28px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    font-size: 13px;
    cursor: pointer;
  }

  .details {
    position: relative;
  }

  .details summary {
    cursor: pointer;
    color: var(--text-muted);
  }

  .details dl {
    position: absolute;
    bottom: calc(100% + var(--space-2));
    left: 0;
    display: grid;
    gap: var(--space-1);
    min-width: 220px;
    margin: 0;
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: 0 4px 16px rgb(0 0 0 / 12%);
  }

  .details dl div {
    display: flex;
    justify-content: space-between;
    gap: var(--space-4);
  }

  .details dt {
    color: var(--text-muted);
  }

  .details dd {
    margin: 0;
  }
</style>
