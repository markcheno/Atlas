<script lang="ts">
  // Related documents in the detail panel (phase2-spec §5.4, Slice 2.5):
  // up to ten documents close in meaning, each with Details, Open and
  // Reveal. Nothing at all when no embedding model is set up. Only stored
  // vectors are compared, so it works while the AI service is down.
  import { untrack } from 'svelte';
  import { atlas, AtlasApiError, toApiError } from '../api/client';
  import type { RelatedHit } from '../api/bindings/RelatedHit';
  import type { RelatedPage } from '../api/bindings/RelatedPage';
  import { statusLabel } from '../libraryCopy';
  import { NOT_INDEXED, relatedBy, relatedNote, relatedPending } from '../relatedCopy';
  import { FROM_RELATED } from '../openVia';
  import { details } from '../stores/details.svelte';
  import { embedding } from '../stores/embedding.svelte';
  import { fromHit, originals } from '../stores/originals.svelte';
  import { folderName, sources } from '../stores/sources.svelte';
  import OriginalActions from './OriginalActions.svelte';
  import OriginalNotice from './OriginalNotice.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  /** How long to wait before asking again while the index opens. */
  const LOADING_RETRY_MS = 1000;
  const LOADING_RETRIES = 60;

  let { documentId }: { documentId: string | null } = $props();

  let page = $state<RelatedPage | null>(null);
  let error = $state<AtlasApiError | null>(null);
  let heading = $state<HTMLElement>();
  let seq = 0;
  let retries = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const modelSetUp = $derived(!!embedding.overview && !embedding.overview.needsModel);
  const note = $derived(page ? relatedNote(page.status, page.hits.length) : null);
  const shown = $derived(
    documentId === null ? modelSetUp : !!error || (!!page && page.status !== 'off'),
  );

  async function load(id: string) {
    const mine = ++seq;
    clearTimeout(timer);
    try {
      const found = await atlas.getRelatedDocuments(id, 10);
      if (mine !== seq) return;
      page = found;
      error = null;
      if (found.status === 'loading' && retries < LOADING_RETRIES) {
        retries++;
        timer = setTimeout(() => void load(id), LOADING_RETRY_MS);
      }
    } catch (err) {
      if (mine !== seq) return;
      error = err instanceof AtlasApiError ? err : toApiError('get_related_documents', err);
    }
  }

  // A new document: start again.
  $effect(() => {
    const id = documentId;
    untrack(() => {
      page = null;
      error = null;
      retries = 0;
      if (id) void load(id);
    });
    return () => {
      seq++;
      clearTimeout(timer);
    };
  });

  // Not indexed yet, or the index was opening: the next embedding-status
  // event is a good moment to ask again.
  $effect(() => {
    if (embedding.overview) {
      untrack(() => {
        if (documentId && page && relatedPending(page.status)) void load(documentId);
      });
    }
  });

  // "More like this": Related takes focus once it's there.
  $effect(() => {
    if (details.focusRelated && shown && heading && (page || error || documentId === null)) {
      untrack(() => {
        details.focusRelated = false;
        heading?.scrollIntoView?.({ block: 'start' });
        heading?.focus();
      });
    }
  });

  function rootName(hit: RelatedHit): string {
    const root = sources.roots.find((r) => r.id === hit.sourceRootId);
    return root ? folderName(root.displayPath) : '';
  }
</script>

{#if shown}
  <section aria-labelledby="details-related" data-testid="related">
    <h3 id="details-related" tabindex="-1" bind:this={heading}>Related</h3>
    {#if documentId === null}
      <p class="muted">{NOT_INDEXED}.</p>
    {:else if error}
      <ProblemNotice
        title="Atlas couldn't find related documents"
        message={error.message}
        recovery={error.recovery}
      />
    {:else if page && page.hits.length > 0}
      <ul>
        {#each page.hits as hit (hit.documentId)}
          <li data-testid="related-hit">
            <div class="related">
              <span class="name">{hit.title}</span>
              <span class="muted small">
                {hit.fileType.toUpperCase()} · {rootName(hit)}
              </span>
              <span class="small">{relatedBy(hit.location)}</span>
            </div>
            <div class="actions">
              {#if hit.availability !== 'available' || hit.storage === 'cloud_only'}
                <span class="badge" data-status={hit.availability}>{statusLabel(hit)}</span>
              {/if}
              <button
                type="button"
                class="button quiet"
                aria-label={`Details for ${hit.title}`}
                onclick={() => void details.show(hit.fileInstanceId)}
              >
                Details
              </button>
              <OriginalActions item={fromHit(hit, FROM_RELATED)} />
            </div>
            {#if originals.hasNotice(hit.fileInstanceId)}
              <OriginalNotice item={fromHit(hit, FROM_RELATED)} />
            {/if}
          </li>
        {/each}
      </ul>
    {:else if note}
      <p class="muted" role="status">{note}</p>
    {/if}
  </section>
{/if}

<style>
  h3 {
    margin: 0 0 var(--space-2);
    font-size: 15px;
  }

  h3:focus {
    outline: none;
  }

  h3:focus-visible {
    outline: 2px solid var(--focus, currentColor);
    outline-offset: 2px;
  }

  ul {
    display: grid;
    gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }

  .related {
    display: grid;
  }

  .name {
    font-weight: 500;
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .small {
    font-size: 12px;
  }

  p {
    margin: 0;
  }
</style>
