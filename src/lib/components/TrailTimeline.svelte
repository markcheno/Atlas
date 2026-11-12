<script lang="ts">
  // A trail, oldest step first (Slice 3.4, phase3-spec §5.1, §7.2): each
  // search, question, source opened and note as it was recorded, with the
  // sources resolved now. A recorded answer is shown from the store, as on
  // the Ask screen, and is not a live region. Continue, Search again and
  // Ask again pick the trail up.
  import { tick } from 'svelte';
  import type { AskEnding } from '../api/bindings/AskEnding';
  import type { TrailStep } from '../api/bindings/TrailStep';
  import { answerError, droppedNote, NOTHING_FOUND, STOPPED, WEAK_NOTE } from '../answerCopy';
  import { trails } from '../stores/trails.svelte';
  import { trailView } from '../stores/trailView.svelte';
  import { openRecorded } from '../trailOpen';
  import {
    actionsText,
    ADD_NOTE,
    ADD_TRAIL_NOTE,
    ASK_AGAIN,
    charactersLeft,
    CONTINUE,
    CONTINUING,
    DELETE_STEP_QUESTION,
    deleteTrailQuestion,
    filtersText,
    matchedThen,
    NOTE_MAX,
    reachedLabel,
    recordedSourceName,
    SEARCH_AGAIN,
    stepsText,
    stepWhen,
    TITLE_MAX,
    trailDates,
  } from '../trailCopy';
  import { EXPORT } from '../exportCopy';
  import { exportDialog } from '../stores/exportDialog.svelte';
  import AddTo from './AddTo.svelte';
  import AnswerText from './AnswerText.svelte';
  import ProblemNotice from './ProblemNotice.svelte';
  import RecordedSource from './RecordedSource.svelte';
  import SaveAction from './SaveAction.svelte';

  const d = $derived(trailView.detail);
  const current = $derived(
    d !== null && (trailView.continued || trails.status?.currentTrailId === d.trail.id),
  );

  let renaming = $state(false);
  let title = $state('');
  let confirmingDelete = $state(false);
  /** The step whose note is being written; 'trail' for a note of its own. */
  let noting = $state<string | null>(null);
  let noteText = $state('');
  let confirmingStep = $state<string | null>(null);

  // A step joining this trail (Search again, a new search) shows here.
  $effect(() => trailView.onStatus(trails.status));

  function ending(e: AskEnding): string | null {
    switch (e.kind) {
      case 'done':
        return null;
      case 'stopped':
        return STOPPED;
      case 'nothing_found':
        return NOTHING_FOUND;
      case 'failed':
        return answerError(e.code);
    }
  }

  async function startRename() {
    title = d?.trail.title ?? '';
    renaming = true;
    await tick();
    document.getElementById('trail-title-input')?.focus();
  }

  async function saveRename(event: SubmitEvent) {
    event.preventDefault();
    if (await trailView.rename(title)) renaming = false;
  }

  async function startNote(key: string, existing = '') {
    noting = key;
    noteText = existing;
    await tick();
    document.getElementById(`note-${key}`)?.focus();
  }

  async function saveNote(event: SubmitEvent, stepId?: string) {
    event.preventDefault();
    if (await trailView.note(noteText, stepId)) {
      noting = null;
      noteText = '';
    }
  }

  /** A chip in a recorded answer: the source as it was sent, or where it went. */
  function cite(step: TrailStep, n: number, trigger: HTMLElement) {
    if (step.payload.kind !== 'ask') return;
    const k = step.payload.sources.findIndex((s) => s.n === n);
    const source = step.payload.sources[k];
    if (!source) return;
    const saveAs = { kind: 'step' as const, stepId: step.id, index: k };
    const question = { n, at: step.at };
    if (!openRecorded(source.anchor, step.anchors[k], trigger, { question, saveAs })) {
      document.getElementById(`${step.id}-source-${n}`)?.focus();
    }
  }
</script>

{#if trailView.detailError}
  <ProblemNotice
    title="Atlas couldn't show this trail"
    message={trailView.detailError.message}
    recovery={trailView.detailError.recovery}
  >
    {#snippet actions()}
      <button type="button" class="button" onclick={() => void trailView.close()}>
        Back to trails
      </button>
    {/snippet}
  </ProblemNotice>
{:else if !d}
  <p class="muted" role="status">Opening the trail…</p>
{:else}
  {@const pinned = d.trail.pinned}
  <section class="trail" aria-labelledby="trail-title">
    <div>
      <button type="button" class="button quiet" onclick={() => void trailView.close()}>
        Back to trails
      </button>
    </div>

    <header class="head">
      {#if renaming}
        <form class="rename" onsubmit={saveRename}>
          <label for="trail-title-input">Trail title</label>
          <input
            id="trail-title-input"
            type="text"
            maxlength={TITLE_MAX}
            bind:value={title}
            onkeydown={(e) => {
              if (e.key === 'Escape') renaming = false;
            }}
          />
          <button type="submit" class="button primary">Save</button>
          <button type="button" class="button quiet" onclick={() => (renaming = false)}>
            Cancel
          </button>
        </form>
        <h2 id="trail-title" class="visually-hidden">{d.trail.label}</h2>
      {:else}
        <h2 id="trail-title">{d.trail.label}</h2>
      {/if}
      <p class="muted small">
        {trailDates(d.trail.startedAt, d.trail.lastStepAt)} · {stepsText(d.trail.stepCount)}
      </p>
      <div class="actions">
        {#if current}
          <span role="status" class="small" data-testid="continuing">{CONTINUING}</span>
        {:else}
          <button
            type="button"
            class="button primary"
            onclick={() => void trailView.continueTrail()}
          >
            {CONTINUE}
          </button>
        {/if}
        {#if !renaming}
          <button type="button" class="button quiet" onclick={() => void startRename()}>
            Rename
          </button>
        {/if}
        <AddTo
          item={{ kind: 'trail', id: d.trail.id }}
          name={d.trail.label}
          key={`trail-${d.trail.id}`}
        />
        <button
          type="button"
          class="button quiet"
          onclick={(e) =>
            exportDialog.show(
              { what: 'trail', id: d.trail.id, name: d.trail.label },
              e.currentTarget,
            )}
        >
          {EXPORT}
        </button>
        <button
          type="button"
          class="button quiet"
          aria-pressed={pinned}
          title={pinned ? 'Pinned trails never expire' : 'Pin to keep it past 90 days'}
          onclick={() => void trailView.pin(!pinned)}
        >
          {pinned ? 'Pinned' : 'Pin'}
        </button>
        {#if confirmingDelete}
          <div class="confirm" role="group" aria-label="Confirm deleting this trail">
            <span class="small">{deleteTrailQuestion(d.trail.stepCount)}</span>
            <button type="button" class="button" onclick={() => void trailView.remove()}>
              Delete
            </button>
            <button type="button" class="button quiet" onclick={() => (confirmingDelete = false)}>
              Keep
            </button>
          </div>
        {:else}
          <button type="button" class="button quiet" onclick={() => (confirmingDelete = true)}>
            Delete…
          </button>
        {/if}
      </div>
      {#if trailView.actionError}
        <ProblemNotice
          title="Atlas couldn't change this trail"
          message={trailView.actionError.message}
          recovery={trailView.actionError.recovery}
        />
      {/if}
    </header>

    <ol class="steps">
      {#each d.steps as step (step.id)}
        {@const p = step.payload}
        <li class="step" data-testid="trail-step" data-kind={p.kind}>
          <p class="muted small">{stepWhen(step.at)}</p>
          {#if p.kind === 'search'}
            <h3>Searched for “{p.words}”</h3>
            {#if filtersText(p.filters)}<p class="muted small">{filtersText(p.filters)}</p>{/if}
            <p class="small">{matchedThen(p.matched)}</p>
            {#if p.results.length > 0}
              <ul class="sources" aria-label="Top results then">
                {#each p.results as r, k (k)}
                  <li>
                    <RecordedSource
                      anchor={r.anchor}
                      resolved={step.anchors[k]}
                      at={step.at}
                      saveAs={{ kind: 'step', stepId: step.id, index: k }}
                    />
                  </li>
                {/each}
              </ul>
            {/if}
            <div>
              <button type="button" class="button" onclick={() => void trailView.searchAgain(p)}>
                {SEARCH_AGAIN}
              </button>
            </div>
          {:else if p.kind === 'ask'}
            <h3>Asked “{p.question}”</h3>
            {#if filtersText(p.filters)}<p class="muted small">{filtersText(p.filters)}</p>{/if}
            {#if p.answer}
              <AnswerText
                text={p.answer}
                headingId={`answer-${step.id}`}
                heading={`Answer to “${p.question}”`}
                label={(n) => {
                  const s = p.sources.find((x) => x.n === n);
                  return s ? recordedSourceName(n, s.anchor) : null;
                }}
                oncite={(n, el) => cite(step, n, el)}
              />
            {/if}
            {#if ending(p.ending)}
              <p class="note" data-testid="ask-ending">{ending(p.ending)}</p>
            {/if}
            {#if p.weak && p.sources.length > 0}
              <p class="note" data-testid="weak-note">{WEAK_NOTE}</p>
            {/if}
            {#if droppedNote(p.dropped)}
              <p class="note" data-testid="dropped-note">{droppedNote(p.dropped)}</p>
            {/if}
            {#if p.sources.length > 0}
              <section class="sources" aria-labelledby={`sources-${step.id}`}>
                <h4 id={`sources-${step.id}`}>Sources then ({p.sources.length})</h4>
                <ol>
                  {#each p.sources as s, k (s.n)}
                    <li>
                      <RecordedSource
                        anchor={s.anchor}
                        resolved={step.anchors[k]}
                        n={s.n}
                        cited={s.cited}
                        at={step.at}
                        id={`${step.id}-source-${s.n}`}
                        saveAs={{ kind: 'step', stepId: step.id, index: k }}
                      />
                    </li>
                  {/each}
                </ol>
              </section>
            {/if}
            <div>
              <button type="button" class="button" onclick={() => void trailView.askAgain(p)}>
                {ASK_AGAIN}
              </button>
              {#if p.answer}
                <SaveAction
                  finding={{ kind: 'step', stepId: step.id }}
                  key={`finding-${step.id}`}
                  name={p.question}
                />
              {/if}
            </div>
          {:else if p.kind === 'open'}
            <h3>Opened {p.anchor.title}</h3>
            <p class="muted small">{reachedLabel(p.via)} · {actionsText(p.actions)}</p>
            <RecordedSource
              anchor={p.anchor}
              resolved={step.anchors[0]}
              at={step.at}
              opens
              saveAs={{ kind: 'step', stepId: step.id, index: 0 }}
            />
          {:else}
            <h3 class="visually-hidden">Your note</h3>
            <p class="note-text">{p.text}</p>
          {/if}

          {#if step.note && noting !== step.id}
            <p class="step-note" data-testid="step-note">
              <span class="muted small">Your note</span>
              {step.note}
            </p>
          {/if}

          {#if noting === step.id}
            <form class="note-form" onsubmit={(e) => void saveNote(e, step.id)}>
              <label for={`note-${step.id}`}>Note on this step</label>
              <textarea id={`note-${step.id}`} maxlength={NOTE_MAX} rows="3" bind:value={noteText}
              ></textarea>
              {#if charactersLeft(noteText, NOTE_MAX)}
                <p class="muted small" role="status">{charactersLeft(noteText, NOTE_MAX)}</p>
              {/if}
              <div class="actions">
                <button type="submit" class="button primary">Save note</button>
                <button type="button" class="button quiet" onclick={() => (noting = null)}>
                  Cancel
                </button>
              </div>
            </form>
          {:else}
            <div class="actions">
              {#if p.kind !== 'note'}
                <button
                  type="button"
                  class="row-button"
                  onclick={() => void startNote(step.id, step.note ?? '')}
                >
                  {step.note ? 'Edit note' : ADD_NOTE}
                </button>
              {/if}
              {#if confirmingStep === step.id}
                <div class="confirm" role="group" aria-label="Confirm deleting this step">
                  <span class="small">{DELETE_STEP_QUESTION}</span>
                  <button
                    type="button"
                    class="button"
                    onclick={() => {
                      confirmingStep = null;
                      void trailView.removeStep(step.id);
                    }}
                  >
                    Delete
                  </button>
                  <button
                    type="button"
                    class="button quiet"
                    onclick={() => (confirmingStep = null)}
                  >
                    Keep
                  </button>
                </div>
              {:else}
                <button type="button" class="row-button" onclick={() => (confirmingStep = step.id)}>
                  Delete this step
                </button>
              {/if}
            </div>
          {/if}
        </li>
      {/each}
    </ol>

    {#if noting === 'trail'}
      <form class="note-form" onsubmit={(e) => void saveNote(e)}>
        <label for="note-trail">A note for this trail</label>
        <textarea id="note-trail" maxlength={NOTE_MAX} rows="3" bind:value={noteText}></textarea>
        {#if charactersLeft(noteText, NOTE_MAX)}
          <p class="muted small" role="status">{charactersLeft(noteText, NOTE_MAX)}</p>
        {/if}
        <div class="actions">
          <button type="submit" class="button primary">Save note</button>
          <button type="button" class="button quiet" onclick={() => (noting = null)}>
            Cancel
          </button>
        </div>
      </form>
    {:else}
      <div>
        <button type="button" class="button quiet" onclick={() => void startNote('trail')}>
          {ADD_TRAIL_NOTE}
        </button>
      </div>
    {/if}
  </section>
{/if}

<style>
  .trail {
    display: grid;
    gap: var(--space-4);
  }

  .head {
    display: grid;
    gap: var(--space-2);
  }

  h2 {
    margin: 0;
    font-size: 20px;
    overflow-wrap: anywhere;
  }

  h3 {
    margin: 0;
    font-size: 15px;
    overflow-wrap: anywhere;
  }

  h4 {
    margin: 0;
    font-size: 14px;
  }

  p {
    margin: 0;
  }

  .actions,
  .confirm,
  .rename {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .rename input {
    flex: 1 1 16rem;
    min-height: var(--control-height, 36px);
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .steps {
    display: grid;
    gap: var(--space-4);
    margin: 0;
    padding: 0 0 0 var(--space-4);
    border-left: 2px solid var(--border);
    list-style: none;
  }

  .step {
    display: grid;
    gap: var(--space-2);
  }

  .sources,
  .sources ol {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .note {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--notice-border);
    border-radius: var(--radius);
    background: var(--notice-bg);
  }

  .note-text,
  .step-note {
    white-space: pre-line;
    overflow-wrap: anywhere;
  }

  .step-note {
    display: grid;
    gap: var(--space-1);
    padding-left: var(--space-3);
    border-left: 3px solid var(--accent-soft);
  }

  .note-form {
    display: grid;
    gap: var(--space-2);
  }

  textarea {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    font: inherit;
    resize: vertical;
  }

  .small {
    font-size: 13px;
  }
</style>
