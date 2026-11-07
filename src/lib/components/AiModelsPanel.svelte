<script lang="ts">
  import { onMount } from 'svelte';
  import type { ConnectionResult } from '../api/bindings/ConnectionResult';
  import type { LocalProviderSuggestion } from '../api/bindings/LocalProviderSuggestion';
  import type { ProviderKind } from '../api/bindings/ProviderKind';
  import type { ProviderProfile } from '../api/bindings/ProviderProfile';
  import type { ProviderProfileInput } from '../api/bindings/ProviderProfileInput';
  import {
    CAPABILITY_NOTE,
    PRESETS,
    PRIVACY_NOTE,
    isSuccess,
    keychainName,
    kindLabel,
    needsAcknowledgement,
    outcomeText,
    storageLabel,
  } from '../providerCopy';
  import { appInfo } from '../stores/app.svelte';
  import { providers } from '../stores/providers.svelte';
  import ProblemNotice from './ProblemNotice.svelte';

  type Form = {
    id: string | null;
    kind: ProviderKind;
    label: string;
    baseUrl: string;
    apiKey: string;
    hadKey: boolean;
    clearApiKey: boolean;
    acknowledged: boolean;
  };

  let form = $state<Form | null>(null);
  let formResult = $state<ConnectionResult | null>(null);
  let confirmingRemove = $state<string | null>(null);

  const os = $derived(appInfo.status?.os);
  const showAck = $derived(form !== null && needsAcknowledgement(form.baseUrl));

  function startNew(presetId = 'ollama') {
    const preset = PRESETS.find((p) => p.id === presetId) ?? PRESETS[0];
    form = {
      id: null,
      kind: preset.kind,
      label: preset.id === 'custom' ? '' : preset.label,
      baseUrl: preset.baseUrl,
      apiKey: '',
      hadKey: false,
      clearApiKey: false,
      acknowledged: false,
    };
    formResult = null;
  }

  function fromSuggestion(s: LocalProviderSuggestion) {
    form = {
      id: null,
      kind: s.kind,
      label: s.label,
      baseUrl: s.baseUrl,
      apiKey: '',
      hadKey: false,
      clearApiKey: false,
      acknowledged: false,
    };
    formResult = null;
  }

  function edit(p: ProviderProfile) {
    form = {
      id: p.id,
      kind: p.kind,
      label: p.label,
      baseUrl: p.baseUrl,
      apiKey: '',
      hadKey: p.hasCredential,
      clearApiKey: false,
      acknowledged: p.insecureHttpAcknowledged,
    };
    formResult = null;
  }

  function input(f: Form): ProviderProfileInput {
    return {
      id: f.id,
      kind: f.kind,
      label: f.label,
      baseUrl: f.baseUrl,
      apiKey: f.apiKey.trim() === '' ? null : f.apiKey,
      clearApiKey: f.clearApiKey,
      insecureHttpAcknowledged: f.acknowledged,
    };
  }

  async function testForm() {
    if (!form) return;
    formResult = await providers.testCandidate(input(form));
  }

  async function saveForm() {
    if (!form) return;
    const saved = await providers.save(input(form));
    if (saved) {
      if (formResult) providers.results[saved.id] = formResult;
      form = null;
      formResult = null;
    }
  }

  function applyPreset(id: string) {
    const preset = PRESETS.find((p) => p.id === id);
    if (!preset || !form) return;
    form.kind = preset.kind;
    form.baseUrl = preset.baseUrl;
    if (preset.id !== 'custom') form.label = preset.label;
    formResult = null;
  }

  onMount(() => {
    void providers.refresh();
  });
</script>

{#snippet resultView(r: ConnectionResult, label: string, kind: ProviderKind, hadKey: boolean)}
  <div class="result" data-outcome={r.outcome} role="status">
    <p class:ok={isSuccess(r)}>{outcomeText(r, label, kind, hadKey)}</p>
    {#if r.outcome === 'reachable'}
      <ul class="models" aria-label="Models">
        {#each r.modelIds.slice(0, 12) as id (id)}
          <li>{id}</li>
        {/each}
        {#if r.modelIds.length > 12}<li class="muted">and {r.modelIds.length - 12} more</li>{/if}
      </ul>
      <p class="muted small">{CAPABILITY_NOTE}</p>
    {/if}
    <p class="muted small">{r.message}</p>
  </div>
{/snippet}

<div class="panel">
  <p class="muted small">{PRIVACY_NOTE}</p>

  {#if providers.profiles.length > 0}
    <ul class="cards" aria-label="AI services">
      {#each providers.profiles as p (p.id)}
        <li class="card">
          <div class="head">
            <div>
              <span class="name">{p.label}</span>
              <span class="muted small"
                >{kindLabel(p.kind)} · {p.onThisComputer
                  ? 'This computer'
                  : 'Another computer'}</span
              >
              <span class="url">{p.baseUrl}</span>
              <span class="muted small">{storageLabel(p.credentialStorage, os)}</span>
            </div>
            <div class="actions">
              <button
                type="button"
                class="button"
                disabled={providers.testing === p.id}
                aria-label={`Test ${p.label}`}
                onclick={() => providers.testSaved(p.id)}
              >
                {providers.testing === p.id ? 'Testing…' : 'Test'}
              </button>
              <button
                type="button"
                class="button quiet"
                aria-label={`Edit ${p.label}`}
                onclick={() => edit(p)}
              >
                Edit
              </button>
              {#if confirmingRemove === p.id}
                <span class="small">Remove this service and its key?</span>
                <button
                  type="button"
                  class="button"
                  onclick={() => {
                    confirmingRemove = null;
                    void providers.remove(p.id);
                  }}>Remove</button
                >
                <button type="button" class="button quiet" onclick={() => (confirmingRemove = null)}
                  >Keep</button
                >
              {:else}
                <button
                  type="button"
                  class="button quiet"
                  aria-label={`Remove ${p.label}`}
                  onclick={() => (confirmingRemove = p.id)}>Remove</button
                >
              {/if}
            </div>
          </div>
          {#if providers.results[p.id]}
            {@render resultView(providers.results[p.id], p.label, p.kind, p.hasCredential)}
          {/if}
        </li>
      {/each}
    </ul>
  {:else}
    <p class="muted">No AI services connected.</p>
  {/if}

  {#if form}
    <form
      class="form"
      aria-label={form.id ? 'Edit AI service' : 'Add an AI service'}
      onsubmit={(e) => {
        e.preventDefault();
        void saveForm();
      }}
    >
      {#if !form.id}
        <label>
          <span class="muted small">Service</span>
          <select onchange={(e) => applyPreset(e.currentTarget.value)}>
            {#each PRESETS as preset (preset.id)}
              <option value={preset.id}>{preset.label}</option>
            {/each}
          </select>
        </label>
      {/if}
      <label>
        <span class="muted small">Type</span>
        <select bind:value={form.kind}>
          <option value="ollama">Ollama</option>
          <option value="openai_compatible">OpenAI-compatible</option>
        </select>
      </label>
      <label>
        <span class="muted small">Name</span>
        <input type="text" bind:value={form.label} placeholder={kindLabel(form.kind)} />
      </label>
      <label>
        <span class="muted small">Address</span>
        <input type="url" bind:value={form.baseUrl} placeholder="http://localhost:11434" required />
      </label>
      <label>
        <span class="muted small">API key (optional)</span>
        <input
          type="password"
          autocomplete="off"
          bind:value={form.apiKey}
          placeholder={form.hadKey
            ? 'Saved; type to replace'
            : 'Not needed for most local services'}
        />
        <span class="muted small">Stored in your {keychainName(os)}, never in Atlas's files.</span>
      </label>
      {#if form.hadKey}
        <label class="check">
          <input type="checkbox" bind:checked={form.clearApiKey} />
          Remove the saved key
        </label>
      {/if}
      {#if showAck}
        <div class="warning" role="alert">
          <p>
            This address uses unencrypted HTTP to another computer. Anyone on the network between
            them could read what is sent, including the key.
          </p>
          <label class="check">
            <input type="checkbox" bind:checked={form.acknowledged} />
            I trust this network; use plain HTTP for this service
          </label>
        </div>
      {/if}

      {#if formResult}
        {@render resultView(
          formResult,
          form.label || kindLabel(form.kind),
          form.kind,
          form.apiKey.trim() !== '' || form.hadKey,
        )}
      {/if}

      <div class="actions">
        <button
          type="button"
          class="button"
          disabled={providers.testing === 'candidate'}
          onclick={testForm}
        >
          {providers.testing === 'candidate' ? 'Testing…' : 'Test'}
        </button>
        <button type="submit" class="button primary">Save</button>
        <button type="button" class="button quiet" onclick={() => (form = null)}>Cancel</button>
      </div>
    </form>
  {:else}
    <div class="actions">
      <button type="button" class="button" onclick={() => startNew()}>Add a service…</button>
      <button
        type="button"
        class="button quiet"
        disabled={providers.detecting}
        onclick={() => providers.detect()}
      >
        {providers.detecting ? 'Looking…' : 'Look for AI services on this computer'}
      </button>
    </div>
    {#if providers.suggestions}
      {#if providers.suggestions.length === 0}
        <p class="muted small">No AI services answered on the usual ports of this computer.</p>
      {:else}
        <ul class="suggestions" aria-label="Services found on this computer">
          {#each providers.suggestions as s (s.baseUrl)}
            <li>
              <button type="button" class="button quiet" onclick={() => fromSuggestion(s)}>
                Add {s.label} ({s.baseUrl}){s.needsKey ? ' · needs a key' : ''}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  {/if}

  {#if providers.error}
    <ProblemNotice
      title="That didn't work"
      message={providers.error.message}
      recovery={providers.error.recovery}
    />
  {/if}
</div>

<style>
  .panel {
    display: grid;
    gap: var(--space-3);
  }

  .cards,
  .suggestions,
  .models {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .card,
  .form {
    display: grid;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .head > div:first-child {
    display: grid;
  }

  .name {
    font-weight: 600;
  }

  .url {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .form label {
    display: grid;
    gap: var(--space-1);
  }

  .form input[type='text'],
  .form input[type='url'],
  .form input[type='password'],
  .form select {
    min-height: 36px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .check {
    display: inline-flex !important;
    align-items: center;
    gap: var(--space-2);
  }

  .warning {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--notice-border);
    border-radius: var(--radius);
    background: var(--notice-bg);
  }

  .result {
    display: grid;
    gap: var(--space-1);
  }

  .result .ok {
    font-weight: 500;
  }

  .models {
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
  }

  .small {
    font-size: 12px;
  }
</style>
