<script lang="ts">
  // Settings › Performance & Battery: when background work runs
  // (phase4-spec §5.4). Slice 4.5 added how often folders are checked for
  // changes; Slice 4.6 adds Background work, its hours, and full speed.
  import { onMount } from 'svelte';
  import { AtlasApiError, toApiError } from '../api/client';
  import type { BackgroundMode } from '../api/bindings/BackgroundMode';
  import type { BackgroundSettings } from '../api/bindings/BackgroundSettings';
  import type { CheckFolders } from '../api/bindings/CheckFolders';
  import { BLOCKED, FULL_SPEED_NOTE, MODE_NOTE, MODES, stateText } from '../scheduleCopy';
  import { resources } from '../stores/resources.svelte';
  import { settings } from '../stores/settings.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  const CHOICES: [CheckFolders, string][] = [
    ['hourly', 'Every hour'],
    ['sixHours', 'Every 6 hours'],
    ['daily', 'Once a day'],
    ['never', 'Only when I ask'],
  ];
  const NOTE: Record<CheckFolders, string> = {
    hourly: 'Atlas checks each folder for changes every hour, and when a drive comes back.',
    sixHours: 'Atlas checks each folder for changes every 6 hours, and when a drive comes back.',
    daily: 'Atlas checks each folder for changes once a day, and when a drive comes back.',
    never: 'Atlas checks a folder only when you choose Rescan or Check all folders now.',
  };

  let saving = $state(false);
  let error = $state<AtlasApiError | null>(null);
  const background = $derived(settings.current.background);
  const current = $derived(background.checkFolders);
  const mode = $derived(background.mode);
  // The times as typed, saved when both are complete.
  let from = $state('');
  let to = $state('');
  $effect(() => {
    from = background.from;
    to = background.to;
  });
  const status = $derived(resources.status);
  const fullSpeed = $derived(status?.fullSpeed ?? null);

  onMount(() => {
    void resources.refresh();
  });

  async function save(change: Partial<BackgroundSettings>) {
    saving = true;
    error = null;
    try {
      await settings.update((s) => ({ ...s, background: { ...s.background, ...change } }));
      await resources.refresh();
    } catch (err) {
      error = err instanceof AtlasApiError ? err : toApiError('update_settings', err);
    } finally {
      saving = false;
    }
  }

  function chooseMode(next: BackgroundMode) {
    void save({ mode: next });
  }

  function saveHours() {
    if (!from || !to || (from === background.from && to === background.to)) return;
    void save({ from, to });
  }
</script>

<fieldset class="choices" aria-describedby="background-mode-note">
  <legend>Background work</legend>
  {#each MODES as [value, label] (value)}
    <label>
      <input
        type="radio"
        name="background-mode"
        {value}
        checked={mode === value}
        disabled={saving}
        onchange={() => chooseMode(value)}
      />
      {label}
    </label>
    {#if value === 'hours' && mode === 'hours'}
      <div class="hours">
        <label>
          From
          <input type="time" bind:value={from} disabled={saving} onchange={saveHours} />
        </label>
        <label>
          to
          <input type="time" bind:value={to} disabled={saving} onchange={saveHours} />
        </label>
      </div>
    {/if}
  {/each}
</fieldset>
<p id="background-mode-note" class="muted small" data-testid="background-mode-note">
  {MODE_NOTE[mode]}
</p>
{#if status}
  <p role="status" data-testid="background-state">{stateText(status)}</p>
{/if}

{#if fullSpeed && mode !== 'paused'}
  <div class="full-speed">
    {#if fullSpeed.on}
      <button
        type="button"
        disabled={resources.busy}
        onclick={() => void resources.setFullSpeed(false)}
      >
        Stop
      </button>
      <span class="muted small">Running at full speed until the work is done.</span>
    {:else if fullSpeed.offered}
      <button
        type="button"
        disabled={resources.busy}
        onclick={() => void resources.setFullSpeed(true)}
      >
        Run at full speed now
      </button>
    {:else if fullSpeed.blocked}
      <span class="muted small" data-testid="full-speed-blocked">{BLOCKED[fullSpeed.blocked]}</span>
    {/if}
  </div>
  <p class="muted small">{FULL_SPEED_NOTE}</p>
  {#if resources.error}
    <ProblemNotice
      title="Atlas couldn’t change full speed"
      message={resources.error.message}
      recovery={resources.error.recovery}
    />
  {/if}
{/if}

<fieldset class="choices" aria-describedby="check-folders-note">
  <legend>Check folders for changes</legend>
  {#each CHOICES as [value, label] (value)}
    <label>
      <input
        type="radio"
        name="check-folders"
        {value}
        checked={current === value}
        disabled={saving}
        onchange={() => void save({ checkFolders: value })}
      />
      {label}
    </label>
  {/each}
</fieldset>
<p id="check-folders-note" class="muted small" data-testid="check-folders-note">{NOTE[current]}</p>
{#if error}
  <ProblemNotice
    title="Atlas couldn’t save that"
    message={error.message}
    recovery={error.recovery}
  />
{/if}

<style>
  .choices {
    display: grid;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    border: 0;
  }

  legend {
    font-weight: 600;
    margin-bottom: var(--space-1);
  }

  label {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    min-height: 32px;
  }

  .hours {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    padding-left: var(--space-5);
  }

  .full-speed {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: center;
    margin-top: var(--space-2);
  }

  p {
    margin: 0;
  }

  .small {
    font-size: 13px;
  }
</style>
