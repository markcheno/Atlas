<script lang="ts">
  // The Saved tab (Slice 3.5, phase3-spec §5.3): bookmarks and findings,
  // newest first, filterable by kind and by text. Each opens its place,
  // edits its note in place, or is deleted. A finding shows its answer or
  // passage as it was kept, its sources resolved now.
  import { onMount, tick } from 'svelte';
  import type { SavedItem } from '../api/bindings/SavedItem';
  import type { SavedKind } from '../api/bindings/SavedKind';
  import { FROM_BOOKMARK } from '../openVia';
  import {
    deleteSavedQuestion,
    KIND_FILTERS,
    noSavedMatch,
    SAVED_EMPTY,
    savedOn,
    savedRange,
  } from '../savedCopy';
  import { saved, SAVED_PAGE_SIZE } from '../stores/saved.svelte';
  import { charactersLeft, NOTE_MAX, recordedSourceName } from '../trailCopy';
  import { openRecorded } from '../trailOpen';
  import { EXPORT } from '../exportCopy';
  import { exportDialog } from '../stores/exportDialog.svelte';
  import AddTo from './AddTo.svelte';
  import AnswerText from './AnswerText.svelte';
  import ProblemNotice from './ProblemNotice.svelte';
  import RecordedSource from './RecordedSource.svelte';

  const page = $derived(saved.result);
  const lastPage = $derived(page ? Math.max(0, Math.ceil(page.total / SAVED_PAGE_SIZE) - 1) : 0);
  const range = $derived.by(() => {
    if (!page || page.total === 0) return '';
    const first = page.page * SAVED_PAGE_SIZE + 1;
    return savedRange(first, first + page.items.length - 1, page.total);
  });

  let noting = $state<string | null>(null);
  let noteText = $state('');
  let confirming = $state<string | null>(null);

  onMount(() => void saved.load());

  function key(item: SavedItem): string {
    return `${item.item}-${item.id}`;
  }

  async function startNote(item: SavedItem) {
    noting = key(item);
    noteText = item.note ?? '';
    await tick();
    document.getElementById(`note-${key(item)}`)?.focus();
  }

  async function saveNote(event: SubmitEvent, item: SavedItem) {
    event.preventDefault();
    if (await saved.setNote(item.item as SavedKind, item.id, noteText)) noting = null;
  }
</script>

<section class="saved" aria-label="Saved">
  <div class="toolbar">
    <fieldset class="mode">
      <legend class="visually-hidden">Show</legend>
      {#each KIND_FILTERS as f (f.value)}
        <label>
          <input
            type="radio"
            name="saved-kind"
            value={f.value}
            checked={(saved.kind ?? '') === f.value}
            onchange={() => void saved.setKind(f.value === '' ? null : f.value)}
          />
          {f.label}
        </label>
      {/each}
    </fieldset>
    <label class="filter">
      <span class="visually-hidden">Filter saved</span>
      <input
        type="search"
        placeholder="Filter saved"
        value={saved.text}
        oninput={(e) => saved.setText(e.currentTarget.value)}
        onkeydown={(e) => {
          if (e.key === 'Escape') saved.setText('', true);
        }}
      />
    </label>
  </div>

  {#if saved.actionError}
    <ProblemNotice
      title="Atlas couldn't change that"
      message={saved.actionError.message}
      recovery={saved.actionError.recovery}
    />
  {/if}

  {#if saved.error}
    <ProblemNotice
      title="Atlas couldn't list what you saved"
      message={saved.error.message}
      recovery={saved.error.recovery}
    />
  {:else if !page}
    <p class="muted" role="status">Loading…</p>
  {:else if page.total === 0}
    <div class="empty">
      <p>{saved.text ? noSavedMatch(saved.text) : SAVED_EMPTY}</p>
    </div>
  {:else}
    <ol class="items">
      {#each page.items as item (key(item))}
        <li class="item" data-testid="saved-item" data-kind={item.item}>
          {#if item.item === 'bookmark'}
            <h3 class="visually-hidden">Bookmark: {item.anchor.title}</h3>
            <RecordedSource
              anchor={item.anchor}
              resolved={item.resolved}
              at={item.createdAt}
              via={FROM_BOOKMARK}
              id={`saved-${item.id}`}
            />
          {:else}
            {@const f = item}
            {#if f.kind === 'answer'}
              <h3>Answer to “{f.question}”</h3>
              <AnswerText
                text={f.text}
                headingId={`finding-${f.id}`}
                heading={`Saved answer to “${f.question}”`}
                label={(n) => {
                  const s = f.sources.find((x) => x.n === n);
                  return s ? recordedSourceName(n, s.anchor) : null;
                }}
                oncite={(n, el) => {
                  const s = f.sources.find((x) => x.n === n);
                  if (s) {
                    openRecorded(s.anchor, s.resolved, el, {
                      question: { n, at: f.createdAt },
                      via: FROM_BOOKMARK,
                    });
                  }
                }}
              />
            {:else}
              <h3>Passage from {f.sources[0]?.anchor.title ?? 'a document'}</h3>
              <p class="passage">{f.text}</p>
            {/if}
            {#if f.sources.length > 0}
              <ul class="sources" aria-label="Its sources">
                {#each f.sources as s (s.n)}
                  <li>
                    <RecordedSource
                      anchor={s.anchor}
                      resolved={s.resolved}
                      n={f.kind === 'answer' ? s.n : undefined}
                      at={f.createdAt}
                      via={FROM_BOOKMARK}
                    />
                  </li>
                {/each}
              </ul>
            {/if}
          {/if}

          <p class="muted small">{savedOn(item.createdAt)}</p>
          {#if item.note && noting !== key(item)}
            <p class="note" data-testid="saved-note">
              <span class="muted small">Your note</span>
              {item.note}
            </p>
          {/if}

          {#if noting === key(item)}
            <form class="note-form" onsubmit={(e) => void saveNote(e, item)}>
              <label for={`note-${key(item)}`}>Note</label>
              <textarea id={`note-${key(item)}`} maxlength={NOTE_MAX} rows="3" bind:value={noteText}
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
              <button type="button" class="row-button" onclick={() => void startNote(item)}>
                {item.note ? 'Edit note' : 'Add a note'}
              </button>
              <AddTo
                item={{ kind: item.item, id: item.id }}
                name={item.item === 'bookmark'
                  ? item.anchor.title
                  : (item.question ?? item.sources[0]?.anchor.title ?? 'this finding')}
                key={key(item)}
              />
              {#if item.item === 'finding'}
                <button
                  type="button"
                  class="row-button"
                  onclick={(e) =>
                    exportDialog.show(
                      {
                        what: 'finding',
                        id: item.id,
                        name: item.question ?? item.sources[0]?.anchor.title ?? 'this finding',
                      },
                      e.currentTarget,
                    )}
                >
                  {EXPORT}
                </button>
              {/if}
              {#if confirming === key(item)}
                <div class="confirm" role="group" aria-label="Confirm deleting">
                  <span class="small">{deleteSavedQuestion(item.item as SavedKind)}</span>
                  <button
                    type="button"
                    class="button"
                    onclick={() => {
                      confirming = null;
                      void saved.remove(item.item as SavedKind, item.id);
                    }}
                  >
                    Delete
                  </button>
                  <button type="button" class="button quiet" onclick={() => (confirming = null)}>
                    Keep
                  </button>
                </div>
              {:else}
                <button type="button" class="row-button" onclick={() => (confirming = key(item))}>
                  Delete…
                </button>
              {/if}
            </div>
          {/if}
        </li>
      {/each}
    </ol>
    {#if page.total > SAVED_PAGE_SIZE}
      <nav class="pager" aria-label="Pages">
        <span class="muted" role="status" data-testid="saved-range">{range}</span>
        <div>
          <button
            type="button"
            class="button"
            disabled={saved.page === 0}
            onclick={() => void saved.goTo(saved.page - 1)}
          >
            Previous
          </button>
          <button
            type="button"
            class="button"
            disabled={saved.page >= lastPage}
            onclick={() => void saved.goTo(saved.page + 1)}
          >
            Next
          </button>
        </div>
      </nav>
    {/if}
  {/if}
</section>

<style>
  .saved {
    display: grid;
    gap: var(--space-4);
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-4);
  }

  .filter input {
    min-width: min(20rem, 100%);
    min-height: var(--control-height, 36px);
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .mode {
    display: inline-flex;
    margin: 0;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    min-height: 36px;
  }

  .mode label {
    display: inline-flex;
    align-items: center;
    padding: 0 var(--space-3);
    border-radius: calc(var(--radius) - 2px);
    cursor: pointer;
  }

  .mode label:has(input:checked) {
    background: var(--accent-soft);
    font-weight: 500;
  }

  .mode label:has(input:focus-visible) {
    outline: 2px solid var(--focus);
    outline-offset: 1px;
  }

  .mode input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .items,
  .sources {
    display: grid;
    gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .item {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
  }

  h3 {
    margin: 0;
    font-size: 15px;
    overflow-wrap: anywhere;
  }

  p {
    margin: 0;
  }

  .passage,
  .note {
    white-space: pre-line;
    overflow-wrap: anywhere;
  }

  .note {
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

  .actions,
  .confirm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .empty {
    padding: var(--space-6);
    border: 1px dashed var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  .pager {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .pager div {
    display: flex;
    gap: var(--space-2);
  }

  .small {
    font-size: 13px;
  }
</style>
