<script lang="ts">
  // Settings › Diagnostics (Slice 4.1, phase4-spec §5.8): what a report
  // holds and never holds, Save a diagnostics report with Show in…, and
  // the log's size with Show the log folder and Clear the log. The
  // renderer sends only which folder to show.
  import { onMount } from 'svelte';
  import { atlas, AtlasApiError, toApiError } from '../api/client';
  import type { DiagnosticsFolder } from '../api/bindings/DiagnosticsFolder';
  import type { DiagnosticsStatus } from '../api/bindings/DiagnosticsStatus';
  import {
    CLEAR_LOG,
    CLEARING,
    DIAGNOSTICS_NOTE,
    LOG_STOPPED,
    logLine,
    REPORT_HOLDS,
    REPORT_NEVER_HOLDS,
    REPORT_SHARE_NOTE,
    SAVE_REPORT,
    SAVING,
    savedText,
    SHOW_LOG,
  } from '../diagnosticsCopy';
  import { revealLabel } from '../libraryCopy';
  import { appInfo } from '../stores/app.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  let status = $state<DiagnosticsStatus | null>(null);
  let saving = $state(false);
  let clearing = $state(false);
  let saved = $state<string | null>(null);
  let error = $state<AtlasApiError | null>(null);
  let errorTitle = $state('');
  const reveal = $derived(revealLabel(appInfo.status?.os));

  function fail(title: string, command: string, err: unknown) {
    errorTitle = title;
    error = err instanceof AtlasApiError ? err : toApiError(command, err);
  }

  async function load() {
    try {
      status = await atlas.getDiagnosticsStatus();
    } catch (err) {
      fail('Atlas couldn’t read its log', 'get_diagnostics_status', err);
    }
  }

  async function save() {
    saving = true;
    saved = null;
    error = null;
    try {
      const r = await atlas.saveDiagnosticsReport();
      saved = savedText(r);
      status = r.status;
    } catch (err) {
      fail('Atlas couldn’t save the report', 'save_diagnostics_report', err);
    } finally {
      saving = false;
    }
  }

  async function show(folder: DiagnosticsFolder) {
    error = null;
    try {
      await atlas.revealDiagnostics(folder);
    } catch (err) {
      fail('Atlas couldn’t show that folder', 'reveal_diagnostics', err);
    }
  }

  async function clear() {
    clearing = true;
    error = null;
    try {
      status = await atlas.clearLog();
    } catch (err) {
      fail('Atlas couldn’t clear the log', 'clear_log', err);
    } finally {
      clearing = false;
    }
  }

  onMount(() => {
    void load();
  });
</script>

<div class="diagnostics">
  <p>{DIAGNOSTICS_NOTE}</p>

  <div class="lists">
    <div>
      <h3 id="report-holds">A report holds</h3>
      <ul aria-labelledby="report-holds">
        {#each REPORT_HOLDS as item (item)}
          <li>{item}</li>
        {/each}
      </ul>
    </div>
    <div>
      <h3 id="report-never">It never holds</h3>
      <ul aria-labelledby="report-never">
        {#each REPORT_NEVER_HOLDS as item (item)}
          <li>{item}</li>
        {/each}
      </ul>
    </div>
  </div>
  <p class="muted small">{REPORT_SHARE_NOTE}</p>

  <div class="actions">
    <button type="button" class="button" disabled={saving} onclick={() => void save()}>
      {saving ? SAVING : SAVE_REPORT}
    </button>
    {#if saved}
      <span role="status" data-testid="report-saved">{saved}</span>
      <button type="button" class="button quiet" onclick={() => void show('reports')}>
        {reveal}
      </button>
    {/if}
  </div>

  {#if status}
    <p data-testid="log-size">{logLine(status)}</p>
    {#if status.logStopped}
      <p class="small" role="status">{LOG_STOPPED}</p>
    {/if}
    <div class="actions">
      <button type="button" class="button quiet" onclick={() => void show('log')}>
        {SHOW_LOG}
      </button>
      <button
        type="button"
        class="button"
        disabled={clearing || status.logFiles === 0}
        onclick={() => void clear()}
      >
        {clearing ? CLEARING : CLEAR_LOG}
      </button>
    </div>
  {/if}

  {#if error}
    <ProblemNotice title={errorTitle} message={error.message} recovery={error.recovery} />
  {/if}
</div>

<style>
  .diagnostics {
    display: grid;
    gap: var(--space-3);
  }

  p {
    margin: 0;
    line-height: 1.5;
  }

  h3 {
    margin: 0;
    font-size: 15px;
  }

  .lists {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: var(--space-3);
  }

  ul {
    margin: var(--space-1) 0 0;
    padding-left: var(--space-4);
    line-height: 1.5;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
  }

  .small {
    font-size: 13px;
  }
</style>
