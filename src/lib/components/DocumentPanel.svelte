<script lang="ts">
  // The document detail panel (phase1-spec §5.3): what Atlas knows about a
  // file and the document read from it, its text status in plain words,
  // and the other places the same content is (Also at). Escape or Close
  // returns focus to whatever opened it. Related documents (Slice 2.5) can
  // be shown in the panel in turn, with Back to the one before. Opened from
  // an Ask Atlas citation, it shows the passage sent first (Slice 2.7).
  // Its document, and the cited passage, can be bookmarked, and its
  // bookmarks are listed (Slice 3.5).
  import type { Bookmark } from '../api/bindings/Bookmark';
  import type { FileInstance } from '../api/bindings/FileInstance';
  import { CITED_PASSAGE, FROM_LAST_QUESTION } from '../answerCopy';
  import { reasonText } from '../extractionCopy';
  import { TITLE_SOURCE_NOTE } from '../searchCopy';
  import { formatBytes } from '../format';
  import { modifiedText, statusLabel } from '../libraryCopy';
  import { details } from '../stores/details.svelte';
  import { fromFile, originals } from '../stores/originals.svelte';
  import { folderName, sources } from '../stores/sources.svelte';
  import OriginalActions from './OriginalActions.svelte';
  import OriginalNotice from './OriginalNotice.svelte';
  import ProblemNotice from './ProblemNotice.svelte';
  import RelatedList from './RelatedList.svelte';
  import TextBadge from './TextBadge.svelte';
  import SaveAction from './SaveAction.svelte';
  import { BOOKMARK_PASSAGE, BOOKMARKS_HERE } from '../savedCopy';
  import { saved } from '../stores/saved.svelte';
  import { placeText } from '../trailCopy';

  let panel = $state<HTMLElement>();
  let closeButton = $state<HTMLButtonElement>();

  const d = $derived(details.detail);
  const heading = $derived(d ? d.title : 'Details');
  const previous = $derived(details.history.at(-1));

  /** This document's bookmarks: refreshed when anything is saved. */
  let here = $state<Bookmark[]>([]);
  $effect(() => {
    const doc = d?.documentId;
    void saved.states;
    if (!doc) {
      here = [];
      return;
    }
    void saved.forDocument(doc).then((b) => {
      if (details.detail?.documentId === doc) here = b;
    });
  });

  function rootName(file: FileInstance): string {
    const root = sources.roots.find((r) => r.id === file.sourceRootId);
    return root ? folderName(root.displayPath) : '';
  }

  function languageName(code: string): string {
    try {
      return new Intl.DisplayNames(['en'], { type: 'language' }).of(code) ?? code;
    } catch {
      return code;
    }
  }

  function close() {
    originals.clear();
    void details.close();
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      close();
      return;
    }
    // Keep Tab inside the panel while it is open.
    if (event.key !== 'Tab' || !panel) return;
    const focusable = [
      ...panel.querySelectorAll<HTMLElement>('button:not(:disabled), [href], [tabindex="0"]'),
    ];
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  $effect(() => {
    if (details.fileId) closeButton?.focus();
  });
</script>

{#if details.fileId}
  <div class="backdrop" aria-hidden="true" onclick={close}></div>
  <div
    class="panel"
    role="dialog"
    aria-modal="true"
    aria-labelledby="details-title"
    tabindex="-1"
    bind:this={panel}
    onkeydown={onKeydown}
  >
    <header>
      <h2 id="details-title">{heading}</h2>
      <button
        type="button"
        class="button quiet"
        bind:this={closeButton}
        aria-label="Close details"
        onclick={close}
      >
        Close
      </button>
    </header>

    {#if previous}
      <button type="button" class="button quiet back" onclick={() => void details.back()}>
        Back to {previous.title}
      </button>
    {/if}

    {#if details.error}
      <ProblemNotice
        title="Atlas couldn't show this file's details"
        message={details.error.message}
        recovery={details.error.recovery}
      />
    {:else if d}
      {@const file = d.file}
      {@const reason = reasonText(file)}
      {@const source = TITLE_SOURCE_NOTE[d.titleSource]}
      {#if d.titleSource !== 'file_name' || source}
        <p class="file-name">
          {#if d.titleSource !== 'file_name'}<span>{file.displayName}</span>{/if}
          {#if source}<span class="source" data-testid="title-source">{source}</span>{/if}
        </p>
      {/if}

      <div class="actions">
        <OriginalActions item={fromFile(file, details.via ?? undefined)} />
        {#if d.documentId}
          <SaveAction
            target={{ kind: 'file', fileId: file.id, place: { kind: 'document' } }}
            key={`bookmark-document-${d.documentId}`}
            name={d.title}
          />
        {/if}
        {#if file.availability !== 'available' || file.storage === 'cloud_only'}
          <span class="badge" data-status={file.availability}>{statusLabel(file)}</span>
        {/if}
      </div>
      <OriginalNotice item={fromFile(file, details.via ?? undefined)} />

      {#if details.cited}
        {@const cited = details.cited}
        <section class="cited" aria-labelledby="cited-title" data-testid="cited-passage">
          <h3 id="cited-title">{CITED_PASSAGE}</h3>
          <p class="cited-meta muted small">
            <span>Source {cited.n}</span>
            {#if cited.location}<span>{cited.location}</span>{/if}
          </p>
          <p class="cited-text">{cited.excerpt}</p>
          <p class="muted small">{details.citedNote ?? FROM_LAST_QUESTION}</p>
          {#if details.saveAs}
            {@const key =
              details.saveAs.kind === 'step'
                ? `${details.saveAs.stepId}-${details.saveAs.index}`
                : `${file.id}-${cited.n}`}
            <div class="actions">
              <SaveAction
                target={details.saveAs}
                key={`bookmark-cited-${key}`}
                label={BOOKMARK_PASSAGE}
              />
              <SaveAction
                finding={{ kind: 'passage', target: details.saveAs }}
                key={`finding-cited-${key}`}
              />
            </div>
          {/if}
        </section>
      {/if}

      <dl>
        {#if d.authors.length > 0}
          <dt>{d.authors.length === 1 ? 'Author' : 'Authors'}</dt>
          <dd>{d.authors.join(', ')}</dd>
        {/if}
        {#if d.language}
          <dt>Language</dt>
          <dd>{languageName(d.language)}</dd>
        {/if}
        {#if d.pageCount !== null}
          <dt>{file.fileType === 'epub' ? 'Chapters' : 'Pages'}</dt>
          <dd>{d.pageCount.toLocaleString('en-US')}</dd>
        {/if}
        <dt>Type</dt>
        <dd>{file.fileType.toUpperCase()}</dd>
        <dt>Size</dt>
        <dd>{file.sizeBytes === null ? 'Unknown' : formatBytes(file.sizeBytes)}</dd>
        <dt>Modified</dt>
        <dd>{modifiedText(file.modifiedAt)}</dd>
        <dt>Folder</dt>
        <dd>{rootName(file)}</dd>
        <dt>Path</dt>
        <dd class="path">{file.displayPath}</dd>
        <dt>Text</dt>
        <dd class="text">
          <TextBadge item={file} />
          {#if reason}
            <span data-testid="text-reason">{reason}</span>
          {:else if file.extraction.state === 'pending'}
            <span data-testid="text-reason">Atlas hasn't read this file yet.</span>
          {/if}
        </dd>
      </dl>

      {#if d.alsoAt.length > 0}
        <section aria-labelledby="details-also-at">
          <h3 id="details-also-at">Also at</h3>
          <ul>
            {#each d.alsoAt as copy (copy.id)}
              <li>
                <div class="copy">
                  <span class="copy-name">{copy.displayName}</span>
                  <span class="muted small">{rootName(copy)}</span>
                  <span class="path">{copy.displayPath}</span>
                </div>
                <div class="actions">
                  <span class="badge" data-status={copy.availability}>{statusLabel(copy)}</span>
                  <OriginalActions item={fromFile(copy, details.via ?? undefined)} />
                </div>
                <OriginalNotice item={fromFile(copy, details.via ?? undefined)} />
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if here.length > 0}
        <section aria-labelledby="details-bookmarks" data-testid="bookmarks-here">
          <h3 id="details-bookmarks">{BOOKMARKS_HERE}</h3>
          <ul class="bookmarks">
            {#each here as b (b.id)}
              <li>
                <span class="small">{placeText(b.anchor) || 'The whole document'}</span>
                {#if b.anchor.excerpt}<span class="muted small excerpt">{b.anchor.excerpt}</span
                  >{/if}
                {#if b.note}<span class="small note">{b.note}</span>{/if}
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      <RelatedList documentId={d.documentId} />
    {:else}
      <p class="muted" role="status">Loading details…</p>
    {/if}
  </div>
{/if}

<style>
  .bookmarks {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .bookmarks li {
    display: grid;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }

  .bookmarks .excerpt {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .bookmarks .note {
    white-space: pre-line;
  }

  .cited {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--notice-border);
    border-radius: var(--radius);
    background: var(--notice-bg);
  }

  .cited h3,
  .cited p {
    margin: 0;
  }

  .cited h3 {
    font-size: 14px;
  }

  .cited-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-3);
  }

  .cited-text {
    max-height: 18em;
    overflow-y: auto;
    white-space: pre-line;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }

  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.25);
    z-index: 20;
  }

  .panel {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 21;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: min(480px, 100vw);
    padding: var(--space-5);
    overflow-y: auto;
    background: var(--surface);
    border-left: 1px solid var(--border);
    box-shadow: -8px 0 24px rgb(0 0 0 / 0.12);
  }

  header {
    display: flex;
    justify-content: space-between;
    align-items: start;
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

  .back {
    align-self: start;
    margin-top: calc(-1 * var(--space-2));
    overflow-wrap: anywhere;
    text-align: start;
  }

  .file-name {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-3);
    margin: calc(-1 * var(--space-3)) 0 0;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .source {
    font-size: 12px;
    font-style: italic;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-2) var(--space-4);
    margin: 0;
  }

  dt {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  dd.text {
    display: grid;
    gap: var(--space-1);
    justify-items: start;
  }

  .path {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    overflow-wrap: anywhere;
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

  .copy {
    display: grid;
  }

  .copy-name {
    font-weight: 500;
  }

  .small {
    font-size: 12px;
  }
</style>
