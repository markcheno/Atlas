<script lang="ts">
  // Sources used (phase2-spec §5.5): every passage sent to the model,
  // numbered as it cites them, cited or not, each as the exact text sent.
  import type { SourceUsed } from '../api/bindings/SourceUsed';
  import { sourceName, sourcesHeading } from '../answerCopy';
  import { details } from '../stores/details.svelte';

  let { sources, cited }: { sources: SourceUsed[]; cited: number[] } = $props();
</script>

<section class="sources" aria-labelledby="sources-used">
  <h2 id="sources-used">{sourcesHeading(sources.length)}</h2>
  <ol>
    {#each sources as s (s.n)}
      <li data-testid="source-used" value={s.n}>
        <div class="head">
          <span class="n" aria-hidden="true">{s.n}</span>
          <span class="title">{s.title}</span>
          {#if s.location}<span class="muted small">{s.location}</span>{/if}
          {#if cited.includes(s.n)}<span class="badge">Cited</span>{/if}
          <button
            type="button"
            class="row-button"
            aria-label={`Details for ${sourceName(s)}`}
            onclick={(e) => details.open(s.fileInstanceId, e.currentTarget, { cited: s })}
          >
            Details
          </button>
        </div>
        <details>
          <summary>The passage sent</summary>
          <p class="excerpt">{s.excerpt}</p>
        </details>
      </li>
    {/each}
  </ol>
</section>

<style>
  .sources {
    display: grid;
    gap: var(--space-3);
  }

  h2 {
    margin: 0;
    font-size: 16px;
  }

  ol {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-3);
  }

  .n {
    min-width: 1.6em;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .title {
    font-weight: 500;
    overflow-wrap: anywhere;
  }

  .head button {
    margin-left: auto;
  }

  summary {
    cursor: pointer;
    font-size: 13px;
  }

  .excerpt {
    margin: var(--space-2) 0 0;
    white-space: pre-line;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }

  .small {
    font-size: 13px;
  }
</style>
