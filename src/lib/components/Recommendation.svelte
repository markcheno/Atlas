<script lang="ts">
  // A recommended model for one role (Slice 2.1, phase2-spec §5.1): what to
  // get for this computer and how, per runtime. From the catalog bundled
  // with Atlas; nothing is fetched or downloaded.
  import type { AiRole } from '../api/bindings/AiRole';
  import type { AiRuntime } from '../api/bindings/AiRuntime';
  import type { RoleRecommendations } from '../api/bindings/RoleRecommendations';
  import { kindLabelOf, NEVER_DOWNLOADS, roleTitle, runtimeLabel, sizeText } from '../roleCopy';

  let {
    role,
    recommendations,
    runtime: initial,
    installedOn,
    onChoose,
  }: {
    role: AiRole;
    recommendations: RoleRecommendations;
    runtime: AiRuntime;
    /** The saved service that listed this model id, if any. */
    installedOn: (modelId: string) => { id: string; label: string } | null;
    onChoose: (profileId: string, modelId: string) => void;
  } = $props();

  let picked = $state<AiRuntime | null>(null);
  let copied = $state(false);
  const runtime = $derived(picked ?? initial);
  const top = $derived(recommendations.models[0]);
  const others = $derived(recommendations.models.slice(1));
  const steps = $derived(top?.runtimes.find((r) => r.runtime === runtime));
  const installed = $derived(
    top?.installedAs ? { modelId: top.installedAs, on: installedOn(top.installedAs) } : null,
  );

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
    } catch {
      copied = false;
    }
  }
</script>

{#if top}
  <div class="rec" role="group" aria-label={`Recommended ${roleTitle(role).toLowerCase()} model`}>
    {#if installed?.on}
      <p>
        <strong>{top.label}</strong> is installed on {installed.on.label}.
        <button
          type="button"
          class="button"
          onclick={() => onChoose(installed.on!.id, installed.modelId)}>Choose it</button
        >
      </p>
    {:else}
      <p>
        <strong>{top.label}</strong>
        <span class="muted small">· {kindLabelOf(top.kind)} · {sizeText(top.sizeBytes)}</span>
      </p>
      <p class="muted small">{top.why}</p>
      <fieldset class="tabs">
        <legend class="muted small">Your AI app</legend>
        {#each top.runtimes as r (r.runtime)}
          <label class:active={r.runtime === runtime}>
            <input
              type="radio"
              name={`runtime-${role}`}
              value={r.runtime}
              checked={r.runtime === runtime}
              onchange={() => {
                picked = r.runtime;
                copied = false;
              }}
            />
            {runtimeLabel(r.runtime)}
          </label>
        {/each}
      </fieldset>
      {#if steps}
        {#if steps.command}
          <div class="command">
            <code>{steps.command}</code>
            <button
              type="button"
              class="button quiet"
              aria-label={`Copy the ${runtimeLabel(steps.runtime)} command`}
              onclick={() => copy(steps.command ?? '')}>{copied ? 'Copied' : 'Copy'}</button
            >
          </div>
        {/if}
        <ol class="steps" aria-label={`Steps for ${runtimeLabel(steps.runtime)}`}>
          {#each steps.steps as step, i (i)}
            <li>{step}</li>
          {/each}
        </ol>
        {#if !steps.verified}
          <p class="muted small">
            Not yet checked with {runtimeLabel(steps.runtime)}: press Test after installing it.
          </p>
        {/if}
      {/if}
      {#each others as alt (alt.id)}
        <p class="muted small">
          {kindLabelOf(alt.kind)}: {alt.label}, {sizeText(alt.sizeBytes)}. {alt.why}
        </p>
      {/each}
      <p class="muted small">{NEVER_DOWNLOADS}</p>
    {/if}
  </div>
{/if}

<style>
  .rec {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px dashed var(--border);
    border-radius: var(--radius);
  }

  .rec p {
    margin: 0;
  }

  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    border: 0;
  }

  .tabs legend {
    width: 100%;
    margin-bottom: var(--space-1);
  }

  .tabs label {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 2px var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }

  .tabs label.active {
    border-color: var(--accent);
  }

  .command {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  code {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .steps {
    margin: 0;
    padding-left: var(--space-4);
  }

  .small {
    font-size: 12px;
  }
</style>
