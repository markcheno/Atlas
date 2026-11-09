<script lang="ts">
  // Settings › AI Models › Models (Slice 2.1, phase2-spec §5.1): which
  // saved service and model does embeddings and chat. A model is used only
  // after a capability test passes; when nothing suitable is installed, a
  // recommendation says what to get.
  import { onMount } from 'svelte';
  import type { AiRole } from '../api/bindings/AiRole';
  import type { AiRuntime } from '../api/bindings/AiRuntime';
  import { locationLabel } from '../providerCopy';
  import {
    capabilityText,
    clearedText,
    clearQuestion,
    ROLES,
    roleTitle,
    roleWhat,
    TESTING_NOTE,
    testedOn,
  } from '../roleCopy';
  import { appInfo } from '../stores/app.svelte';
  import { providers } from '../stores/providers.svelte';
  import { roles } from '../stores/roles.svelte';
  import Recommendation from './Recommendation.svelte';

  let editing = $state<AiRole | null>(null);
  let confirmingClear = $state<AiRole | null>(null);
  /** The role cleared last, for one line of confirmation. */
  let cleared = $state<AiRole | null>(null);
  let profileId = $state('');
  let modelId = $state('');

  const listed = $derived(Object.values(providers.results).flatMap((r) => r.modelIds));
  const listedFor = (id: string) => providers.results[id]?.modelIds ?? [];

  const runtime = $derived.by((): AiRuntime => {
    for (const p of providers.profiles) {
      if (p.kind === 'ollama' || p.baseUrl.includes(':11434')) return 'ollama';
      if (p.baseUrl.includes(':1234')) return 'lmstudio';
      if (p.baseUrl.includes(':8000')) return 'omlx';
    }
    return appInfo.status?.os === 'macos' ? 'omlx' : 'ollama';
  });

  function installedOn(id: string) {
    const p = providers.profiles.find((p) => listedFor(p.id).includes(id));
    return p ? { id: p.id, label: p.label } : null;
  }

  function start(role: AiRole, model = '', profile = '') {
    const current = roles.roles[role];
    confirmingClear = null;
    cleared = null;
    editing = role;
    profileId = profile || current?.profileId || providers.profiles[0]?.id || '';
    modelId = model || current?.modelId || '';
    roles.forget(role);
  }

  function passing(role: AiRole) {
    const r = roles.results[role];
    return r && r.outcome === 'works' && r.modelId === modelId.trim() ? r : null;
  }

  async function clearRole(role: AiRole) {
    confirmingClear = null;
    await roles.clearRole(role);
    if (!roles.error) cleared = role;
  }

  async function use(role: AiRole) {
    const r = passing(role);
    if (r && (await roles.use(role, profileId, r))) editing = null;
  }

  $effect(() => {
    void roles.recommend(listed);
  });

  onMount(() => {
    void roles.refresh();
  });
</script>

<section class="roles" aria-labelledby="models-heading">
  <h3 id="models-heading">Models</h3>
  <p class="muted small">
    Choose which model does each job. Atlas checks it first with one short test request.
  </p>

  {#each ROLES as role (role)}
    {@const assigned = roles.roles[role]}
    {@const title = roleTitle(role)}
    <div class="role" role="group" aria-label={title}>
      <div class="head">
        <span class="name">{title}</span>
        <span class="muted small">{roleWhat(role)}</span>
      </div>

      {#if editing === role}
        <form
          class="form"
          aria-label={`Choose the ${title.toLowerCase()} model`}
          onsubmit={(e) => {
            e.preventDefault();
            void roles.test(role, profileId, modelId);
          }}
        >
          <label>
            <span class="muted small">Service</span>
            <select bind:value={profileId} onchange={() => roles.forget(role)}>
              {#each providers.profiles as p (p.id)}
                <option value={p.id}>{p.label} ({locationLabel(p.location)})</option>
              {/each}
            </select>
          </label>
          <label>
            <span class="muted small">Model</span>
            <input
              type="text"
              list={`models-${role}`}
              bind:value={modelId}
              oninput={() => roles.forget(role)}
              placeholder="Choose or type a model id"
            />
            <datalist id={`models-${role}`}>
              {#each listedFor(profileId) as id (id)}
                <option value={id}></option>
              {/each}
            </datalist>
          </label>
          {#if listedFor(profileId).length === 0}
            <p class="muted small">
              <button
                type="button"
                class="button quiet"
                disabled={providers.testing === profileId}
                onclick={() => providers.testSaved(profileId)}
              >
                {providers.testing === profileId ? 'Listing…' : 'List its models'}
              </button>
            </p>
          {/if}
          {#if roles.testing === role}
            <p class="muted small" role="status">{TESTING_NOTE}</p>
          {:else if roles.results[role]}
            <p
              role="status"
              class:ok={roles.results[role]?.outcome === 'works'}
              data-outcome={roles.results[role]?.outcome}
            >
              {capabilityText(roles.results[role]!)}
            </p>
          {/if}
          <div class="actions">
            <button type="submit" class="button" disabled={roles.testing !== null || !profileId}>
              Test
            </button>
            {#if passing(role)}
              <button type="button" class="button primary" onclick={() => use(role)}>Use</button>
            {/if}
            <button
              type="button"
              class="button quiet"
              onclick={() => {
                editing = null;
                roles.forget(role);
              }}>Cancel</button
            >
          </div>
        </form>
      {:else if assigned}
        <p data-testid={`role-${role}`}>
          {assigned.profileLabel} ·
          <span class="model">{assigned.modelId}</span>{assigned.dimensions
            ? ` · ${assigned.dimensions.toLocaleString('en-US')} dimensions`
            : ''}
          <span class="muted small">· tested {testedOn(assigned.testedAt)}</span>
        </p>
        <div class="actions">
          <button
            type="button"
            class="button quiet"
            aria-label={`Change the ${title.toLowerCase()} model`}
            disabled={providers.profiles.length === 0}
            onclick={() => start(role)}>Change</button
          >
          {#if confirmingClear === role}
            <span class="small">{clearQuestion(role)}</span>
            <button type="button" class="button" onclick={() => clearRole(role)}>Clear</button>
            <button type="button" class="button quiet" onclick={() => (confirmingClear = null)}
              >Keep</button
            >
          {:else}
            <button
              type="button"
              class="button quiet"
              aria-label={`Clear the ${title.toLowerCase()} model`}
              onclick={() => {
                confirmingClear = role;
                cleared = null;
              }}>Clear</button
            >
          {/if}
        </div>
      {:else}
        {#if cleared === role}
          <p role="status" class="small">{clearedText(role)}</p>
        {/if}
        <p class="muted">Not set.</p>
        {#if providers.profiles.length > 0}
          <div class="actions">
            <button
              type="button"
              class="button"
              aria-label={`Choose a model for ${title.toLowerCase()}`}
              onclick={() => start(role)}>Choose a model</button
            >
          </div>
        {:else}
          <p class="muted small">Add the AI service that runs it above, then choose its model.</p>
        {/if}
      {/if}

      {#if !assigned && editing !== role && roles.recommendations}
        <Recommendation
          {role}
          recommendations={roles.recommendations[role]}
          {runtime}
          {installedOn}
          onChoose={(profile, model) => start(role, model, profile)}
        />
      {/if}
    </div>
  {/each}
</section>

<style>
  .roles {
    display: grid;
    gap: var(--space-3);
  }

  h3 {
    margin: 0;
    font-size: 15px;
  }

  .roles > p {
    margin: 0;
  }

  .role {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .role p {
    margin: 0;
  }

  .head {
    display: grid;
  }

  .name {
    font-weight: 600;
  }

  .model {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .form {
    display: grid;
    gap: var(--space-2);
  }

  .form label {
    display: grid;
    gap: var(--space-1);
  }

  .form input,
  .form select {
    min-height: 36px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .ok {
    font-weight: 500;
  }

  .small {
    font-size: 12px;
  }
</style>
