<script lang="ts">
  // A recorded or saved source (Slices 3.4, 3.5; phase3-spec §5.1, §5.3):
  // its title and place as they were, what became of it, and Details (or
  // Open) when it is still in the library. What is gone says so, with the
  // excerpt saved. With `saveAs`, it can be bookmarked.
  import type { Anchor } from '../api/bindings/Anchor';
  import type { OpenVia } from '../api/bindings/OpenVia';
  import type { ResolvedAnchor } from '../api/bindings/ResolvedAnchor';
  import type { SaveTarget } from '../api/bindings/SaveTarget';
  import { FROM_TRAIL } from '../openVia';
  import { openableFile, openRecorded } from '../trailOpen';
  import { anchorNote, placeText, recordedSourceName } from '../trailCopy';
  import OriginalActions from './OriginalActions.svelte';
  import SaveAction from './SaveAction.svelte';
  import { pageView } from '../stores/pageView.svelte';
  import { SHOW_PAGE } from '../readerCopy';

  let {
    anchor,
    resolved,
    n,
    cited = false,
    at,
    opens = false,
    id,
    via = FROM_TRAIL,
    saveAs,
  }: {
    anchor: Anchor;
    resolved: ResolvedAnchor | undefined;
    /** A question's source number. */
    n?: number;
    cited?: boolean;
    /** When the step was recorded. */
    at: string;
    /** Offer Open and Show as well as Details (a source opened). */
    opens?: boolean;
    id?: string;
    /** How opening it is recorded: from a trail, or a bookmark. */
    via?: OpenVia;
    /** When set, a Bookmark button keeps this. */
    saveAs?: SaveTarget;
  } = $props();

  const name = $derived(n ? recordedSourceName(n, anchor) : anchor.title);
  const place = $derived(placeText(anchor));
  const note = $derived(resolved ? anchorNote(resolved) : null);
  const fileId = $derived(openableFile(resolved));
  const gone = $derived(!fileId);
  const saveKey = $derived(
    saveAs?.kind === 'step' ? `bookmark-${saveAs.stepId}-${saveAs.index}` : `bookmark-${id ?? ''}`,
  );
</script>

<!-- Focused (never tabbed to) when a chip names a source that has gone. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="source" {id} tabindex={id ? -1 : undefined} data-testid="trail-source">
  <div class="head">
    {#if n}<span class="n" aria-hidden="true">{n}</span>{/if}
    <span class="title">{anchor.title}</span>
    {#if place}<span class="muted small">{place}</span>{/if}
    {#if cited}<span class="badge">Cited</span>{/if}
    <span class="actions">
      {#if fileId}
        <button
          type="button"
          class="row-button"
          aria-label={`Details for ${name}`}
          onclick={(e) =>
            openRecorded(anchor, resolved, e.currentTarget, {
              question: n ? { n, at } : undefined,
              via,
              saveAs,
            })}
        >
          Details
        </button>
        {#if anchor.fileType === 'pdf' && (resolved?.pageNo ?? anchor.pageNo)}
          {@const pageNo = (resolved?.pageNo ?? anchor.pageNo) as number}
          <button
            type="button"
            class="row-button"
            aria-label={`${SHOW_PAGE}: ${name}`}
            onclick={(e) =>
              void pageView.show(
                { fileId, pageNo, find: anchor.excerpt ?? null, title: anchor.title, via },
                e.currentTarget,
              )}
          >
            {SHOW_PAGE}
          </button>
        {/if}
        {#if opens}
          <OriginalActions
            item={{ id: fileId, displayName: anchor.title, availability: 'available', via }}
          />
        {/if}
      {/if}
      {#if saveAs}
        <SaveAction target={saveAs} key={saveKey} {name} />
      {/if}
    </span>
  </div>
  {#if note}
    <p class="small" data-testid="anchor-note" data-state={resolved?.state}>{note}</p>
  {/if}
  {#if anchor.excerpt}
    {#if gone}
      <p class="excerpt">{anchor.excerpt}</p>
    {:else}
      <details>
        <summary>{n ? 'The passage sent' : 'The passage then'}</summary>
        <p class="excerpt">{anchor.excerpt}</p>
      </details>
    {/if}
  {/if}
</div>

<style>
  .source {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-3);
  }

  .n {
    min-width: 1.6em;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .title {
    font-weight: 500;
    overflow-wrap: anywhere;
  }

  .actions {
    display: inline-flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-left: auto;
  }

  p {
    margin: 0;
  }

  summary {
    cursor: pointer;
    font-size: 13px;
  }

  .excerpt {
    margin: var(--space-2) 0 0;
    white-space: pre-line;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }

  .small {
    font-size: 13px;
  }
</style>
