<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { atlas } from '../api/client';
  import type { AppStatus } from '../api/bindings/AppStatus';
  import { activityText, count, elapsedText, statusLine } from '../inventoryCopy';
  import { folderName, sources } from '../stores/sources.svelte';
  import { extractionLine } from '../extractionCopy';
  import { meaningLine, stoppedFolders } from '../embeddingCopy';
  import { embedding } from '../stores/embedding.svelte';
  import { view } from '../stores/view.svelte';
  import { extraction } from '../stores/extraction.svelte';
  import { inventory } from '../stores/inventory.svelte';
  import { trails } from '../stores/trails.svelte';
  import { TRAIL_FAILED, trailLabel, trailToggleLabel } from '../trailCopy';

  let status = $state<AppStatus | null>(null);

  const active = $derived(inventory.overview?.active ?? null);
  const pausable = $derived(
    active !== null &&
      ['queued', 'running', 'paused_policy', 'failed_retryable'].includes(active.state),
  );
  const resumable = $derived(active?.state === 'paused_user');

  const reading = $derived(extraction.overview?.active ?? null);
  const readingLine = $derived(extractionLine(extraction.overview));
  const readingPausable = $derived(
    reading !== null &&
      ['queued', 'running', 'paused_policy', 'failed_retryable'].includes(reading.state),
  );

  const meaning = $derived(embedding.overview?.active ?? null);
  const meaningStatus = $derived(meaningLine(embedding.overview));
  const meaningPausable = $derived(
    meaning !== null && ['queued', 'running', 'paused_policy'].includes(meaning.state),
  );

  /** Index meaning now, in every folder whose job stopped for good. */
  function indexStopped() {
    for (const id of stoppedFolders(embedding.overview)) void embedding.indexNow(id);
  }

  /**
   * Every folder with something going on, in cataloging, reading or
   * meaning (Slice 4.5): the lines above show one job each.
   */
  const activity = $derived.by(() => {
    // Built afresh on every change, so a plain record (in first-seen order).
    const rows: Record<string, string[]> = {};
    const add = (id: string, text: string) => (rows[id] = [...(rows[id] ?? []), text]);
    for (const j of inventory.overview?.activeJobs ?? [])
      add(
        j.sourceRootId,
        activityText('catalog', j.state, { origin: j.origin, pauseReason: j.pauseReason }),
      );
    for (const r of extraction.overview?.roots ?? [])
      if (r.activeState) add(r.sourceRootId, activityText('read', r.activeState));
    for (const r of embedding.overview?.roots ?? [])
      if (r.activeState) add(r.sourceRootId, activityText('meaning', r.activeState));
    return Object.entries(rows).map(([id, parts]) => ({
      id,
      name: folderName(
        sources.displayName(id) ??
          inventory.overview?.activeJobs.find((j) => j.sourceRootId === id)
            ?.sourceRootDisplayPath ??
          'A folder',
      ),
      text: parts.join(' · '),
    }));
  });

  async function openStorage() {
    view.go('settings');
    await tick();
    document.getElementById('settings-storage')?.scrollIntoView?.({ block: 'start' });
  }

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
          <button
            type="button"
            class="small-button"
            aria-label="Pause cataloging"
            onclick={() => inventory.pause(active.jobId)}
          >
            Pause
          </button>
        {/if}
        {#if resumable}
          <button
            type="button"
            class="small-button"
            aria-label="Resume cataloging"
            onclick={() => inventory.resume(active.jobId)}
          >
            Resume
          </button>
        {/if}
        <button
          type="button"
          class="small-button"
          aria-label="Cancel cataloging"
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
  {#if readingLine}
    <div class="cataloging">
      <span role="status" data-testid="extraction-status">{readingLine}</span>
      {#if reading}
        <div class="controls">
          {#if readingPausable}
            <button
              type="button"
              class="small-button"
              aria-label="Pause reading text"
              onclick={() => extraction.pause(reading.jobId)}
            >
              Pause
            </button>
          {/if}
          {#if reading.state === 'paused_user'}
            <button
              type="button"
              class="small-button"
              aria-label="Resume reading text"
              onclick={() => extraction.resume(reading.jobId)}
            >
              Resume
            </button>
          {/if}
          <button
            type="button"
            class="small-button"
            aria-label="Cancel reading text"
            title="Stop reading text. Text already read stays in your library."
            onclick={() => extraction.cancel(reading.jobId)}
          >
            Cancel
          </button>
        </div>
      {/if}
    </div>
  {/if}
  {#if meaningStatus}
    <div class="cataloging">
      <span role="status" data-testid="embedding-status">{meaningStatus.text}</span>
      <div class="controls">
        {#if meaningStatus.action === 'retry' && meaning}
          <button
            type="button"
            class="small-button"
            aria-label="Retry indexing meaning"
            onclick={() => embedding.resume(meaning.jobId)}
          >
            Retry
          </button>
        {:else if meaningStatus.action === 'choose_model' || meaningStatus.action === 'allow_remote'}
          <button type="button" class="small-button" onclick={() => view.go('settings')}>
            Open Settings
          </button>
        {:else if meaningStatus.action === 'index_now'}
          <button type="button" class="small-button" onclick={indexStopped}>
            Index meaning now
          </button>
        {/if}
        {#if meaning && meaningPausable}
          <button
            type="button"
            class="small-button"
            aria-label="Pause indexing meaning"
            onclick={() => embedding.pause(meaning.jobId)}
          >
            Pause
          </button>
        {/if}
        {#if meaning?.state === 'paused_user'}
          <button
            type="button"
            class="small-button"
            aria-label="Resume indexing meaning"
            onclick={() => embedding.resume(meaning.jobId)}
          >
            Resume
          </button>
        {/if}
        {#if meaning}
          <button
            type="button"
            class="small-button"
            aria-label="Cancel indexing meaning"
            title="Stop indexing meaning. Files already indexed stay indexed."
            onclick={() => embedding.cancel(meaning.jobId)}
          >
            Cancel
          </button>
        {/if}
      </div>
    </div>
  {/if}
  {#if activity.length > 1 || (activity.length === 1 && activity[0].text.includes(' · '))}
    <details class="details activity" data-testid="all-activity">
      <summary>All activity</summary>
      <ul aria-label="Every folder's activity">
        {#each activity as row (row.id)}
          <li><span class="folder-name">{row.name}</span> {row.text}</li>
        {/each}
      </ul>
    </details>
  {/if}
  {#if trails.status}
    <div class="cataloging" data-testid="trail-indicator">
      {#if trails.status.state === 'unavailable'}
        <button
          type="button"
          class="small-button"
          title="Settings › Storage says what happened"
          onclick={() => void openStorage()}
        >
          {trailLabel(trails.status)}
        </button>
      {:else}
        <button
          type="button"
          class="small-button"
          aria-pressed={trails.recording}
          title={trailToggleLabel(trails.status)}
          onclick={() => void trails.toggle()}
        >
          {trailLabel(trails.status)}
        </button>
      {/if}
      {#if trails.status.failed}
        <span role="status" data-testid="trail-failed">{TRAIL_FAILED}</span>
      {/if}
    </div>
  {/if}
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

  .activity ul {
    position: absolute;
    bottom: calc(100% + var(--space-2));
    left: 0;
    display: grid;
    gap: var(--space-1);
    min-width: 280px;
    margin: 0;
    padding: var(--space-3) var(--space-4);
    list-style: none;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: 0 4px 16px rgb(0 0 0 / 12%);
  }

  .folder-name {
    font-weight: 600;
  }

  .folder-name::after {
    content: ':';
  }
</style>
