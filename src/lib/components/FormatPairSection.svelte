<script lang="ts">
  // The detail panel's two formats (Slice 4.5.1): the other format beside
  // this file, whether Atlas counts them as one book and why, and the
  // owner's say. Instant and reversible, so it asks nothing first.
  import type { FormatPair } from '../api/bindings/FormatPair';
  import { NOT_SAME, SAME, pairNote } from '../formatsCopy';
  import { statusLabel } from '../libraryCopy';
  import { details } from '../stores/details.svelte';
  import { fromFile } from '../stores/originals.svelte';
  import OriginalActions from './OriginalActions.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  let { pair }: { pair: FormatPair } = $props();
  const partner = $derived(pair.partner);
</script>

<section aria-labelledby="details-formats" data-testid="format-pair">
  <h3 id="details-formats">Other format</h3>
  <p class="small" data-testid="format-note">{pairNote(pair)}</p>
  <div class="copy">
    <span class="copy-name">{partner.displayName}</span>
    <span class="path">{partner.displayPath}</span>
  </div>
  <div class="actions">
    <span class="badge" data-status={partner.availability}>{statusLabel(partner)}</span>
    <OriginalActions item={fromFile(partner, details.via ?? undefined)} />
    {#if pair.state === 'same'}
      <button
        type="button"
        class="button quiet"
        disabled={details.saving}
        onclick={() => void details.setSameBook(false)}
      >
        {NOT_SAME}
      </button>
    {:else if pair.state === 'different'}
      <button
        type="button"
        class="button quiet"
        disabled={details.saving}
        onclick={() => void details.setSameBook(true)}
      >
        {SAME}
      </button>
    {/if}
  </div>
  {#if details.sameBookError}
    <ProblemNotice
      title="Atlas couldn’t save that"
      message={details.sameBookError.message}
      recovery={details.sameBookError.recovery}
    />
  {/if}
</section>

<style>
  section {
    display: grid;
    gap: var(--space-2);
  }

  .copy {
    display: grid;
  }

  .path {
    font-family: ui-monospace, Menlo, Consolas, monospace;
    font-size: 12px;
    overflow-wrap: anywhere;
    color: var(--text-muted);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
  }
</style>
