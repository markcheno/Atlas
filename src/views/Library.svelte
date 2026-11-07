<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { FileInstance } from '../lib/api/bindings/FileInstance';
  import ProblemNotice from '../lib/components/ProblemNotice.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { formatBytes } from '../lib/format';
  import { count } from '../lib/inventoryCopy';
  import {
    downloadPrompt,
    modifiedText,
    rangeText,
    revealLabel,
    statusLabel,
  } from '../lib/libraryCopy';
  import { appInfo } from '../lib/stores/app.svelte';
  import { inventory } from '../lib/stores/inventory.svelte';
  import { library, PAGE_SIZE } from '../lib/stores/library.svelte';
  import { folderName, sources } from '../lib/stores/sources.svelte';

  const scannableRoots = $derived(sources.roots.filter((r) => !r.nestedInRootId));
  const page = $derived(library.result);
  const lastPage = $derived(page ? Math.max(0, Math.ceil(page.matchedSoFar / PAGE_SIZE) - 1) : 0);
  const reveal = $derived(revealLabel(appInfo.status?.os));

  function rootName(item: FileInstance): string {
    const root = sources.roots.find((r) => r.id === item.sourceRootId);
    return root ? folderName(root.displayPath) : '';
  }

  function ariaSort(column: 'name' | 'modified'): 'ascending' | 'descending' | 'none' {
    const s = library.sort;
    if (column === 'name') {
      return s === 'name_asc' ? 'ascending' : s === 'name_desc' ? 'descending' : 'none';
    }
    return s === 'modified_asc' ? 'ascending' : s === 'modified_desc' ? 'descending' : 'none';
  }

  onMount(() => {
    void library.load();
  });

  // Refresh the visible page as cataloging makes progress (throttled).
  $effect(() => {
    if (inventory.overview) untrack(() => library.refreshLive());
  });
</script>

<div class="view">
  <ViewHeading>Library</ViewHeading>

  {#if sources.roots.length === 0}
    <div class="empty">
      <p>Choose a folder to start discovering your library.</p>
    </div>
  {:else}
    <div class="toolbar" role="search">
      <label class="search">
        <span class="visually-hidden">Search file names and folders</span>
        <input
          type="search"
          placeholder="Search file names and folders"
          value={library.text}
          oninput={(e) => library.setText(e.currentTarget.value)}
          onkeydown={(e) => {
            if (e.key === 'Escape') library.setText('', true);
          }}
        />
      </label>
      <label>
        <span class="muted small">Folder</span>
        <select
          value={library.sourceRootId ?? ''}
          onchange={(e) => library.setFilters({ sourceRootId: e.currentTarget.value || null })}
        >
          <option value="">All folders</option>
          {#each scannableRoots as root (root.id)}
            <option value={root.id}>{folderName(root.displayPath)}</option>
          {/each}
        </select>
      </label>
      <label>
        <span class="muted small">Type</span>
        <select
          value={library.fileType ?? ''}
          onchange={(e) =>
            library.setFilters({
              fileType: (e.currentTarget.value || null) as 'pdf' | 'epub' | null,
            })}
        >
          <option value="">All types</option>
          <option value="pdf">PDF</option>
          <option value="epub">EPUB</option>
        </select>
      </label>
      <label class="check">
        <input
          type="checkbox"
          checked={library.includeMissing}
          onchange={(e) => library.setFilters({ includeMissing: e.currentTarget.checked })}
        />
        Include missing files
      </label>
    </div>

    {#if library.error}
      <ProblemNotice
        title="Atlas couldn't list your files"
        message={library.error.message}
        recovery={library.error.recovery}
      />
    {:else if page && page.items.length === 0}
      <div class="empty">
        {#if library.text.trim()}
          <p>No files match “{library.text.trim()}”{page.scanning ? ' yet' : ''}.</p>
        {:else if (inventory.overview?.catalogedFiles ?? 0) === 0}
          <p>
            {page.scanning
              ? 'Atlas is cataloging your folders. Files will appear here as they are found.'
              : "Atlas hasn't found any PDF or EPUB files in your folders yet."}
          </p>
        {:else}
          <p>No files match these filters.</p>
        {/if}
      </div>
    {:else if page}
      <table>
        <caption class="visually-hidden">
          Cataloged files, {count(page.matchedSoFar, 'match', 'matches')}
        </caption>
        <thead>
          <tr>
            <th scope="col" aria-sort={ariaSort('name')}>
              <button type="button" class="sort" onclick={() => library.toggleSort('name')}>
                Name
              </button>
            </th>
            <th scope="col">Type</th>
            <th scope="col">Folder</th>
            <th scope="col">Size</th>
            <th scope="col" aria-sort={ariaSort('modified')}>
              <button type="button" class="sort" onclick={() => library.toggleSort('modified')}>
                Modified
              </button>
            </th>
            <th scope="col">Status</th>
            <th scope="col"><span class="visually-hidden">Actions</span></th>
          </tr>
        </thead>
        <tbody>
          {#each page.items as item (item.id)}
            <tr>
              <th scope="row" class="name">
                <span>{item.displayName}</span>
                <span class="path">{item.displayPath}</span>
              </th>
              <td>{item.fileType.toUpperCase()}</td>
              <td>{rootName(item)}</td>
              <td>{item.sizeBytes === null ? 'Unknown' : formatBytes(item.sizeBytes)}</td>
              <td>{modifiedText(item.modifiedAt)}</td>
              <td>
                <span class="badge" data-status={item.availability}>{statusLabel(item)}</span>
              </td>
              <td class="actions">
                <button
                  type="button"
                  class="row-button"
                  disabled={item.availability !== 'available'}
                  aria-label={`Open ${item.displayName}`}
                  onclick={() => library.open(item)}
                >
                  Open
                </button>
                <button
                  type="button"
                  class="row-button"
                  disabled={item.availability === 'unreachable'}
                  aria-label={`${reveal}: ${item.displayName}`}
                  onclick={() => library.reveal(item)}
                >
                  {reveal}
                </button>
              </td>
            </tr>
            {#if library.confirmDownloadId === item.id}
              <tr class="detail">
                <td colspan="7">
                  <div class="confirm" role="group" aria-label="Confirm download">
                    <span>{downloadPrompt(item)}</span>
                    <button
                      type="button"
                      class="button primary"
                      onclick={() => library.open(item, true)}
                    >
                      Download and open
                    </button>
                    <button
                      type="button"
                      class="button quiet"
                      onclick={() => (library.confirmDownloadId = null)}
                    >
                      Cancel
                    </button>
                  </div>
                </td>
              </tr>
            {/if}
            {#if library.rowError?.id === item.id}
              <tr class="detail">
                <td colspan="7">
                  <ProblemNotice
                    title={`Couldn't open ${item.displayName}`}
                    message={library.rowError.error.message}
                    recovery={library.rowError.error.recovery}
                  />
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>

      <nav class="pager" aria-label="Pages">
        <span class="muted" role="status" data-testid="range">
          {rangeText(page.page, PAGE_SIZE, page.items.length, page.matchedSoFar, page.scanning)}
        </span>
        <div>
          <button
            type="button"
            class="button"
            disabled={library.page === 0}
            onclick={() => library.goTo(library.page - 1)}
          >
            Previous
          </button>
          <button
            type="button"
            class="button"
            disabled={library.page >= lastPage}
            onclick={() => library.goTo(library.page + 1)}
          >
            Next
          </button>
        </div>
      </nav>
    {:else}
      <p class="muted" role="status">Loading library…</p>
    {/if}
  {/if}
</div>

<style>
  .view {
    display: grid;
    gap: var(--space-5);
    max-width: 1200px;
    margin: 0 auto;
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    gap: var(--space-4);
  }

  .toolbar label {
    display: grid;
    gap: var(--space-1);
  }

  .toolbar .search {
    flex: 1 1 280px;
  }

  .toolbar input[type='search'],
  .toolbar select {
    min-height: 40px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .toolbar .check {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 40px;
  }

  .empty {
    padding: var(--space-6);
    border: 1px dashed var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 14px;
  }

  th,
  td {
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--border);
    text-align: left;
    vertical-align: middle;
  }

  thead th {
    color: var(--text-muted);
    font-weight: 500;
    font-size: 13px;
  }

  .sort {
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  th[aria-sort='ascending'] .sort::after {
    content: ' ↑';
  }

  th[aria-sort='descending'] .sort::after {
    content: ' ↓';
  }

  .name {
    display: grid;
    font-weight: 500;
    min-width: 240px;
  }

  .path {
    color: var(--text-muted);
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 11px;
    font-weight: 400;
    overflow-wrap: anywhere;
  }

  .badge {
    padding: 2px var(--space-2);
    border-radius: 999px;
    background: var(--accent-soft);
    font-size: 12px;
    white-space: nowrap;
  }

  .badge[data-status='missing'],
  .badge[data-status='unreachable'] {
    background: var(--notice-bg);
  }

  .actions {
    white-space: nowrap;
  }

  .row-button {
    min-height: 30px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    cursor: pointer;
  }

  .row-button:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .detail td {
    background: var(--surface-muted);
  }

  .confirm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
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
    font-size: 12px;
  }
</style>
