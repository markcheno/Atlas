<script lang="ts">
  // Settings › Privacy › Research trails (Slice 3.3, phase3-spec §5.7):
  // what is kept and that it stays on this computer; recording and expiry;
  // deleting every trail, after a confirmation naming how many.
  import { atlas, AtlasApiError, toApiError } from '../api/client';
  import { settings } from '../stores/settings.svelte';
  import { trails } from '../stores/trails.svelte';
  import {
    DELETE_ALL,
    EXPIRE_LABEL,
    EXPIRE_NOTE,
    RECORD_LABEL,
    RECORD_PAUSED_NOTE,
    RESEARCH_TRAILS_NOTE,
    RESEARCH_TRAILS_TITLE,
    deleteAllQuestion,
    deletedTrails,
  } from '../trailCopy';
  import ProblemNotice from './ProblemNotice.svelte';

  let saving = $state(false);
  let error = $state<AtlasApiError | null>(null);
  /** How many trails the confirmation names, while it shows. */
  let confirming = $state<number | null>(null);
  let result = $state('');

  const privacy = $derived(settings.current.privacy);
  const unavailable = $derived(trails.status?.state === 'unavailable');

  async function save(change: { recordTrails?: boolean; expireTrails?: boolean }) {
    saving = true;
    error = null;
    try {
      await settings.update((s) => ({ ...s, privacy: { ...s.privacy, ...change } }));
      await trails.refresh();
    } catch (err) {
      error = err instanceof AtlasApiError ? err : toApiError('update_settings', err);
    } finally {
      saving = false;
    }
  }

  async function askToDelete() {
    result = '';
    error = null;
    try {
      confirming = (await atlas.getKnowledgeStatus()).trails;
    } catch (err) {
      error = err instanceof AtlasApiError ? err : toApiError('get_knowledge_status', err);
    }
  }

  async function deleteAll() {
    const n = await trails.deleteAll();
    confirming = null;
    if (n === null) {
      error = trails.error;
    } else {
      result = deletedTrails(n);
    }
  }

  function checked(event: Event): boolean {
    return (event.currentTarget as HTMLInputElement).checked;
  }
</script>

<section class="trails" aria-labelledby="privacy-trails">
  <h3 id="privacy-trails">{RESEARCH_TRAILS_TITLE}</h3>
  <p>{RESEARCH_TRAILS_NOTE}</p>

  <label class="choice">
    <input
      type="checkbox"
      checked={privacy.recordTrails}
      disabled={saving || unavailable}
      onchange={(e) => {
        const next = checked(e);
        void save({ recordTrails: next });
      }}
      aria-describedby={privacy.recordTrails ? undefined : 'record-trails-note'}
    />
    {RECORD_LABEL}
  </label>
  {#if !privacy.recordTrails}
    <p id="record-trails-note" class="muted small">{RECORD_PAUSED_NOTE}</p>
  {/if}

  <label class="choice">
    <input
      type="checkbox"
      checked={privacy.expireTrails}
      disabled={saving || unavailable}
      onchange={(e) => {
        const next = checked(e);
        void save({ expireTrails: next });
      }}
      aria-describedby="expire-trails-note"
    />
    {EXPIRE_LABEL}
  </label>
  <p id="expire-trails-note" class="muted small">{EXPIRE_NOTE}</p>

  {#if confirming !== null}
    <div class="confirm" role="group" aria-label="Confirm deleting all trails">
      <span class="small">{deleteAllQuestion(confirming)}</span>
      <button
        type="button"
        class="button"
        disabled={trails.busy || confirming === 0}
        onclick={() => void deleteAll()}
      >
        Delete
      </button>
      <button type="button" class="button quiet" onclick={() => (confirming = null)}>Keep</button>
    </div>
  {:else}
    <div>
      <button
        type="button"
        class="button quiet"
        disabled={unavailable}
        onclick={() => void askToDelete()}
      >
        {DELETE_ALL}
      </button>
    </div>
  {/if}
  <p role="status" class="muted small" data-testid="trails-deleted">{result}</p>

  {#if error}
    <ProblemNotice
      title="Atlas couldn't change your trails"
      message={error.message}
      recovery={error.recovery}
    />
  {/if}
</section>

<style>
  .trails {
    display: grid;
    gap: var(--space-2);
  }

  .trails p {
    margin: 0;
  }

  h3 {
    margin: var(--space-2) 0 var(--space-1);
    font-size: 14px;
  }

  .choice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .confirm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .small {
    font-size: 12px;
  }
</style>
