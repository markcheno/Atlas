<script lang="ts">
  import type { Snippet } from 'svelte';
  import { NAV_VIEWS, view } from '../stores/view.svelte';
  import NavBar from './NavBar.svelte';
  import StatusBar from './StatusBar.svelte';

  let { children }: { children: Snippet } = $props();

  // Cmd+1–5 on macOS, Ctrl+1–5 elsewhere, jump between the main views.
  function onKeydown(event: KeyboardEvent) {
    if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
    const index = Number(event.key) - 1;
    const target = NAV_VIEWS[index];
    if (target) {
      event.preventDefault();
      view.go(target.id);
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="shell">
  <a class="skip-link" href="#main">Skip to content</a>
  <header class="topbar">
    <span class="brand" aria-hidden="true">Atlas</span>
    <NavBar />
  </header>
  <main id="main" class="content">
    {@render children()}
  </main>
  <StatusBar />
</div>

<style>
  .shell {
    display: grid;
    grid-template-rows: auto 1fr auto;
    height: 100%;
  }

  .topbar {
    display: flex;
    align-items: center;
    gap: var(--space-6);
    padding: var(--space-3) var(--space-5);
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  .brand {
    font-weight: 650;
    letter-spacing: 0.02em;
  }

  .content {
    overflow: auto;
    padding: var(--space-7) var(--space-6);
  }

  .skip-link {
    position: absolute;
    left: var(--space-3);
    top: -100px;
    padding: var(--space-2) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    z-index: 10;
  }

  .skip-link:focus {
    top: var(--space-3);
  }
</style>
