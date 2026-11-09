<script lang="ts">
  // The document detail panel (phase1-spec §5.3): what Atlas knows about a
  // file and the document read from it, its text status in plain words,
  // and the other places the same content is (Also at). Escape or Close
  // returns focus to whatever opened it.
  import type { FileInstance } from '../api/bindings/FileInstance';
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
  import TextBadge from './TextBadge.svelte';

  let panel = $state<HTMLElement>();
  let closeButton = $state<HTMLButtonElement>();

  const d = $derived(details.detail);
  const heading = $derived(d ? d.title : 'Details');

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
        <OriginalActions item={fromFile(file)} />
        {#if file.availability !== 'available' || file.storage === 'cloud_only'}
          <span class="badge" data-status={file.availability}>{statusLabel(file)}</span>
        {/if}
      </div>
      <OriginalNotice item={fromFile(file)} />

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
                  <OriginalActions item={fromFile(copy)} />
                </div>
                <OriginalNotice item={fromFile(copy)} />
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    {:else}
      <p class="muted" role="status">Loading details…</p>
    {/if}
  </div>
{/if}

<style>
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
