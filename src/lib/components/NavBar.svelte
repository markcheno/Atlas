<script lang="ts">
  import { NAV_VIEWS, view } from '../stores/view.svelte';

  const modifier =
    typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform) ? '⌘' : 'Ctrl+';
</script>

<nav aria-label="Main">
  <ul>
    {#each NAV_VIEWS as item, index (item.id)}
      <li>
        <button
          type="button"
          class:active={view.current === item.id}
          aria-current={view.current === item.id ? 'page' : undefined}
          title={`${item.label} (${modifier}${index + 1})`}
          onclick={() => view.go(item.id)}
        >
          {item.label}
        </button>
      </li>
    {/each}
  </ul>
</nav>

<style>
  ul {
    display: flex;
    gap: var(--space-1);
    list-style: none;
    margin: 0;
    padding: 0;
  }

  button {
    min-height: 36px;
    padding: 0 var(--space-4);
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    font-weight: 500;
  }

  button:hover {
    color: var(--text);
    background: var(--surface-muted);
  }

  button.active {
    color: var(--text);
    background: var(--accent-soft);
  }
</style>
