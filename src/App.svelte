<script lang="ts">
  import { onMount } from 'svelte';
  import { boot } from './lib/boot';
  import AppShell from './lib/components/AppShell.svelte';
  import { view } from './lib/stores/view.svelte';
  import DataLocation from './views/DataLocation.svelte';
  import DeviceCheck from './views/DeviceCheck.svelte';
  import Home from './views/Home.svelte';
  import DocumentPanel from './lib/components/DocumentPanel.svelte';
  import Library from './views/Library.svelte';
  import Trails from './views/Trails.svelte';
  import PageView from './lib/components/PageView.svelte';
  import LibraryFolders from './views/LibraryFolders.svelte';
  import RootUnavailable from './views/RootUnavailable.svelte';
  import Search from './views/Search.svelte';
  import Settings from './views/Settings.svelte';
  import Welcome from './views/Welcome.svelte';

  onMount(boot);
</script>

{#if view.current === 'loading'}
  <main id="main" class="loading">
    <p class="muted" role="status">Opening Atlas…</p>
  </main>
{:else if view.current === 'welcome'}
  <Welcome />
{:else if view.current === 'data-location'}
  <DataLocation />
{:else if view.current === 'library-folders'}
  <LibraryFolders />
{:else if view.current === 'device-check'}
  <DeviceCheck />
{:else if view.current === 'unavailable'}
  <RootUnavailable />
{:else}
  <AppShell>
    {#if view.current === 'home'}
      <Home />
    {:else if view.current === 'search'}
      <Search />
    {:else if view.current === 'library'}
      <Library />
    {:else if view.current === 'trails'}
      <Trails />
    {:else}
      <Settings />
    {/if}
    <DocumentPanel />
    <PageView />
  </AppShell>
{/if}

<style>
  .loading {
    display: grid;
    place-items: center;
    height: 100%;
  }
</style>
