<script lang="ts">
  // A Bookmark or Save as a finding button (Slice 3.5, phase3-spec §5.3),
  // and what came of pressing it. The renderer names what to keep; Rust
  // builds it from the catalog and from what it showed.
  import type { FindingSource } from '../api/bindings/FindingSource';
  import type { SaveTarget } from '../api/bindings/SaveTarget';
  import { BOOKMARK, SAVE_FINDING } from '../savedCopy';
  import { saved } from '../stores/saved.svelte';

  let {
    target,
    finding,
    key,
    label,
    name,
  }: {
    /** Bookmark this. */
    target?: SaveTarget;
    /** Or save this as a finding. */
    finding?: FindingSource;
    /** Names the button, for its status. */
    key: string;
    label?: string;
    /** What it saves, for the accessible name: "Bookmark: The Prince". */
    name?: string;
  } = $props();

  const text = $derived(label ?? (finding ? SAVE_FINDING : BOOKMARK));
  const state = $derived(saved.states[key]);

  function press() {
    if (finding) void saved.saveFinding(finding, key);
    else if (target) void saved.bookmark(target, key);
  }
</script>

<span class="save">
  <button
    type="button"
    class="row-button"
    aria-label={name ? `${text}: ${name}` : undefined}
    onclick={press}
  >
    {text}
  </button>
  <span role="status" class="muted small" data-testid={`save-status-${key}`}>
    {#if state?.done}{state.done}{:else if state?.error}{state.error.message}{/if}
  </span>
</span>

<style>
  .save {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .small {
    font-size: 13px;
  }
</style>
