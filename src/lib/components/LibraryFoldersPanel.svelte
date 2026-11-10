<script lang="ts">
  import { onMount } from 'svelte';
  import type { SourceRoot } from '../api/bindings/SourceRoot';
  import { rootTextLine } from '../extractionCopy';
  import { rootLine } from '../inventoryCopy';
  import { extraction } from '../stores/extraction.svelte';
  import { embedding } from '../stores/embedding.svelte';
  import { rootMeaningLine } from '../embeddingCopy';
  import { inventory } from '../stores/inventory.svelte';
  import { folderName, sources } from '../stores/sources.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  // Lists library folders with live status and lets the user add more.
  // `removable` adds a Remove action with an inline confirmation.
  let { removable = false, primary = false }: { removable?: boolean; primary?: boolean } = $props();

  let confirmingId = $state<string | null>(null);

  // Availability is live (drives come and go), so re-check whenever the
  // list is shown.
  onMount(() => {
    sources.refresh();
  });

  function statusText(root: SourceRoot): string {
    if (root.nestedInRootId) {
      const outer = sources.displayName(root.nestedInRootId);
      return outer ? `Inside “${folderName(outer)}”` : 'Inside another folder';
    }
    switch (root.status) {
      case 'available':
        return 'Available';
      case 'unavailable':
        return 'Not connected';
      case 'no_access':
        return 'Atlas needs permission';
    }
  }

  async function confirmRemove(id: string) {
    confirmingId = null;
    await sources.remove(id);
  }
</script>

<div class="panel">
  {#if sources.roots.length > 0}
    <ul class="folders" aria-label="Library folders">
      {#each sources.roots as root (root.id)}
        <li class="folder">
          <div class="info">
            <span class="name">{folderName(root.displayPath)}</span>
            <span class="path">{root.displayPath}</span>
            {#if root.containsDataRoot}
              <span class="muted small">Atlas's own data folder inside it will be skipped.</span>
            {/if}
            {#if !root.nestedInRootId}
              <span class="muted small" data-testid="root-line">
                {rootLine(
                  inventory.overview?.roots.find((r) => r.sourceRootId === root.id),
                  inventory.overview,
                )}
              </span>
              {#if rootTextLine(extraction.overview, root.id)}
                <span class="muted small" data-testid="root-text-line">
                  {rootTextLine(extraction.overview, root.id)}
                </span>
              {/if}
              {#if rootMeaningLine(embedding.overview, root.id)}
                <span class="muted small" data-testid="root-meaning-line">
                  {rootMeaningLine(embedding.overview, root.id)}
                </span>
              {/if}
            {/if}
          </div>
          <span class="badge" data-status={root.nestedInRootId ? 'nested' : root.status}>
            {statusText(root)}
          </span>
          {#if !root.nestedInRootId && root.status === 'available'}
            <button
              type="button"
              class="button quiet"
              aria-label={`Rescan ${folderName(root.displayPath)}`}
              onclick={() => inventory.rescan(root.id)}
            >
              Rescan
            </button>
            <button
              type="button"
              class="button quiet"
              aria-label={`Read text now in ${folderName(root.displayPath)}`}
              onclick={() => extraction.readNow(root.id)}
            >
              Read text now
            </button>
            {#if embedding.overview?.model && !embedding.overview.needsPermission}
              <button
                type="button"
                class="button quiet"
                aria-label={`Index meaning now in ${folderName(root.displayPath)}`}
                onclick={() => embedding.indexNow(root.id)}
              >
                Index meaning now
              </button>
            {/if}
          {/if}
          {#if removable}
            {#if confirmingId === root.id}
              <div class="confirm" role="group" aria-label="Confirm removal">
                <span class="small">Remove from Atlas? Your files stay where they are.</span>
                <button type="button" class="button" onclick={() => confirmRemove(root.id)}>
                  Remove
                </button>
                <button type="button" class="button quiet" onclick={() => (confirmingId = null)}>
                  Keep
                </button>
              </div>
            {:else}
              <button
                type="button"
                class="button quiet"
                aria-label={`Remove ${folderName(root.displayPath)} from Atlas`}
                onclick={() => (confirmingId = root.id)}
              >
                Remove
              </button>
            {/if}
          {/if}
        </li>
      {/each}
    </ul>
  {:else}
    <p class="muted">Choose a folder to start discovering your library.</p>
  {/if}

  {#each sources.rejected as rejection (rejection.displayPath)}
    <ProblemNotice
      title={`Not added: ${folderName(rejection.displayPath)}`}
      message={rejection.error.message +
        (sources.displayName(rejection.conflictingRootId)
          ? ` (“${folderName(sources.displayName(rejection.conflictingRootId)!)}”)`
          : '')}
      recovery={rejection.error.recovery}
    />
  {/each}

  {#if sources.error}
    <ProblemNotice
      title="That didn't work"
      message={sources.error.message}
      recovery={sources.error.recovery}
    />
  {/if}

  <div>
    <button
      type="button"
      class="button"
      class:primary
      disabled={sources.busy}
      onclick={() => sources.addFromPicker()}
    >
      Add folders…
    </button>
  </div>
</div>

<style>
  .panel {
    display: grid;
    gap: var(--space-3);
  }

  .folders {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .folder {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .info {
    display: grid;
    flex: 1 1 280px;
    min-width: 0;
  }

  .name {
    font-weight: 600;
  }

  .path {
    color: var(--text-muted);
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .badge {
    padding: 2px var(--space-2);
    border-radius: 999px;
    background: var(--surface-muted);
    font-size: 12px;
    white-space: nowrap;
  }

  .badge[data-status='available'] {
    background: var(--accent-soft);
  }

  .badge[data-status='unavailable'],
  .badge[data-status='no_access'] {
    background: var(--notice-bg);
  }

  .confirm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .small {
    font-size: 13px;
  }
</style>
