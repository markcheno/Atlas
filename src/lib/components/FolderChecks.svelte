<script lang="ts">
  // A folder's recent checks and what its last check couldn't read
  // (Slice 4.5, phase4-spec §5.3). Loaded when opened, and again when the
  // folder's last check changes.
  import { atlas, AtlasApiError, toApiError } from '../api/client';
  import type { RootInventorySummary } from '../api/bindings/RootInventorySummary';
  import type { ScanProblem } from '../api/bindings/ScanProblem';
  import type { ScanRecord } from '../api/bindings/ScanRecord';
  import { problemText, scanRecordText } from '../inventoryCopy';
  import { appInfo } from '../stores/app.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  let {
    summary,
    name,
    now,
  }: { summary: RootInventorySummary | undefined; name: string; now: number } = $props();

  let historyOpen = $state(false);
  let problemsOpen = $state(false);
  let history = $state<ScanRecord[] | null>(null);
  let problems = $state<ScanProblem[] | null>(null);
  let error = $state<AtlasApiError | null>(null);

  const lastJob = $derived(summary?.lastScan?.jobId ?? null);
  const problemCount = $derived(summary?.lastScan?.problems ?? 0);

  async function loadHistory() {
    if (!summary) return;
    try {
      history = await atlas.listScans(summary.sourceRootId, 10);
    } catch (err) {
      error = err instanceof AtlasApiError ? err : toApiError('list_scans', err);
    }
  }

  async function loadProblems() {
    if (!lastJob) return;
    try {
      problems = await atlas.getScanProblems(lastJob);
    } catch (err) {
      error = err instanceof AtlasApiError ? err : toApiError('get_scan_problems', err);
    }
  }

  // A new check ended: what is shown is out of date.
  $effect(() => {
    void lastJob;
    if (historyOpen) void loadHistory();
    if (problemsOpen) void loadProblems();
  });
</script>

{#if summary?.lastScan}
  <details
    class="checks"
    ontoggle={(e) => {
      historyOpen = (e.currentTarget as HTMLDetailsElement).open;
      if (historyOpen) void loadHistory();
    }}
  >
    <summary aria-label={`Recent checks of ${name}`}>Recent checks</summary>
    {#if history}
      <ol class="history" aria-label={`Recent checks of ${name}`}>
        {#each history as record (record.jobId)}
          <li>{scanRecordText(record, now)}</li>
        {/each}
      </ol>
    {:else}
      <p class="muted small">Loading…</p>
    {/if}
  </details>
{/if}

{#if problemCount > 0}
  <details
    class="checks"
    ontoggle={(e) => {
      problemsOpen = (e.currentTarget as HTMLDetailsElement).open;
      if (problemsOpen) void loadProblems();
    }}
  >
    <summary aria-label={`Folders Atlas couldn't read in ${name}`}>
      Couldn't read {problemCount === 1 ? '1 folder' : `${problemCount} folders`}
    </summary>
    {#if problems}
      <ul class="problems">
        {#each problems as problem, i (i)}
          {@const text = problemText(problem.code, appInfo.status?.os)}
          <li>
            <span class="path">{problem.displayPath}</span>
            <span class="small">{text.why}. {text.fix}</span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted small">Loading…</p>
    {/if}
  </details>
{/if}

{#if error}
  <ProblemNotice
    title="Atlas couldn’t show the folder’s checks"
    message={error.message}
    recovery={error.recovery}
  />
{/if}

<style>
  .checks {
    font-size: 13px;
  }

  summary {
    cursor: pointer;
    color: var(--text-muted);
    width: fit-content;
  }

  summary:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }

  .history,
  .problems {
    margin: var(--space-1) 0 0;
    padding-left: var(--space-4);
    display: grid;
    gap: var(--space-1);
  }

  .problems li {
    display: grid;
  }

  .path {
    font-family: ui-monospace, Menlo, Consolas, monospace;
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .small {
    font-size: 12px;
  }

  .muted {
    color: var(--text-muted);
  }
</style>
