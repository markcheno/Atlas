<script lang="ts">
  // Export… (Slice 3.8, phase3-spec §5.8): what (the item it was opened
  // from, or everything), the format, and for Markdown the link style. Then
  // Rust's folder dialog, starting at the last folder used. The result says
  // where Atlas wrote and what. Escape closes and returns focus.
  import { tick } from 'svelte';
  import {
    CHOOSE_AND_EXPORT,
    EVERYTHING,
    EXPORT_NOTE,
    EXPORT_TITLE,
    EXPORTING,
    exportResult,
    MARKDOWN_LINKS,
    MARKDOWN_NOTES,
    OBSIDIAN_LINKS,
    ONE_JSON_FILE,
    thisItem,
  } from '../exportCopy';
  import { exportDialog } from '../stores/exportDialog.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  let dialog = $state<HTMLElement>();
  let exportButton = $state<HTMLButtonElement>();

  $effect(() => {
    if (exportDialog.open) void tick().then(() => exportButton?.focus());
  });

  /** Tab stops: buttons and the checked radio of each group. */
  function stops(): HTMLElement[] {
    if (!dialog) return [];
    return [
      ...dialog.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input[type="radio"]:checked:not(:disabled)',
      ),
    ];
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      void exportDialog.close();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusable = stops();
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
</script>

{#if exportDialog.open}
  <div class="backdrop" role="presentation" onclick={() => void exportDialog.close()}></div>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="export-title"
    tabindex="-1"
    bind:this={dialog}
    onkeydown={onKeydown}
  >
    <h2 id="export-title">
      {EXPORT_TITLE}{#if exportDialog.item && !exportDialog.everything}: {exportDialog.item
          .name}{/if}
    </h2>

    {#if exportDialog.item}
      <fieldset>
        <legend class="muted small">What</legend>
        <label class="choice">
          <input
            type="radio"
            name="export-what"
            checked={!exportDialog.everything}
            onchange={() => (exportDialog.everything = false)}
          />
          {thisItem(exportDialog.item.what)}
        </label>
        <label class="choice">
          <input
            type="radio"
            name="export-what"
            checked={exportDialog.everything}
            onchange={() => (exportDialog.everything = true)}
          />
          {EVERYTHING}
        </label>
      </fieldset>
    {/if}

    <fieldset>
      <legend class="muted small">Format</legend>
      <label class="choice">
        <input
          type="radio"
          name="export-format"
          checked={exportDialog.format === 'markdown'}
          onchange={() => (exportDialog.format = 'markdown')}
        />
        {MARKDOWN_NOTES}
      </label>
      <label class="choice">
        <input
          type="radio"
          name="export-format"
          checked={exportDialog.format === 'json'}
          onchange={() => (exportDialog.format = 'json')}
        />
        {ONE_JSON_FILE}
      </label>
    </fieldset>

    {#if exportDialog.format === 'markdown'}
      <fieldset>
        <legend class="muted small">Links between notes</legend>
        <label class="choice">
          <input
            type="radio"
            name="export-links"
            checked={exportDialog.linkStyle === 'obsidian'}
            onchange={() => (exportDialog.linkStyle = 'obsidian')}
          />
          {OBSIDIAN_LINKS}
        </label>
        <label class="choice">
          <input
            type="radio"
            name="export-links"
            checked={exportDialog.linkStyle === 'markdown'}
            onchange={() => (exportDialog.linkStyle = 'markdown')}
          />
          {MARKDOWN_LINKS}
        </label>
      </fieldset>
    {/if}

    <p class="muted small">{EXPORT_NOTE}</p>

    {#if exportDialog.running}
      <p class="muted small" role="status">{EXPORTING}</p>
    {:else if exportDialog.report}
      <p role="status" data-testid="export-result">{exportResult(exportDialog.report)}</p>
    {/if}
    {#if exportDialog.error}
      <ProblemNotice
        title="Atlas couldn't export that"
        message={exportDialog.error.message}
        recovery={exportDialog.error.recovery}
      />
    {/if}

    <div class="actions">
      <button
        type="button"
        class="button primary"
        bind:this={exportButton}
        disabled={exportDialog.running}
        onclick={() => void exportDialog.run()}
      >
        {CHOOSE_AND_EXPORT}
      </button>
      <button type="button" class="button" onclick={() => void exportDialog.close()}>
        {exportDialog.report ? 'Done' : 'Cancel'}
      </button>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgb(0 0 0 / 0.35);
  }

  .dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    z-index: 41;
    display: grid;
    gap: var(--space-3);
    width: min(520px, calc(100vw - 2 * var(--space-5)));
    max-height: calc(100vh - 2 * var(--space-5));
    overflow: auto;
    padding: var(--space-5);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    transform: translate(-50%, -50%);
  }

  h2 {
    margin: 0;
    font-size: 18px;
    overflow-wrap: anywhere;
  }

  p {
    margin: 0;
    line-height: 1.5;
  }

  fieldset {
    display: grid;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    border: 0;
  }

  .choice {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .small {
    font-size: 13px;
  }
</style>
