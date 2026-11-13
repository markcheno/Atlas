<script lang="ts">
  import { onMount } from 'svelte';
  import type { SourceRoot } from '../api/bindings/SourceRoot';
  import { rootTextLine } from '../extractionCopy';
  import { awayText, lastCheckLine, rootLine } from '../inventoryCopy';
  import { appInfo } from '../stores/app.svelte';
  import FolderChecks from './FolderChecks.svelte';
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
  /** Relative times ("Checked 2 h ago") move on while the list is open. */
  let now = $state(Date.now());
  let lastSourcesRefresh = 0;

  // Availability is live (drives come and go), so re-check whenever the
  // list is shown, and again (at most every 5 s) when cataloging reports a
  // change, as the folder check does when a drive comes or goes.
  onMount(() => {
    sources.refresh();
    lastSourcesRefresh = Date.now();
    const tick = setInterval(() => (now = Date.now()), 60_000);
    return () => clearInterval(tick);
  });
  $effect(() => {
    void inventory.overview;
    if (Date.now() - lastSourcesRefresh < 5_000) return;
    lastSourcesRefresh = Date.now();
    void sources.refresh();
  });

  /** A symbol beside the word, so the state is never colour alone. */
  function statusText(root: SourceRoot): string {
    if (root.nestedInRootId) {
      const outer = sources.displayName(root.nestedInRootId);
      return outer ? `↳ Inside “${folderName(outer)}”` : '↳ Inside another folder';
    }
    switch (root.status) {
      case 'available':
        return '● Available';
      case 'unavailable':
        return '○ Not connected';
      case 'no_access':
        return '⚠ Atlas needs permission';
    }
  }

  function summaryFor(id: string) {
    return inventory.overview?.roots.find((r) => r.sourceRootId === id);
  }

  function activeFor(id: string) {
    return inventory.overview?.activeJobs.find((j) => j.sourceRootId === id) ?? null;
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
                {rootLine(summaryFor(root.id), inventory.overview, now)}
              </span>
              {#if awayText(root.status, appInfo.status?.os)}
                <span class="small" data-testid="root-away-line">
                  {awayText(root.status, appInfo.status?.os)}
                </span>
              {:else if !activeFor(root.id) && lastCheckLine(summaryFor(root.id))}
                <span class="muted small" data-testid="root-check-line">
                  {lastCheckLine(summaryFor(root.id))}
                </span>
              {/if}
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
          {#if !root.nestedInRootId && activeFor(root.id)}
            {@const job = activeFor(root.id)!}
            {#if job.state === 'paused_user'}
              <button
                type="button"
                class="button quiet"
                aria-label={`Resume cataloging ${folderName(root.displayPath)}`}
                onclick={() => inventory.resume(job.jobId)}
              >
                Resume
              </button>
            {:else}
              <button
                type="button"
                class="button quiet"
                aria-label={`Pause cataloging ${folderName(root.displayPath)}`}
                onclick={() => inventory.pause(job.jobId)}
              >
                Pause
              </button>
            {/if}
            <button
              type="button"
              class="button quiet"
              aria-label={`Cancel cataloging ${folderName(root.displayPath)}`}
              title="Stop cataloging this folder. Files found so far stay in your library."
              onclick={() => inventory.cancel(job.jobId)}
            >
              Cancel
            </button>
          {/if}
          {#if !root.nestedInRootId && root.status === 'available'}
            {#if !activeFor(root.id)}
              <button
                type="button"
                class="button quiet"
                aria-label={`Rescan ${folderName(root.displayPath)}`}
                onclick={() => inventory.rescan(root.id)}
              >
                Rescan
              </button>
            {/if}
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
          {#if !root.nestedInRootId}
            <div class="checks">
              <FolderChecks
                summary={summaryFor(root.id)}
                name={folderName(root.displayPath)}
                {now}
              />
            </div>
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

  {#each [inventory.actionError, extraction.actionError, embedding.actionError] as problem, i (i)}
    {#if problem}
      <ProblemNotice
        title="That didn't work"
        message={problem.message}
        recovery={problem.recovery}
      />
    {/if}
  {/each}

  {#if sources.error}
    <ProblemNotice
      title="That didn't work"
      message={sources.error.message}
      recovery={sources.error.recovery}
    />
  {/if}

  <div class="actions">
    <button
      type="button"
      class="button"
      class:primary
      disabled={sources.busy}
      onclick={() => sources.addFromPicker()}
    >
      Add folders…
    </button>
    {#if sources.roots.some((r) => !r.nestedInRootId)}
      <button type="button" class="button quiet" onclick={() => inventory.checkAll()}>
        Check all folders now
      </button>
    {/if}
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

  .checks {
    flex: 1 1 100%;
    display: grid;
    gap: var(--space-1);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
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
