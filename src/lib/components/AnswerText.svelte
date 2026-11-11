<script lang="ts">
  // An answer's text as Ask Atlas shows it (phase2-spec §5.5): paragraphs,
  // bold and italic from closed markers only, never markup, and a chip for
  // each `[n]` that names a source. Used live (AnswerView) and recorded
  // (a trail's question, Slice 3.4).
  import { paragraphs } from '../answerText';

  let {
    text,
    label,
    oncite,
    busy = false,
    heading = 'Answer',
    headingId,
  }: {
    text: string;
    /** The chip's name for source `n`; no chip without one. */
    label: (n: number) => string | null;
    oncite: (n: number, trigger: HTMLElement) => void;
    busy?: boolean;
    heading?: string;
    headingId: string;
  } = $props();

  const shown = $derived(paragraphs(text));
</script>

<article class="answer" aria-labelledby={headingId} aria-busy={busy}>
  <h2 id={headingId} class="visually-hidden">{heading}</h2>
  {#each shown as paragraph, i (i)}
    <p>
      {#each paragraph as piece, j (j)}{#if piece.kind === 'cite'}{@const name = label(
            piece.n,
          )}{#if name}<button
              type="button"
              class="chip"
              aria-label={name}
              title={name}
              onclick={(e) => oncite(piece.n, e.currentTarget)}>{piece.n}</button
            >{/if}{:else if piece.bold && piece.italic}<strong><em>{piece.text}</em></strong
          >{:else if piece.bold}<strong>{piece.text}</strong>{:else if piece.italic}<em
            >{piece.text}</em
          >{:else}{piece.text}{/if}{/each}
    </p>
  {/each}
</article>

<style>
  p {
    margin: 0;
  }

  .answer {
    display: grid;
    gap: var(--space-3);
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    line-height: 1.6;
  }

  .answer p {
    white-space: pre-line;
    overflow-wrap: anywhere;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.6em;
    height: 1.4em;
    margin: 0 0.1em;
    padding: 0 0.35em;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-muted, var(--surface));
    font-size: 0.8em;
    font-weight: 600;
    line-height: 1;
    vertical-align: 0.15em;
    cursor: pointer;
  }

  .chip:hover {
    border-color: var(--accent, currentColor);
  }
</style>
