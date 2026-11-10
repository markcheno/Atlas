<script lang="ts">
  // Settings › Privacy (phase2-spec §5.7). Slice 2.3 has one choice:
  // embedding the library through an internet service, off by default.
  // Services on this computer or another one need no permission.
  import { onMount } from 'svelte';
  import { atlas, type AtlasApiError } from '../api/client';
  import { formatBytes } from '../format';
  import { count } from '../inventoryCopy';
  import { embedding } from '../stores/embedding.svelte';
  import { settings } from '../stores/settings.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  let textBytes = $state<number | null>(null);
  let saving = $state(false);
  let error = $state<AtlasApiError | null>(null);

  const allowed = $derived(settings.current.privacy.allowRemoteEmbedding);
  const files = $derived(embedding.overview?.counts.total ?? 0);

  async function toggle(event: Event) {
    const next = (event.currentTarget as HTMLInputElement).checked;
    saving = true;
    error = null;
    try {
      await settings.update((s) => ({
        ...s,
        privacy: { ...s.privacy, allowRemoteEmbedding: next },
      }));
      await embedding.refresh();
    } catch (err) {
      error = err as AtlasApiError;
    } finally {
      saving = false;
    }
  }

  onMount(() => {
    if (!settings.loaded) void settings.load();
    atlas.getStorageUsage().then(
      (u) => (textBytes = u.textBytes),
      () => (textBytes = null),
    );
  });
</script>

<div class="privacy">
  <p>
    Atlas reads only the names, sizes, and dates of files in your library folders until it reads
    their text, and never changes them. Testing an AI service sends nothing from your library. To
    index meaning, Atlas sends the text of your books and papers to the embedding model you choose;
    a service on this computer or another one of yours needs no permission.
  </p>
  <label class="choice">
    <input
      type="checkbox"
      checked={allowed}
      disabled={saving}
      onchange={toggle}
      aria-describedby="remote-embedding-volume"
    />
    Allow embedding my library through an internet service
  </label>
  <p id="remote-embedding-volume" class="muted small">
    {#if textBytes !== null && files > 0}
      That would send about {formatBytes(textBytes)} of text from {count(files, 'file')} to the service.
    {:else}
      That would send the text of every file Atlas has read to the service.
    {/if}
    Off until you turn it on.
  </p>
  {#if error}
    <ProblemNotice
      title="Atlas couldn't save this choice"
      message={error.message}
      recovery={error.recovery}
    />
  {/if}
</div>

<style>
  .privacy {
    display: grid;
    gap: var(--space-2);
  }

  .privacy p {
    margin: 0;
  }

  .choice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .small {
    font-size: 12px;
  }
</style>
