<script lang="ts">
  import { onMount } from 'svelte';
  import { atlas } from '../lib/api/client';
  import type { DataRootCandidate } from '../lib/api/bindings/DataRootCandidate';
  import type { DeviceCheck } from '../lib/api/bindings/DeviceCheck';
  import DeviceReadings from '../lib/components/DeviceReadings.svelte';
  import LibraryFoldersPanel from '../lib/components/LibraryFoldersPanel.svelte';
  import AiModelsPanel from '../lib/components/AiModelsPanel.svelte';
  import ProblemNotice from '../lib/components/ProblemNotice.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { PROVISIONAL_NOTE, policySentence } from '../lib/deviceCopy';
  import { formatBytes } from '../lib/format';
  import { THEME_OPTIONS, theme } from '../lib/stores/theme.svelte';
  import { view } from '../lib/stores/view.svelte';
  import { workspace } from '../lib/stores/workspace.svelte';

  /** How often device readings refresh while Settings is open. */
  const DEVICE_REFRESH_MS = 30_000;

  let storage = $state<DataRootCandidate | null>(null);
  let device = $state<DeviceCheck | null>(null);

  async function refreshDevice() {
    try {
      device = await atlas.getDeviceCheck();
    } catch {
      device = null;
    }
  }

  onMount(() => {
    refreshDevice();
    const timer = setInterval(refreshDevice, DEVICE_REFRESH_MS);
    const path = workspace.status?.displayPath;
    if (workspace.isOpen && path) {
      atlas.inspectDataRoot(path).then(
        (result) => (storage = result),
        () => (storage = null),
      );
    }
    return () => clearInterval(timer);
  });
</script>

<div class="view">
  <ViewHeading>Settings</ViewHeading>

  <section aria-labelledby="settings-appearance">
    <h2 id="settings-appearance">Appearance</h2>
    <fieldset>
      <legend class="muted">Theme</legend>
      {#each THEME_OPTIONS as option (option.value)}
        <label class="choice">
          <input
            type="radio"
            name="theme"
            value={option.value}
            checked={theme.preference === option.value}
            onchange={() => theme.choose(option.value, workspace.isOpen)}
          />
          {option.label}
        </label>
      {/each}
    </fieldset>
    {#if theme.saveError}
      <ProblemNotice
        title="Your theme choice could not be saved"
        message={theme.saveError.message}
        recovery={theme.saveError.recovery}
      />
    {/if}
  </section>

  <section aria-labelledby="settings-folders">
    <h2 id="settings-folders">Library Folders</h2>
    {#if workspace.isOpen}
      <p class="muted small">
        Atlas only reads these folders. Removing one from Atlas never changes or deletes its files.
      </p>
      <LibraryFoldersPanel removable />
    {:else}
      <p class="muted">No Atlas library is open.</p>
    {/if}
  </section>

  <section aria-labelledby="settings-storage">
    <h2 id="settings-storage">Storage</h2>
    {#if workspace.isOpen && workspace.status}
      <dl>
        <div>
          <dt>Atlas library location</dt>
          <dd class="path">{workspace.status.displayPath}</dd>
        </div>
        <div>
          <dt>Free space on that drive</dt>
          <dd>
            {storage
              ? `${formatBytes(storage.freeBytes)} of ${formatBytes(storage.totalBytes)}`
              : 'Checking…'}
          </dd>
        </div>
        <div>
          <dt>Library ID</dt>
          <dd class="path">{workspace.status.rootId?.slice(0, 8)}</dd>
        </div>
      </dl>
      <p class="muted small">
        Atlas keeps its catalog and generated data here. Your books stay where they are.
      </p>
      <div>
        <button type="button" class="button" onclick={() => view.go('data-location')}>
          Use a different location…
        </button>
      </div>
    {:else}
      <p class="muted">No Atlas library is open.</p>
    {/if}
  </section>

  <section aria-labelledby="settings-performance">
    <h2 id="settings-performance">Performance & Battery</h2>
    {#if device}
      <DeviceReadings snapshot={device.snapshot} />
      <p data-testid="policy-sentence">
        When Atlas catalogs your folders (coming in slice 0.5), it will {policySentence(
          device.policy,
          device.snapshot,
        )}
      </p>
      <p class="muted small">
        Atlas pauses bulk work when the battery reaches 20% and resumes at 25% or when plugged in.
        {PROVISIONAL_NOTE}
      </p>
    {:else}
      <p class="muted" role="status">Checking this computer…</p>
    {/if}
  </section>

  <section aria-labelledby="settings-privacy">
    <h2 id="settings-privacy">Privacy</h2>
    <p>
      Atlas reads only the names, sizes, and dates of files in your library folders, never their
      contents, and never changes them. It contacts an AI service only when you test a connection,
      and sends nothing from your library.
    </p>
  </section>

  <section aria-labelledby="settings-ai">
    <h2 id="settings-ai">AI Models</h2>
    {#if workspace.isOpen}
      <AiModelsPanel />
    {:else}
      <p class="muted">No Atlas library is open.</p>
    {/if}
  </section>
</div>

<style>
  .view {
    display: grid;
    gap: var(--space-6);
    max-width: 720px;
    margin: 0 auto;
  }

  section {
    display: grid;
    gap: var(--space-3);
  }

  fieldset {
    display: flex;
    gap: var(--space-5);
    margin: 0;
    padding: 0;
    border: 0;
  }

  legend {
    padding: 0;
    margin-bottom: var(--space-2);
    float: left;
    width: 100%;
  }

  .choice {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--control-height);
    cursor: pointer;
  }

  dl {
    display: grid;
    gap: var(--space-3);
    margin: 0;
  }

  dt {
    color: var(--text-muted);
    font-size: 13px;
  }

  dd {
    margin: 0;
  }

  .path {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 13px;
    overflow-wrap: anywhere;
  }

  .small {
    font-size: 13px;
  }
</style>
