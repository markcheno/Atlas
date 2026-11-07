<script lang="ts">
  import { onMount } from 'svelte';
  import { atlas, AtlasApiError, toApiError } from '../lib/api/client';
  import type { DocumentPage } from '../lib/api/bindings/DocumentPage';
  import NotImplementedNotice from '../lib/components/NotImplementedNotice.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';

  type LoadState =
    | { kind: 'loading' }
    | { kind: 'loaded'; page: DocumentPage }
    | { kind: 'error'; error: AtlasApiError };

  let load = $state<LoadState>({ kind: 'loading' });

  onMount(() => {
    atlas
      .listDocuments({
        text: null,
        sourceRootId: null,
        fileType: null,
        sort: 'name_asc',
        page: 0,
        pageSize: 50,
      })
      .then(
        (page) => (load = { kind: 'loaded', page }),
        (err: unknown) =>
          (load = {
            kind: 'error',
            error: err instanceof AtlasApiError ? err : toApiError('list_documents', err),
          }),
      );
  });
</script>

<div class="view">
  <ViewHeading>Library</ViewHeading>

  {#if load.kind === 'loading'}
    <p class="muted" role="status">Loading library…</p>
  {:else if load.kind === 'error'}
    <div class="empty">
      <p>Choose a folder to start discovering your library.</p>
      <NotImplementedNotice title="Cataloging and browsing files" error={load.error} />
    </div>
  {:else if load.page.items.length === 0}
    <div class="empty">
      <p>Choose a folder to start discovering your library.</p>
    </div>
  {:else}
    <!-- The real table arrives with Slice 0.6; the core cannot return rows before then. -->
    <p>{load.page.matchedSoFar} files cataloged so far.</p>
  {/if}
</div>

<style>
  .view {
    display: grid;
    gap: var(--space-5);
    max-width: 960px;
    margin: 0 auto;
  }

  .empty {
    display: grid;
    gap: var(--space-4);
    padding: var(--space-6);
    border: 1px dashed var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }
</style>
