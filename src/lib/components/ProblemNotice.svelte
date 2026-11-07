<script lang="ts">
  import type { Snippet } from 'svelte';

  // A problem the user can act on: what happened, what to do, and optional
  // action buttons. Announced politely so screen readers hear it.
  let {
    title,
    message = undefined,
    recovery = undefined,
    actions = undefined,
  }: { title: string; message?: string; recovery?: string; actions?: Snippet } = $props();
</script>

<div class="problem" role="alert">
  <p class="title">{title}</p>
  {#if message}<p>{message}</p>{/if}
  {#if recovery}<p class="muted">{recovery}</p>{/if}
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>

<style>
  .problem {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-4);
    border: 1px solid var(--problem-border);
    border-radius: var(--radius);
    background: var(--problem-bg);
  }

  .title {
    font-weight: 600;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }
</style>
