<script lang="ts">
  // A book's other format (Slice 4.5.1): "Also as PDF", with its own Open
  // and Show in folder. The actions' labels carry the file name, so each
  // format's are told apart.
  import type { FormatFile } from '../api/bindings/FormatFile';
  import type { OpenVia } from '../api/bindings/OpenVia';
  import { alsoAs, fromFormat } from '../formatsCopy';
  import OriginalActions from './OriginalActions.svelte';

  let { other, via }: { other: FormatFile; via?: OpenVia } = $props();
</script>

<span class="other" data-testid="other-format">
  <span class="small">{alsoAs(other)}</span>
  <OriginalActions item={fromFormat(other, via)} />
</span>

<style>
  .other {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
</style>
