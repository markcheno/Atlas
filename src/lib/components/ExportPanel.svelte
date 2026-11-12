<script lang="ts">
  // Settings › Export and import (Slice 3.8, phase3-spec §5.7, §5.8):
  // Export… opens the dialog with everything; Import from a file… shows
  // Rust's file dialog, then what the file would add, and imports only on
  // Import. It never removes anything.
  import { atlas, AtlasApiError, toApiError } from '../api/client';
  import type { ImportPreview } from '../api/bindings/ImportPreview';
  import {
    EXPORT_EVERYTHING,
    EXPORT_IMPORT_NOTE,
    IMPORT,
    IMPORT_FROM_FILE,
    IMPORT_NEVER_REMOVES,
    IMPORTING,
    importPreviewText,
    importResult,
  } from '../exportCopy';
  import { exportDialog } from '../stores/exportDialog.svelte';
  import { trails } from '../stores/trails.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  let preview = $state<ImportPreview | null>(null);
  let busy = $state(false);
  let applying = $state(false);
  let result = $state('');
  let error = $state<AtlasApiError | null>(null);

  async function choose() {
    busy = true;
    error = null;
    result = '';
    preview = null;
    try {
      preview = await atlas.previewImport();
    } catch (err) {
      error = err instanceof AtlasApiError ? err : toApiError('preview_import', err);
    } finally {
      busy = false;
    }
  }

  async function apply() {
    if (!preview) return;
    busy = true;
    applying = true;
    error = null;
    try {
      const report = await atlas.applyImport(preview.token);
      result = importResult(report.summary);
      preview = null;
      await trails.refresh();
    } catch (err) {
      error = err instanceof AtlasApiError ? err : toApiError('apply_import', err);
      preview = null;
    } finally {
      busy = false;
      applying = false;
    }
  }
</script>

<p>{EXPORT_IMPORT_NOTE}</p>
<div class="actions">
  <button type="button" class="button" onclick={(e) => exportDialog.show(null, e.currentTarget)}>
    {EXPORT_EVERYTHING}
  </button>
  <button type="button" class="button" disabled={busy} onclick={() => void choose()}>
    {IMPORT_FROM_FILE}
  </button>
</div>

{#if preview}
  <div class="preview" role="group" aria-label="Import {preview.fileName}">
    <p data-testid="import-preview">
      <strong>{preview.fileName}</strong>: {importPreviewText(preview)}
    </p>
    <p class="muted small">{IMPORT_NEVER_REMOVES}</p>
    <div class="actions">
      <button type="button" class="button primary" disabled={busy} onclick={() => void apply()}>
        {IMPORT}
      </button>
      <button type="button" class="button quiet" onclick={() => (preview = null)}>Cancel</button>
    </div>
  </div>
{/if}
{#if applying}
  <p class="muted small" role="status">{IMPORTING}</p>
{/if}
{#if result}
  <p role="status" class="small" data-testid="import-result">{result}</p>
{/if}
{#if error}
  <ProblemNotice
    title="Atlas couldn't import that"
    message={error.message}
    recovery={error.recovery}
  />
{/if}

<style>
  p {
    margin: 0 0 var(--space-2);
    line-height: 1.5;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }

  .preview {
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }

  .small {
    font-size: 13px;
  }
</style>
