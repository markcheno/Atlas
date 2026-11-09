<script lang="ts">
  // Library › Duplicates (Slice 1.6): groups of files with identical
  // content, most extra space first. Atlas never deletes or moves a copy;
  // each one can be opened, shown in its folder, or inspected.
  import type { FileInstance } from '../api/bindings/FileInstance';
  import { formatBytes } from '../format';
  import { count } from '../inventoryCopy';
  import { statusLabel } from '../libraryCopy';
  import { TITLE_SOURCE_NOTE } from '../searchCopy';
  import { duplicatesSummary } from '../storageCopy';
  import { details } from '../stores/details.svelte';
  import { duplicates, DUPLICATES_PAGE_SIZE } from '../stores/duplicates.svelte';
  import { fromFile, originals } from '../stores/originals.svelte';
  import { folderName, sources } from '../stores/sources.svelte';
  import OriginalActions from './OriginalActions.svelte';
  import OriginalNotice from './OriginalNotice.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  const page = $derived(duplicates.result);
  const lastPage = $derived(
    page ? Math.max(0, Math.ceil(page.totalGroups / DUPLICATES_PAGE_SIZE) - 1) : 0,
  );

  function rootName(file: FileInstance): string {
    const root = sources.roots.find((r) => r.id === file.sourceRootId);
    return root ? folderName(root.displayPath) : '';
  }
</script>

{#if duplicates.error}
  <ProblemNotice
    title="Atlas couldn't list duplicates"
    message={duplicates.error.message}
    recovery={duplicates.error.recovery}
  />
{:else if page && page.groups.length === 0 && page.totalGroups === 0}
  <div class="empty">
    <p>No duplicates found.</p>
    <p class="muted">
      Atlas finds copies by their contents once it has read them, whatever their names.
    </p>
  </div>
{:else if page}
  <div class="intro">
    <p role="status" data-testid="duplicates-summary">
      {duplicatesSummary(page.totalGroups, page.extraBytes)}
    </p>
    <p class="muted small">
      Atlas never deletes or moves your files. Open a copy or show it in its folder to tidy up
      yourself.
    </p>
  </div>
  <ol class="groups" aria-busy={duplicates.loading}>
    {#each page.groups as group (group.documentId)}
      {@const available = group.copies.filter((c) => c.availability !== 'missing').length}
      {@const source = TITLE_SOURCE_NOTE[group.titleSource]}
      <li data-testid="duplicate-group">
        <h3>{group.title}</h3>
        <p class="meta muted small">
          <span>{count(available, 'copy', 'copies')}</span>
          <span>{formatBytes(group.sizeBytes)} each</span>
          <span>{formatBytes(group.extraBytes)} in extra copies</span>
          {#if source}<span>{source}</span>{/if}
        </p>
        <ul aria-label={`Copies of ${group.title}`}>
          {#each group.copies as copy (copy.id)}
            <li>
              <div class="copy">
                <span class="name">{copy.displayName}</span>
                <span class="muted small">{rootName(copy)}</span>
                <span class="path">{copy.displayPath}</span>
              </div>
              <div class="actions">
                {#if copy.availability !== 'available' || copy.storage === 'cloud_only'}
                  <span class="badge" data-status={copy.availability}>{statusLabel(copy)}</span>
                {/if}
                <button
                  type="button"
                  class="row-button"
                  aria-label={`Details for ${copy.displayName}`}
                  onclick={(e) => details.open(copy.id, e.currentTarget)}
                >
                  Details
                </button>
                <OriginalActions item={fromFile(copy)} />
              </div>
              {#if originals.hasNotice(copy.id)}
                <OriginalNotice item={fromFile(copy)} />
              {/if}
            </li>
          {/each}
        </ul>
      </li>
    {/each}
  </ol>

  <nav class="pager" aria-label="Duplicate pages">
    <span class="muted" data-testid="duplicates-range">
      {#if page.groups.length > 0}
        Showing {page.page * DUPLICATES_PAGE_SIZE + 1}–{page.page * DUPLICATES_PAGE_SIZE +
          page.groups.length} of {count(page.totalGroups, 'group')}
      {/if}
    </span>
    <div>
      <button
        type="button"
        class="button"
        disabled={duplicates.page === 0}
        onclick={() => duplicates.goTo(duplicates.page - 1)}
      >
        Previous
      </button>
      <button
        type="button"
        class="button"
        disabled={duplicates.page >= lastPage}
        onclick={() => duplicates.goTo(duplicates.page + 1)}
      >
        Next
      </button>
    </div>
  </nav>
{:else}
  <p class="muted" role="status">Looking for duplicates…</p>
{/if}

<style>
  .empty {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-6);
    border: 1px dashed var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  .empty p,
  .intro p {
    margin: 0;
  }

  .intro {
    display: grid;
    gap: var(--space-1);
  }

  .groups {
    display: grid;
    gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .groups > li {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  h3 {
    margin: 0;
    font-size: 16px;
    overflow-wrap: anywhere;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-3);
    margin: 0;
  }

  ul {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  ul li {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius);
    background: var(--surface-muted);
  }

  .copy {
    display: grid;
  }

  .name {
    font-weight: 500;
  }

  .path {
    color: var(--text-muted);
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 11px;
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .pager {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-4);
  }

  .pager div {
    display: flex;
    gap: var(--space-2);
  }

  .small {
    font-size: 13px;
  }
</style>
