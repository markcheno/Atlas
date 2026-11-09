<script lang="ts">
  // The Text badge: a symbol and a label for whether Atlas has read a file,
  // with the reason as a tooltip.
  import type { FileInstance } from '../api/bindings/FileInstance';
  import { reasonText, rowState, TEXT_ICON, TEXT_LABEL } from '../extractionCopy';
  import { extraction } from '../stores/extraction.svelte';

  let { item }: { item: FileInstance } = $props();
  const text = $derived(rowState(item, extraction.overview));
</script>

<span
  class="badge text"
  data-text={text}
  data-testid="text-status"
  title={reasonText(item) ?? undefined}
>
  <span aria-hidden="true">{TEXT_ICON[text]}</span>
  {TEXT_LABEL[text]}
</span>
