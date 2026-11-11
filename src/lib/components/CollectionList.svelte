<script lang="ts">
  // Interests and projects (Slice 3.6, phase3-spec §5.4): active first,
  // then finished, then archived; New interest and New project; each opens
  // its page.
  import { onMount, tick } from 'svelte';
  import type { CollectionKind } from '../api/bindings/CollectionKind';
  import {
    COLLECTIONS_EMPTY,
    countsText,
    kindLabel,
    NEW_INTEREST,
    NEW_PROJECT,
    statusLabel,
  } from '../collectionCopy';
  import { collections } from '../stores/collections.svelte';
  import { NOTE_MAX, TITLE_MAX } from '../trailCopy';
  import CollectionDetail from './CollectionDetail.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  let creating = $state<CollectionKind | null>(null);
  let name = $state('');
  let description = $state('');
  let goal = $state('');

  const groups = $derived.by(() => {
    const all = collections.all ?? [];
    return [
      { label: 'Active', items: all.filter((c) => c.status === 'active') },
      { label: 'Finished', items: all.filter((c) => c.status === 'finished') },
      { label: 'Archived', items: all.filter((c) => c.status === 'archived') },
    ].filter((g) => g.items.length > 0);
  });

  onMount(() => {
    if (!collections.selectedId) void collections.load();
  });

  async function start(kind: CollectionKind) {
    creating = kind;
    name = '';
    description = '';
    goal = '';
    await tick();
    document.getElementById('collection-name')?.focus();
  }

  async function create(event: SubmitEvent) {
    event.preventDefault();
    if (!creating) return;
    const made = await collections.save({
      id: null,
      kind: creating,
      name,
      description: description || null,
      goal: creating === 'project' ? goal || null : null,
      status: null,
    });
    if (made) {
      creating = null;
      await collections.open(made.id);
    }
  }
</script>

{#if collections.selectedId}
  <CollectionDetail />
{:else}
  <section class="collections" aria-label="Interests and projects">
    <div class="actions">
      <button type="button" class="button" onclick={() => void start('interest')}>
        {NEW_INTEREST}
      </button>
      <button type="button" class="button" onclick={() => void start('project')}>
        {NEW_PROJECT}
      </button>
    </div>

    {#if creating}
      <form class="form" onsubmit={(e) => void create(e)}>
        <label for="collection-name">Name of the new {creating}</label>
        <input id="collection-name" type="text" maxlength={TITLE_MAX} bind:value={name} />
        <label for="collection-description">Description (optional)</label>
        <textarea id="collection-description" rows="2" maxlength={NOTE_MAX} bind:value={description}
        ></textarea>
        {#if creating === 'project'}
          <label for="collection-goal">Goal (optional)</label>
          <textarea id="collection-goal" rows="2" maxlength={NOTE_MAX} bind:value={goal}></textarea>
        {/if}
        <div class="actions">
          <button type="submit" class="button primary">Create</button>
          <button type="button" class="button quiet" onclick={() => (creating = null)}>
            Cancel
          </button>
        </div>
      </form>
    {/if}

    {#if collections.actionError}
      <ProblemNotice
        title="Atlas couldn't save that"
        message={collections.actionError.message}
        recovery={collections.actionError.recovery}
      />
    {/if}

    {#if collections.error}
      <ProblemNotice
        title="Atlas couldn't list your interests and projects"
        message={collections.error.message}
        recovery={collections.error.recovery}
      />
    {:else if !collections.all}
      <p class="muted" role="status">Loading…</p>
    {:else if collections.all.length === 0}
      <div class="empty"><p>{COLLECTIONS_EMPTY}</p></div>
    {:else}
      {#each groups as g (g.label)}
        <section aria-labelledby={`collections-${g.label}`}>
          <h3 id={`collections-${g.label}`}>{g.label}</h3>
          <ul class="list">
            {#each g.items as c (c.id)}
              <li data-testid="collection-row">
                <button
                  type="button"
                  class="name-button"
                  onclick={() => void collections.open(c.id)}
                >
                  {c.name}
                </button>
                <span class="muted small">
                  {kindLabel(c.kind)} · {countsText(c.counts)}{c.status !== 'active'
                    ? ` · ${statusLabel(c.status)}`
                    : ''}
                </span>
              </li>
            {/each}
          </ul>
        </section>
      {/each}
    {/if}
  </section>
{/if}

<style>
  .collections,
  .form {
    display: grid;
    gap: var(--space-3);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
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

  h3 {
    margin: 0 0 var(--space-2);
    font-size: 14px;
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
    gap: var(--space-1);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
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

  .name-button:hover {
    text-decoration: underline;
  }

  .empty {
    padding: var(--space-6);
    border: 1px dashed var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  p {
    margin: 0;
  }

  .small {
    font-size: 13px;
  }
</style>
