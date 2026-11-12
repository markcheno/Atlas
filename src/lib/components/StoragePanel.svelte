<script lang="ts">
  // Settings › Storage (phase1-spec §5.5): what the library's data takes by
  // category, the estimate for the text still to be read, and Clean caches.
  // Your trails and notes and their snapshots (Slice 3.2) are shown apart:
  // Clean caches never touches them.
  import { onMount } from 'svelte';
  import { atlas, AtlasApiError, toApiError } from '../api/client';
  import type { StorageUsage } from '../api/bindings/StorageUsage';
  import type { KnowledgeStatus } from '../api/bindings/KnowledgeStatus';
  import { formatBytes } from '../format';
  import {
    estimateText,
    meaningEstimateText,
    freedText,
    knowledgeText,
    snapshotsLabel,
  } from '../storageCopy';
  import ProblemNotice from './ProblemNotice.svelte';

  let usage = $state<StorageUsage | null>(null);
  let knowledge = $state<KnowledgeStatus | null>(null);
  let error = $state<AtlasApiError | null>(null);
  let cleaning = $state(false);
  let cleaned = $state<string | null>(null);

  const asError = (command: string, err: unknown) =>
    err instanceof AtlasApiError ? err : toApiError(command, err);

  async function readKnowledge() {
    try {
      knowledge = await atlas.getKnowledgeStatus();
    } catch {
      // The sizes below still show; the sentence needs the status.
      knowledge = null;
    }
  }

  async function measure() {
    void readKnowledge();
    try {
      usage = await atlas.getStorageUsage();
      error = null;
    } catch (err) {
      error = asError('get_storage_usage', err);
    }
  }

  async function clean() {
    cleaning = true;
    cleaned = null;
    try {
      const result = await atlas.cleanCaches();
      usage = result.usage;
      cleaned = freedText(result.freedBytes);
      error = null;
    } catch (err) {
      error = asError('clean_caches', err);
    } finally {
      cleaning = false;
    }
  }

  onMount(() => {
    void measure();
  });
</script>

<div class="storage" data-testid="storage-usage">
  {#if error}
    <ProblemNotice
      title="Atlas couldn't measure its storage"
      message={error.message}
      recovery={error.recovery}
    />
  {/if}
  {#if usage}
    <dl>
      <div>
        <dt>Text Atlas has read</dt>
        <dd>{formatBytes(usage.textBytes)}</dd>
      </div>
      <div>
        <dt>Search index</dt>
        <dd>{formatBytes(usage.indexBytes)}</dd>
      </div>
      {#if usage.meaningCatalogBytes + usage.meaningFileBytes > 0 || usage.meaningEstimate}
        <div>
          <dt>Meaning index</dt>
          <dd data-testid="meaning-bytes">
            {formatBytes(usage.meaningCatalogBytes + usage.meaningFileBytes)}
          </dd>
        </div>
      {/if}
      <div>
        <dt>Rest of the catalog</dt>
        <dd>{formatBytes(usage.otherCatalogBytes)}</dd>
      </div>
      <div>
        <dt>Caches</dt>
        <dd>{formatBytes(usage.cacheBytes)}</dd>
      </div>
      <div>
        <dt>Your trails and notes</dt>
        <dd data-testid="knowledge-bytes">{formatBytes(usage.knowledgeBytes)}</dd>
      </div>
      <div>
        <dt>{snapshotsLabel(usage.snapshotCount)}</dt>
        <dd data-testid="snapshot-bytes">{formatBytes(usage.snapshotBytes)}</dd>
      </div>
      <div>
        <dt>The log</dt>
        <dd data-testid="log-bytes">{formatBytes(usage.logBytes)}</dd>
      </div>
    </dl>
    {#if knowledgeText(knowledge)}
      <p data-testid="knowledge-note">{knowledgeText(knowledge)}</p>
    {/if}
    <p data-testid="indexing-estimate">
      {estimateText(usage.estimate, usage.dataVolumeFreeBytes)}
    </p>
    {#if meaningEstimateText(usage.meaningEstimate)}
      <p data-testid="meaning-estimate">{meaningEstimateText(usage.meaningEstimate)}</p>
    {/if}
  {:else if !error}
    <p class="muted" role="status">Measuring…</p>
  {/if}
  <div class="clean">
    <button type="button" class="button" disabled={cleaning || !usage} onclick={clean}>
      {cleaning ? 'Cleaning…' : 'Clean caches'}
    </button>
    <span role="status" data-testid="clean-result">{cleaned ?? ''}</span>
  </div>
  <p class="muted small">
    Clean caches removes only temporary files Atlas can make again. It never removes your catalog,
    the text Atlas has read, the meaning index, your settings, the log, or your trails and notes and
    their snapshots.
  </p>
</div>

<style>
  .storage {
    display: grid;
    gap: var(--space-3);
  }

  dl {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: var(--space-3);
    margin: 0;
  }

  dt {
    color: var(--text-muted);
    font-size: 13px;
  }

  dd {
    margin: 0;
    font-weight: 500;
  }

  p {
    margin: 0;
  }

  .clean {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .small {
    font-size: 13px;
  }
</style>
