<script lang="ts">
  // Settings › Privacy (phase2-spec §5.7, ADR 0015): what goes to an AI
  // service and when. Embedding through an internet service is off until
  // allowed; "Disable all external AI" stops every request to one; Ask
  // Atlas's agreements, per service, can be forgotten. Services on this
  // computer or another one need no permission. Research trails (Slice
  // 3.3) have their own part.
  import { onMount } from 'svelte';
  import { atlas, type AtlasApiError } from '../api/client';
  import type { PrivacySettings } from '../api/bindings/PrivacySettings';
  import { formatBytes } from '../format';
  import { count } from '../inventoryCopy';
  import {
    AGREED_TITLE,
    DISABLE_ALL_LABEL,
    DISABLE_ALL_NOTE,
    EXTERNAL_AI_NOTE,
    agreedOn,
  } from '../privacyCopy';
  import { embedding } from '../stores/embedding.svelte';
  import { settings } from '../stores/settings.svelte';
  import ProblemNotice from './ProblemNotice.svelte';
  import TrailPrivacy from './TrailPrivacy.svelte';

  let textBytes = $state<number | null>(null);
  let saving = $state(false);
  let error = $state<AtlasApiError | null>(null);

  const privacy = $derived(settings.current.privacy);
  const files = $derived(embedding.overview?.counts.total ?? 0);

  async function save(change: (p: PrivacySettings) => PrivacySettings) {
    saving = true;
    error = null;
    try {
      await settings.update((s) => ({ ...s, privacy: change(s.privacy) }));
      await embedding.refresh();
    } catch (err) {
      error = err as AtlasApiError;
    } finally {
      saving = false;
    }
  }

  function checked(event: Event): boolean {
    return (event.currentTarget as HTMLInputElement).checked;
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
  <p>{EXTERNAL_AI_NOTE}</p>

  <label class="choice">
    <input
      type="checkbox"
      checked={privacy.disableExternalAi}
      disabled={saving}
      onchange={(e) => {
        const next = checked(e);
        void save((p) => ({ ...p, disableExternalAi: next }));
      }}
      aria-describedby="disable-external-ai-note"
    />
    {DISABLE_ALL_LABEL}
  </label>
  <p id="disable-external-ai-note" class="muted small">{DISABLE_ALL_NOTE}</p>

  <label class="choice">
    <input
      type="checkbox"
      checked={privacy.allowRemoteEmbedding}
      disabled={saving || privacy.disableExternalAi}
      onchange={(e) => {
        const next = checked(e);
        void save((p) => ({ ...p, allowRemoteEmbedding: next }));
      }}
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

  {#if privacy.acknowledged.length > 0}
    <section aria-labelledby="privacy-agreed">
      <h3 id="privacy-agreed">{AGREED_TITLE}</h3>
      <ul>
        {#each privacy.acknowledged as agreed (agreed.profileId)}
          <li data-testid="agreed-service">
            <span>
              <span class="name">{agreed.label}</span>
              <span class="muted small">{agreed.baseUrl} · {agreedOn(agreed.at)}</span>
            </span>
            <button
              type="button"
              class="button quiet"
              disabled={saving}
              aria-label={`Forget agreement for ${agreed.label}`}
              onclick={() =>
                void save((p) => ({
                  ...p,
                  acknowledged: p.acknowledged.filter((a) => a.profileId !== agreed.profileId),
                }))}
            >
              Forget
            </button>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <TrailPrivacy />

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

  h3 {
    margin: var(--space-2) 0 var(--space-1);
    font-size: 14px;
  }

  ul {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  li > span {
    display: grid;
  }

  .name {
    font-weight: 500;
  }

  .small {
    font-size: 12px;
    overflow-wrap: anywhere;
  }
</style>
