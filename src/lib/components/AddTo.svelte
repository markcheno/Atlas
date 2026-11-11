<script lang="ts">
  // Add to… (Slice 3.6, phase3-spec §5.4): the active interests and
  // projects, each a toggle showing whether the item is in it, and "New
  // interest…" / "New project…". Nothing requires one.
  import { tick } from 'svelte';
  import type { CollectionItemRef } from '../api/bindings/CollectionItemRef';
  import type { CollectionKind } from '../api/bindings/CollectionKind';
  import { ADD_TO, kindLabel, NEW_INTEREST, NEW_PROJECT } from '../collectionCopy';
  import { collections } from '../stores/collections.svelte';

  let { item, name, key }: { item: CollectionItemRef; name: string; key: string } = $props();

  let open = $state(false);
  let member = $state<string[]>([]);
  let creating = $state<CollectionKind | null>(null);
  let newName = $state('');

  async function toggleOpen() {
    open = !open;
    creating = null;
    if (open) {
      await collections.load();
      member = await collections.membership(item);
    }
  }

  async function toggle(id: string) {
    const ok = member.includes(id)
      ? await collections.take(id, item)
      : await collections.add(id, item);
    if (ok) member = await collections.membership(item);
  }

  async function startNew(kind: CollectionKind) {
    creating = kind;
    newName = '';
    await tick();
    document.getElementById(`new-collection-${key}`)?.focus();
  }

  async function create(event: SubmitEvent) {
    event.preventDefault();
    if (!creating) return;
    const made = await collections.save({
      id: null,
      kind: creating,
      name: newName,
      description: null,
      goal: null,
      status: null,
    });
    if (made && (await collections.add(made.id, item))) {
      creating = null;
      member = await collections.membership(item);
    }
  }
</script>

<span class="add-to">
  <button
    type="button"
    class="row-button"
    aria-expanded={open}
    aria-label={`${ADD_TO} ${name}`}
    onclick={() => void toggleOpen()}
  >
    {ADD_TO}
  </button>
  {#if open}
    <div class="menu" role="group" aria-label={`Add ${name} to an interest or project`}>
      {#each collections.active as c (c.id)}
        <button
          type="button"
          class="row-button"
          aria-pressed={member.includes(c.id)}
          onclick={() => void toggle(c.id)}
        >
          {c.name} <span class="muted small">{kindLabel(c.kind)}</span>
        </button>
      {/each}
      {#if creating}
        <form class="new" onsubmit={(e) => void create(e)}>
          <label for={`new-collection-${key}`}>Name of the new {creating}</label>
          <input id={`new-collection-${key}`} type="text" maxlength="200" bind:value={newName} />
          <button type="submit" class="button primary">Create and add</button>
          <button type="button" class="button quiet" onclick={() => (creating = null)}>
            Cancel
          </button>
        </form>
      {:else}
        <button type="button" class="row-button" onclick={() => void startNew('interest')}>
          {NEW_INTEREST}
        </button>
        <button type="button" class="row-button" onclick={() => void startNew('project')}>
          {NEW_PROJECT}
        </button>
      {/if}
      {#if collections.actionError}
        <p class="small" role="alert">{collections.actionError.message}</p>
      {/if}
    </div>
  {/if}
</span>

<style>
  .add-to {
    display: inline-grid;
    gap: var(--space-2);
  }

  .menu {
    display: grid;
    gap: var(--space-1);
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .menu button[aria-pressed='true'] {
    font-weight: 600;
  }

  .menu button[aria-pressed='true']::before {
    content: '✓ ';
  }

  .new {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .new input {
    min-height: var(--control-height, 36px);
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  p {
    margin: 0;
  }

  .small {
    font-size: 13px;
  }
</style>
