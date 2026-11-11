<script lang="ts">
  // The Ask Atlas answer (phase2-spec §5.5, ADR 0015): the streamed text
  // with a chip for each checked citation, what Atlas noticed about it, why
  // it stopped if it did, and every source sent. The answer itself is not a
  // live region: screen readers hear "Atlas is answering", then the whole
  // answer once it ends, never each piece.
  import { tick } from 'svelte';
  import type { SourceUsed } from '../api/bindings/SourceUsed';
  import {
    ANSWERING,
    answerError,
    askMeaningNote,
    droppedNote,
    failureTitle,
    FINDING,
    NOTHING_FOUND,
    sourceName,
    STOPPED,
    THINKING,
    WAITING,
    WEAK_NOTE,
  } from '../answerCopy';
  import { paragraphs } from '../answerText';
  import { answer } from '../stores/answer.svelte';
  import { fromCitation } from '../openVia';
  import { details } from '../stores/details.svelte';
  import { view } from '../stores/view.svelte';
  import DisclosureDialog from './DisclosureDialog.svelte';
  import ProblemNotice from './ProblemNotice.svelte';
  import SourcesUsed from './SourcesUsed.svelte';

  const shown = $derived(paragraphs(answer.text));
  const bySource = $derived(new Map(answer.sources.map((s) => [s.n, s])));
  const status = $derived(
    answer.phase === 'finding'
      ? FINDING
      : answer.phase === 'waiting'
        ? WAITING
        : answer.phase === 'thinking'
          ? THINKING
          : answer.phase === 'stopped'
            ? STOPPED
            : '',
  );
  const failure = $derived(answer.failure);
  const meaning = $derived(answer.phase === 'idle' ? null : askMeaningNote(answer.semantic));
  const ended = $derived(
    answer.phase === 'done' || answer.phase === 'stopped' || answer.phase === 'failed',
  );
  /** What screen readers hear: that it started, then the answer once. */
  const announcement = $derived(
    answer.live && answer.phase !== 'finding'
      ? ANSWERING
      : ended && answer.text
        ? `Answer: ${answer.text.replace(/\[(\d+)\]/g, ' (source $1)')}`
        : '',
  );

  function openSource(s: SourceUsed, trigger: HTMLElement) {
    void details.open(s.fileInstanceId, trigger, {
      cited: s,
      via: fromCitation(answer.answerId, s.n),
    });
  }

  async function goToSettings(section: string) {
    view.go('settings');
    await tick();
    document.getElementById(section)?.scrollIntoView?.({ block: 'start' });
  }

  async function cancelDisclosure() {
    answer.cancelDisclosure();
    await tick();
    document.getElementById('ask-input')?.focus();
  }
</script>

<div class="visually-hidden" aria-live="polite" data-testid="answer-announcer">{announcement}</div>

{#if status}
  <p class="muted" role="status" data-testid="answer-status">{status}</p>
{/if}

{#if answer.phase === 'nothing'}
  <div class="empty" data-testid="answer-nothing">
    <p>{NOTHING_FOUND}</p>
    {#if meaning}<p class="muted">{meaning}</p>{/if}
  </div>
{/if}

{#if failure?.kind === 'request'}
  {@const code = failure.error.code}
  <ProblemNotice
    title={code === 'chat_model_needed'
      ? 'Ask Atlas needs a chat model'
      : code === 'external_ai_disabled'
        ? 'External AI is turned off'
        : "Atlas couldn't ask your question"}
    message={failure.error.message}
    recovery={failure.error.recovery}
  >
    {#snippet actions()}
      {#if code === 'chat_model_needed'}
        <button type="button" class="button" onclick={() => goToSettings('settings-ai')}>
          Set up AI models
        </button>
      {:else if code === 'external_ai_disabled'}
        <button type="button" class="button" onclick={() => goToSettings('settings-privacy')}>
          Open Settings › Privacy
        </button>
      {:else}
        <button type="button" class="button" onclick={() => void answer.retry()}>Try again</button>
      {/if}
    {/snippet}
  </ProblemNotice>
{/if}

{#if answer.text}
  <article class="answer" aria-labelledby="answer-heading" aria-busy={answer.live}>
    <h2 id="answer-heading" class="visually-hidden">Answer</h2>
    {#each shown as paragraph, i (i)}
      <p>
        {#each paragraph as piece, j (j)}{#if piece.kind === 'cite'}{@const s = bySource.get(
              piece.n,
            )}{#if s}<button
                type="button"
                class="chip"
                aria-label={sourceName(s)}
                title={sourceName(s)}
                onclick={(e) => openSource(s, e.currentTarget)}>{piece.n}</button
              >{/if}{:else if piece.bold && piece.italic}<strong><em>{piece.text}</em></strong
            >{:else if piece.bold}<strong>{piece.text}</strong>{:else if piece.italic}<em
              >{piece.text}</em
            >{:else}{piece.text}{/if}{/each}
      </p>
    {/each}
  </article>
{/if}

{#if failure?.kind === 'stream'}
  <ProblemNotice title={failureTitle(failure.partial)} message={answerError(failure.code)}>
    {#snippet actions()}
      <button type="button" class="button" onclick={() => void answer.retry()}>Try again</button>
    {/snippet}
  </ProblemNotice>
{/if}

{#if answer.sources.length > 0}
  {#if answer.weak}<p class="note" data-testid="weak-note">{WEAK_NOTE}</p>{/if}
  {#if ended && droppedNote(answer.dropped)}
    <p class="note" data-testid="dropped-note">{droppedNote(answer.dropped)}</p>
  {/if}
  {#if meaning}<p class="note" data-testid="ask-meaning-note">{meaning}</p>{/if}
  <SourcesUsed sources={answer.sources} cited={answer.cited} />
{/if}

{#if answer.phase === 'disclosure' && answer.disclosure}
  <DisclosureDialog
    needed={answer.disclosure}
    oncontinue={() => void answer.continueDisclosure()}
    oncancel={() => void cancelDisclosure()}
  />
{/if}

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

  .note {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--notice-border);
    border-radius: var(--radius);
    background: var(--notice-bg);
  }

  .empty {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-6);
    border: 1px dashed var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
  }
</style>
