<script lang="ts">
  import { onMount, type Snippet } from 'svelte';

  // Moves keyboard and screen-reader focus to the new view's title when the
  // view mounts, so navigation is announced and Tab starts from the top.
  let { children }: { children: Snippet } = $props();
  let heading: HTMLHeadingElement;

  onMount(() => heading.focus());
</script>

<h1 bind:this={heading} tabindex="-1">{@render children()}</h1>
