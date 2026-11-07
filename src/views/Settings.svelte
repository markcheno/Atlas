<script lang="ts">
  import NotImplementedNotice from '../lib/components/NotImplementedNotice.svelte';
  import ViewHeading from '../lib/components/ViewHeading.svelte';
  import { THEME_OPTIONS, theme } from '../lib/stores/theme.svelte';

  const PENDING_SECTIONS = [
    { id: 'folders', title: 'Library Folders', slice: '0.3' },
    { id: 'ai', title: 'AI Models', slice: '0.7' },
    { id: 'storage', title: 'Storage', slice: '0.2' },
    { id: 'performance', title: 'Performance & Battery', slice: '0.4' },
  ] as const;
</script>

<div class="view">
  <ViewHeading>Settings</ViewHeading>

  <section aria-labelledby="settings-appearance">
    <h2 id="settings-appearance">Appearance</h2>
    <fieldset>
      <legend class="muted">Theme</legend>
      {#each THEME_OPTIONS as option (option.value)}
        <label class="choice">
          <input
            type="radio"
            name="theme"
            value={option.value}
            checked={theme.preference === option.value}
            onchange={() => theme.set(option.value)}
          />
          {option.label}
        </label>
      {/each}
    </fieldset>
    <p class="muted small">Remembered between launches once the Atlas data location exists.</p>
  </section>

  <section aria-labelledby="settings-privacy">
    <h2 id="settings-privacy">Privacy</h2>
    <p>This build of Atlas makes no network requests and reads no files from your library.</p>
  </section>

  {#each PENDING_SECTIONS as section (section.id)}
    <section aria-labelledby={`settings-${section.id}`}>
      <h2 id={`settings-${section.id}`}>{section.title}</h2>
      <NotImplementedNotice title={section.title} plannedSlice={section.slice} />
    </section>
  {/each}
</div>

<style>
  .view {
    display: grid;
    gap: var(--space-6);
    max-width: 720px;
    margin: 0 auto;
  }

  section {
    display: grid;
    gap: var(--space-3);
  }

  fieldset {
    display: flex;
    gap: var(--space-5);
    margin: 0;
    padding: 0;
    border: 0;
  }

  legend {
    padding: 0;
    margin-bottom: var(--space-2);
    float: left;
    width: 100%;
  }

  .choice {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--control-height);
    cursor: pointer;
  }

  .small {
    font-size: 13px;
  }
</style>
