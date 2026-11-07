<script lang="ts">
  import { onMount } from 'svelte';
  import { atlas, AtlasApiError, toApiError } from '../lib/api/client';
  import type { DataRootCandidate } from '../lib/api/bindings/DataRootCandidate';
  import { applyStatus } from '../lib/boot';
  import ProblemNotice from '../lib/components/ProblemNotice.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { KIND_TEXT, problemText } from '../lib/dataRootCopy';
  import { formatBytes } from '../lib/format';
  import { view } from '../lib/stores/view.svelte';
  import { workspace } from '../lib/stores/workspace.svelte';

  let candidate = $state<DataRootCandidate | null>(null);
  let error = $state<AtlasApiError | null>(null);
  let busy = $state(false);

  // Reached from Settings or the "library unavailable" screen as well as
  // from onboarding; offer a way back in those cases.
  const backTarget = $derived(
    workspace.status === null || workspace.status.state === 'not_configured'
      ? null
      : workspace.isOpen
        ? 'settings'
        : 'unavailable',
  );

  function asApiError(command: string, err: unknown): AtlasApiError {
    return err instanceof AtlasApiError ? err : toApiError(command, err);
  }

  async function inspect(path: string) {
    error = null;
    try {
      candidate = await atlas.inspectDataRoot(path);
    } catch (err) {
      error = asApiError('inspect_data_root', err);
    }
  }

  async function chooseAnother() {
    error = null;
    try {
      const picked = await atlas.pickFolder();
      if (picked) await inspect(picked);
    } catch (err) {
      error = asApiError('pick_folder', err);
    }
  }

  async function useLocation() {
    if (!candidate) return;
    busy = true;
    error = null;
    try {
      await applyStatus(await atlas.chooseDataRoot(candidate.displayPath));
    } catch (err) {
      error = asApiError('choose_data_root', err);
    } finally {
      busy = false;
    }
  }

  onMount(async () => {
    try {
      await inspect(await atlas.getDefaultDataRoot());
    } catch (err) {
      error = asApiError('get_default_data_root', err);
    }
  });
</script>

<main id="main" class="screen">
  <div class="card">
    <ViewHeading>Choose where Atlas keeps its library</ViewHeading>
    <p class="lede">
      Atlas stores its catalog and generated data here. Your books stay where they are.
    </p>

    {#if candidate}
      <section class="location" aria-labelledby="location-heading">
        <h2 id="location-heading" class="visually-hidden">Selected location</h2>
        <p class="path" data-testid="candidate-path">{candidate.displayPath}</p>
        <p>{KIND_TEXT[candidate.kind]}</p>
        <dl>
          <div>
            <dt>Free space</dt>
            <dd>{formatBytes(candidate.freeBytes)} of {formatBytes(candidate.totalBytes)}</dd>
          </div>
          <div>
            <dt>Can write here</dt>
            <dd>{candidate.problems.includes('not_writable') ? 'No' : 'Yes'}</dd>
          </div>
        </dl>
      </section>

      {#if candidate.problems.length > 0}
        <ProblemNotice title="Atlas can't use this location">
          {#snippet actions()}
            <ul class="problems">
              {#each candidate!.problems as problem (problem)}
                <li>{problemText(problem, candidate!)}</li>
              {/each}
            </ul>
          {/snippet}
        </ProblemNotice>
      {:else if candidate.kind === 'non_empty_folder' && candidate.suggestedSubfolder}
        <ProblemNotice title="Use a new folder inside it instead?">
          {#snippet actions()}
            <button
              type="button"
              class="button"
              onclick={() => inspect(candidate!.suggestedSubfolder!)}
            >
              Create an “Atlas Library” folder here
            </button>
          {/snippet}
        </ProblemNotice>
      {/if}
    {:else if !error}
      <p class="muted" role="status">Checking the suggested location…</p>
    {/if}

    {#if error}
      <ProblemNotice title="That didn't work" message={error.message} recovery={error.recovery} />
    {/if}

    <div class="actions">
      <button
        type="button"
        class="button primary"
        disabled={!candidate?.canUse || busy}
        onclick={useLocation}
      >
        {candidate?.kind === 'existing_library' ? 'Open this library' : 'Use this location'}
      </button>
      <button type="button" class="button" onclick={chooseAnother} disabled={busy}>
        Choose another location…
      </button>
      {#if backTarget}
        <button type="button" class="button quiet" onclick={() => view.go(backTarget)}>
          Back
        </button>
      {/if}
    </div>

    {#if workspace.isOpen}
      <p class="muted small">
        This opens or creates a separate library. It does not move your current one.
      </p>
    {/if}
  </div>
</main>

<style>
  .screen {
    display: grid;
    place-items: center;
    min-height: 100%;
    padding: var(--space-6);
  }

  .card {
    display: grid;
    gap: var(--space-5);
    width: min(640px, 100%);
  }

  .lede {
    font-size: 17px;
  }

  .location {
    display: grid;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  .path {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 13px;
    overflow-wrap: anywhere;
  }

  dl {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-6);
    margin: 0;
  }

  dt {
    color: var(--text-muted);
    font-size: 13px;
  }

  dd {
    margin: 0;
  }

  .problems {
    margin: 0;
    padding-left: var(--space-5);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .small {
    font-size: 13px;
  }
</style>
