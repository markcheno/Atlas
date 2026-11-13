<script lang="ts">
  // Settings › Performance & Battery: when background work runs
  // (phase4-spec §5.4). Slice 4.5 adds how often folders are checked for
  // changes; Slice 4.6 adds the rest.
  import { AtlasApiError, toApiError } from '../api/client';
  import type { CheckFolders } from '../api/bindings/CheckFolders';
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
  const current = $derived(settings.current.background.checkFolders);

  async function choose(checkFolders: CheckFolders) {
    saving = true;
    error = null;
    try {
      await settings.update((s) => ({ ...s, background: { ...s.background, checkFolders } }));
    } catch (err) {
      error = err instanceof AtlasApiError ? err : toApiError('update_settings', err);
    } finally {
      saving = false;
    }
  }
</script>

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
        onchange={() => void choose(value)}
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

  p {
    margin: 0;
  }

  .small {
    font-size: 13px;
  }
</style>
