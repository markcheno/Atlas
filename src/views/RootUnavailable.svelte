<script lang="ts">
  import { atlas, AtlasApiError, toApiError } from '../lib/api/client';
  import type { DataRootState } from '../lib/api/bindings/DataRootState';
  import { applyStatus } from '../lib/boot';
  import ProblemNotice from '../lib/components/ProblemNotice.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { view } from '../lib/stores/view.svelte';
  import { workspace } from '../lib/stores/workspace.svelte';

  const HEADLINES: Partial<Record<DataRootState, string>> = {
    missing: "Atlas can't find your library",
    in_use: 'Your library is open in another Atlas window',
    incompatible: 'This library needs a newer version of Atlas',
    invalid: "Atlas can't open your library",
  };

  let retryError = $state<AtlasApiError | null>(null);
  let busy = $state(false);

  const status = $derived(workspace.status);
  const problem = $derived(retryError ?? status?.problem ?? null);

  async function tryAgain() {
    if (!status?.displayPath) return;
    busy = true;
    retryError = null;
    try {
      await applyStatus(await atlas.openDataRoot(status.displayPath));
    } catch (err) {
      retryError = err instanceof AtlasApiError ? err : toApiError('open_data_root', err);
    } finally {
      busy = false;
    }
  }
</script>

<main id="main" class="screen">
  <div class="card">
    <ViewHeading>{HEADLINES[status?.state ?? 'invalid'] ?? HEADLINES.invalid}</ViewHeading>

    {#if status?.displayPath}
      <p>
        Your library was last at
        <span class="path">{status.displayPath}</span>.
      </p>
    {/if}

    {#if status?.state === 'missing'}
      <p>If it is on an external drive, reconnect the drive, then try again.</p>
    {/if}

    {#if problem}
      <ProblemNotice title="Details" message={problem.message} recovery={problem.recovery} />
    {/if}

    <div class="actions">
      <button type="button" class="button primary" onclick={tryAgain} disabled={busy}>
        Try again
      </button>
      <button type="button" class="button" onclick={() => view.go('data-location')}>
        Choose another location…
      </button>
    </div>

    <p class="muted small">
      Choosing another location does not delete or move this library. You can open it again later by
      choosing its folder.
    </p>
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
    gap: var(--space-4);
    width: min(640px, 100%);
  }

  .path {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 13px;
    overflow-wrap: anywhere;
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
