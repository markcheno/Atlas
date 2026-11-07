<script lang="ts">
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { view } from '../lib/stores/view.svelte';
</script>

<main id="main" class="welcome">
  <div class="card">
    <ViewHeading>Explore everything you've collected.</ViewHeading>
    <p class="lede">Atlas catalogs your existing files without moving or changing them.</p>
    <p class="muted small">No account, sign-in, or AI setup needed.</p>
    <button type="button" class="button primary" onclick={() => view.go('data-location')}>
      Get Started
    </button>
  </div>
</main>

<style>
  .welcome {
    display: grid;
    place-items: center;
    height: 100%;
    padding: var(--space-6);
  }

  .card {
    display: grid;
    justify-items: center;
    gap: var(--space-4);
    max-width: 520px;
    text-align: center;
  }

  .card :global(h1) {
    font-size: 34px;
  }

  .lede {
    font-size: 17px;
  }

  .small {
    font-size: 13px;
    margin-bottom: var(--space-4);
  }
</style>
