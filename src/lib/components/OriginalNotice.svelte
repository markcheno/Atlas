<script lang="ts">
  // What shows under a file after Open or Reveal: the download question for
  // a cloud-only file, or the problem that stopped it.
  import { downloadPrompt } from '../libraryCopy';
  import { originals, type Original } from '../stores/originals.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  let { item }: { item: Original } = $props();
</script>

{#if originals.confirmId === item.id}
  <div class="confirm" role="group" aria-label="Confirm download">
    <span>{downloadPrompt(item)}</span>
    <button type="button" class="button primary" onclick={() => originals.open(item, true)}>
      Download and open
    </button>
    <button type="button" class="button quiet" onclick={() => originals.cancel()}>Cancel</button>
  </div>
{/if}
{#if originals.problem?.id === item.id}
  <ProblemNotice
    title={`Couldn't open ${item.displayName}`}
    message={originals.problem.error.message}
    recovery={originals.problem.error.recovery}
  />
{/if}

<style>
  .confirm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }
</style>
