<script lang="ts">
  // A collection's page (Slice 3.6, phase3-spec §5.4): its trails,
  // findings, bookmarks and documents, each as it is now; Edit, Mark as
  // finished (a project), Archive, Delete. Taking an item out, or deleting
  // the collection, never deletes the item.
  import type { CollectionItemRef } from '../api/bindings/CollectionItemRef';
  import {
    countsText,
    deleteCollectionQuestion,
    DOCUMENT_GONE,
    DOCUMENT_NOT_READ,
    kindLabel,
    removeFrom,
    statusLabel,
  } from '../collectionCopy';
  import { FROM_BOOKMARK } from '../openVia';
  import { collections } from '../stores/collections.svelte';
  import { details } from '../stores/details.svelte';
  import { trailView } from '../stores/trailView.svelte';
  import { NOTE_MAX, recordedSourceName, stepsText, TITLE_MAX, trailDates } from '../trailCopy';
  import { openRecorded } from '../trailOpen';
  import AnswerText from './AnswerText.svelte';
  import ProblemNotice from './ProblemNotice.svelte';
  import RecordedSource from './RecordedSource.svelte';

  const d = $derived(collections.detail);
  let editing = $state(false);
  let name = $state('');
  let description = $state('');
  let goal = $state('');
  let confirmingDelete = $state(false);

  function startEdit() {
    if (!d) return;
    const c = d.collection;
    name = c.name;
    description = c.description ?? '';
    goal = c.goal ?? '';
    editing = true;
  }

  async function saveEdit(event: SubmitEvent) {
    event.preventDefault();
    if (!d) return;
    const c = d.collection;
    const saved = await collections.save({
      id: c.id,
      kind: c.kind,
      name,
      description: description || null,
      goal: c.kind === 'project' ? goal || null : null,
      status: c.status,
    });
    if (saved) editing = false;
  }

  function take(item: CollectionItemRef) {
    if (d) void collections.take(d.collection.id, item);
  }

  async function openTrail(id: string) {
    trailView.tab = 'trails';
    await trailView.open(id);
  }
</script>

{#if collections.detailError}
  <ProblemNotice
    title="Atlas couldn't show this"
    message={collections.detailError.message}
    recovery={collections.detailError.recovery}
  >
    {#snippet actions()}
      <button type="button" class="button" onclick={() => void collections.close()}>
        Back to interests and projects
      </button>
    {/snippet}
  </ProblemNotice>
{:else if !d}
  <p class="muted" role="status">Opening…</p>
{:else}
  {@const c = d.collection}
  <section class="collection" aria-labelledby="collection-title">
    <div>
      <button type="button" class="button quiet" onclick={() => void collections.close()}>
        Back to interests and projects
      </button>
    </div>

    {#if editing}
      <form class="form" onsubmit={(e) => void saveEdit(e)}>
        <label for="edit-collection-name">Name</label>
        <input id="edit-collection-name" type="text" maxlength={TITLE_MAX} bind:value={name} />
        <label for="edit-collection-description">Description</label>
        <textarea
          id="edit-collection-description"
          rows="2"
          maxlength={NOTE_MAX}
          bind:value={description}></textarea>
        {#if c.kind === 'project'}
          <label for="edit-collection-goal">Goal</label>
          <textarea id="edit-collection-goal" rows="2" maxlength={NOTE_MAX} bind:value={goal}
          ></textarea>
        {/if}
        <div class="actions">
          <button type="submit" class="button primary">Save</button>
          <button type="button" class="button quiet" onclick={() => (editing = false)}>
            Cancel
          </button>
        </div>
      </form>
      <h2 id="collection-title" class="visually-hidden">{c.name}</h2>
    {:else}
      <h2 id="collection-title">{c.name}</h2>
      <p class="muted small">
        {kindLabel(c.kind)} · {statusLabel(c.status)} · {countsText(c.counts)}
      </p>
      {#if c.description}<p class="text">{c.description}</p>{/if}
      {#if c.goal}<p class="text"><span class="muted small">Goal</span> {c.goal}</p>{/if}
    {/if}

    <div class="actions">
      {#if !editing}
        <button type="button" class="button quiet" onclick={startEdit}>Edit</button>
      {/if}
      {#if c.kind === 'project' && c.status === 'active'}
        <button
          type="button"
          class="button quiet"
          onclick={() => void collections.setStatus(c, 'finished')}
        >
          Mark as finished
        </button>
      {/if}
      {#if c.status === 'archived' || c.status === 'finished'}
        <button
          type="button"
          class="button quiet"
          onclick={() => void collections.setStatus(c, 'active')}
        >
          {c.status === 'archived' ? 'Unarchive' : 'Reopen'}
        </button>
      {/if}
      {#if c.status !== 'archived'}
        <button
          type="button"
          class="button quiet"
          onclick={() => void collections.setStatus(c, 'archived')}
        >
          Archive
        </button>
      {/if}
      {#if confirmingDelete}
        <div class="confirm" role="group" aria-label="Confirm deleting this collection">
          <span class="small">{deleteCollectionQuestion(c.kind, c.name)}</span>
          <button type="button" class="button" onclick={() => void collections.remove(c.id)}>
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

    {#if collections.actionError}
      <ProblemNotice
        title="Atlas couldn't change that"
        message={collections.actionError.message}
        recovery={collections.actionError.recovery}
      />
    {/if}

    {#if d.trails.length > 0}
      <section aria-labelledby="collection-trails">
        <h3 id="collection-trails">Trails</h3>
        <ul class="list">
          {#each d.trails as t (t.id)}
            <li data-testid="collection-trail">
              <button type="button" class="name-button" onclick={() => void openTrail(t.id)}>
                {t.label}
              </button>
              <span class="muted small"
                >{trailDates(t.startedAt, t.lastStepAt)} · {stepsText(t.stepCount)}</span
              >
              <button
                type="button"
                class="row-button"
                onclick={() => take({ kind: 'trail', id: t.id })}
              >
                {removeFrom(c.kind)}
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if d.findings.length > 0}
      <section aria-labelledby="collection-findings">
        <h3 id="collection-findings">Findings</h3>
        <ul class="list">
          {#each d.findings as f (f.id)}
            <li data-testid="collection-finding">
              {#if f.kind === 'answer'}
                <AnswerText
                  text={f.text}
                  headingId={`collection-finding-${f.id}`}
                  heading={`Answer to “${f.question}”`}
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
                <p class="text">{f.text}</p>
              {/if}
              <button
                type="button"
                class="row-button"
                onclick={() => take({ kind: 'finding', id: f.id })}
              >
                {removeFrom(c.kind)}
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if d.bookmarks.length > 0}
      <section aria-labelledby="collection-bookmarks">
        <h3 id="collection-bookmarks">Bookmarks</h3>
        <ul class="list">
          {#each d.bookmarks as b (b.id)}
            <li data-testid="collection-bookmark">
              <RecordedSource
                anchor={b.anchor}
                resolved={b.resolved}
                at={b.createdAt}
                via={FROM_BOOKMARK}
              />
              <button
                type="button"
                class="row-button"
                onclick={() => take({ kind: 'bookmark', id: b.id })}
              >
                {removeFrom(c.kind)}
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if d.documents.length > 0}
      <section aria-labelledby="collection-documents">
        <h3 id="collection-documents">Documents</h3>
        <ul class="list">
          {#each d.documents as doc (doc.documentId)}
            {@const file = doc.resolved.fileInstanceId}
            <li data-testid="collection-document">
              <span class="title">{doc.title ?? 'A document'}</span>
              {#if file && (doc.resolved.state === 'found' || doc.resolved.state === 'found_elsewhere')}
                <button
                  type="button"
                  class="row-button"
                  aria-label={`Details for ${doc.title ?? 'the document'}`}
                  onclick={(e) => void details.open(file, e.currentTarget, { via: FROM_BOOKMARK })}
                >
                  Details
                </button>
              {:else}
                <span class="small"
                  >{doc.resolved.state === 'unknown' ? DOCUMENT_NOT_READ : DOCUMENT_GONE}</span
                >
              {/if}
              <button
                type="button"
                class="row-button"
                onclick={() => take({ kind: 'document', fileId: null, documentId: doc.documentId })}
              >
                {removeFrom(c.kind)}
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
  </section>
{/if}

<style>
  .collection,
  .form {
    display: grid;
    gap: var(--space-3);
  }

  h2 {
    margin: 0;
    font-size: 20px;
    overflow-wrap: anywhere;
  }

  h3 {
    margin: 0 0 var(--space-2);
    font-size: 15px;
  }

  p {
    margin: 0;
  }

  .text {
    white-space: pre-line;
    overflow-wrap: anywhere;
  }

  .actions,
  .confirm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  input,
  textarea {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    font: inherit;
  }

  .list {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .list li {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }

  .name-button {
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    font-weight: 500;
    text-align: left;
    cursor: pointer;
  }

  .title {
    font-weight: 500;
  }

  .small {
    font-size: 13px;
  }
</style>
