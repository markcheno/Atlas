<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { FileInstance } from '../lib/api/bindings/FileInstance';
  import ProblemNotice from '../lib/components/ProblemNotice.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { formatBytes } from '../lib/format';
  import { count } from '../lib/inventoryCopy';
  import { modifiedText, rangeText, statusLabel } from '../lib/libraryCopy';
  import type { ExtractionState } from '../lib/api/bindings/ExtractionState';
  import OriginalActions from '../lib/components/OriginalActions.svelte';
  import OtherFormat from '../lib/components/OtherFormat.svelte';
  import { typesText } from '../lib/formatsCopy';
  import OriginalNotice from '../lib/components/OriginalNotice.svelte';
  import TextBadge from '../lib/components/TextBadge.svelte';
  import { TEXT_FILTERS } from '../lib/extractionCopy';
  import DuplicatesList from '../lib/components/DuplicatesList.svelte';
  import { details } from '../lib/stores/details.svelte';
  import { duplicates } from '../lib/stores/duplicates.svelte';
  import { fromFile, originals } from '../lib/stores/originals.svelte';
  import { extraction } from '../lib/stores/extraction.svelte';
  import { inventory } from '../lib/stores/inventory.svelte';
  import { library, PAGE_SIZE } from '../lib/stores/library.svelte';
  import { folderName, sources } from '../lib/stores/sources.svelte';

  const scannableRoots = $derived(sources.roots.filter((r) => !r.nestedInRootId));
  const page = $derived(library.result);
  const lastPage = $derived(page ? Math.max(0, Math.ceil(page.matchedSoFar / PAGE_SIZE) - 1) : 0);

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

  const showingDuplicates = $derived(library.mode === 'duplicates');
  // The filters both lists share; each list keeps its own values.
  const filters = $derived(showingDuplicates ? duplicates : library);

  function setMode(mode: 'files' | 'duplicates') {
    library.mode = mode;
    if (mode === 'duplicates') void duplicates.load();
    else void library.load();
  }

  onMount(() => {
    if (showingDuplicates) void duplicates.load();
    else void library.load();
  });

  // Refresh the visible page as cataloging or reading makes progress
  // (throttled).
  // Both are read (no short-circuit), so either one changing refreshes.
  $effect(() => {
    const progress = [inventory.overview, extraction.overview];
    if (progress.some(Boolean)) {
      untrack(() => (showingDuplicates ? duplicates.refreshLive() : library.refreshLive()));
    }
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
      <fieldset class="mode">
        <legend class="visually-hidden">Show</legend>
        <label>
          <input
            type="radio"
            name="library-mode"
            value="files"
            checked={!showingDuplicates}
            onchange={() => setMode('files')}
          />
          Files
        </label>
        <label>
          <input
            type="radio"
            name="library-mode"
            value="duplicates"
            checked={showingDuplicates}
            onchange={() => setMode('duplicates')}
          />
          Duplicates
        </label>
      </fieldset>
      {#if !showingDuplicates}
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
      {/if}
      <label>
        <span class="muted small">Folder</span>
        <select
          value={filters.sourceRootId ?? ''}
          onchange={(e) => filters.setFilters({ sourceRootId: e.currentTarget.value || null })}
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
          value={filters.fileType ?? ''}
          onchange={(e) =>
            filters.setFilters({
              fileType: (e.currentTarget.value || null) as 'pdf' | 'epub' | null,
            })}
        >
          <option value="">All types</option>
          <option value="pdf">PDF</option>
          <option value="epub">EPUB</option>
        </select>
      </label>
      {#if !showingDuplicates}
        <label>
          <span class="muted small">Text status</span>
          <select
            value={library.textStatus ?? ''}
            onchange={(e) =>
              library.setFilters({
                textStatus: (e.currentTarget.value || null) as ExtractionState | null,
              })}
          >
            <option value="">Any text status</option>
            {#each TEXT_FILTERS as f (f.value)}
              <option value={f.value}>{f.label}</option>
            {/each}
          </select>
        </label>
      {/if}
      <label class="check">
        <input
          type="checkbox"
          checked={filters.includeMissing}
          onchange={(e) => filters.setFilters({ includeMissing: e.currentTarget.checked })}
        />
        Include missing files
      </label>
    </div>

    {#if showingDuplicates}
      <DuplicatesList />
    {:else if library.error}
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
            <th scope="col">Text</th>
            <th scope="col"><span class="visually-hidden">Actions</span></th>
          </tr>
        </thead>
        <tbody>
          {#each page.items as item (item.id)}
            <tr>
              <th scope="row" class="name">
                <button
                  type="button"
                  class="name-button"
                  aria-label={`Details for ${item.displayName}`}
                  onclick={(e) => details.open(item.id, e.currentTarget)}
                >
                  {item.displayName}
                </button>
                <span class="path">{item.displayPath}</span>
                {#if item.otherFormat}<OtherFormat other={item.otherFormat} />{/if}
              </th>
              <td>{typesText(item.fileType, item.otherFormat)}</td>
              <td>{rootName(item)}</td>
              <td>{item.sizeBytes === null ? 'Unknown' : formatBytes(item.sizeBytes)}</td>
              <td>{modifiedText(item.modifiedAt)}</td>
              <td>
                <span class="badge" data-status={item.availability}>{statusLabel(item)}</span>
              </td>
              <td><TextBadge {item} /></td>
              <td><OriginalActions item={fromFile(item)} /></td>
            </tr>
            {#if originals.hasNotice(item.id)}
              <tr class="detail">
                <td colspan="8"><OriginalNotice item={fromFile(item)} /></td>
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

  .mode {
    display: inline-flex;
    gap: 0;
    margin: 0;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    min-height: 40px;
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

  .name-button {
    justify-self: start;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .name-button:hover {
    text-decoration: underline;
  }

  .path {
    color: var(--text-muted);
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 11px;
    font-weight: 400;
    overflow-wrap: anywhere;
  }

  .detail td {
    background: var(--surface-muted);
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
