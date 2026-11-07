<script lang="ts">
  import AppShell from './lib/components/AppShell.svelte';
  import { view } from './lib/stores/view.svelte';
  import Home from './views/Home.svelte';
  import Library from './views/Library.svelte';
  import Settings from './views/Settings.svelte';
  import Welcome from './views/Welcome.svelte';
</script>

{#if view.current === 'welcome'}
  <Welcome />
{:else}
  <AppShell>
    {#if view.current === 'home'}
      <Home />
    {:else if view.current === 'library'}
      <Library />
    {:else}
      <Settings />
    {/if}
  </AppShell>
{/if}
