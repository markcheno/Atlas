<script lang="ts">
  import { onMount } from 'svelte';
  import { atlas, AtlasApiError, toApiError } from '../lib/api/client';
  import type { DeviceCheck } from '../lib/api/bindings/DeviceCheck';
  import DeviceReadings from '../lib/components/DeviceReadings.svelte';
  import ProblemNotice from '../lib/components/ProblemNotice.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { NOTE_TEXT, PROVISIONAL_NOTE, recommendation } from '../lib/deviceCopy';
  import { view } from '../lib/stores/view.svelte';

  let check = $state<DeviceCheck | null>(null);
  let error = $state<AtlasApiError | null>(null);

  onMount(async () => {
    try {
      check = await atlas.getDeviceCheck();
    } catch (err) {
      error = err instanceof AtlasApiError ? err : toApiError('get_device_check', err);
    }
  });
</script>

<main id="main" class="screen">
  <div class="card">
    <ViewHeading>Checking this computer</ViewHeading>

    {#if check}
      <DeviceReadings snapshot={check.snapshot} />
      <p class="lede">{recommendation(check.assessment)}</p>
      {#if check.assessment.notes.length > 0}
        <ul class="notes">
          {#each check.assessment.notes as note (note)}
            <li>{NOTE_TEXT[note]}</li>
          {/each}
        </ul>
      {/if}
      <p class="muted small">{PROVISIONAL_NOTE}</p>
    {:else if error}
      <ProblemNotice
        title="Atlas couldn't check this computer"
        message={error.message}
        recovery="You can continue; Atlas will work conservatively."
      />
    {:else}
      <p class="muted" role="status">Checking memory, storage, and power…</p>
    {/if}

    <div>
      <button type="button" class="button primary" onclick={() => view.go('home')}>
        Continue
      </button>
    </div>
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
    width: min(680px, 100%);
  }

  .lede {
    font-size: 17px;
  }

  .notes {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding-left: var(--space-5);
  }

  .small {
    font-size: 13px;
  }
</style>
