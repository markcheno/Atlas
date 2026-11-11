<script lang="ts">
  // Ask Atlas's question field (phase2-spec §5.5): Enter asks, Shift+Enter
  // starts a new line; Stop takes Ask's place while an answer is on its way.
  import { ASK_HELP, ASK_LABEL, MAX_QUESTION_CHARS, questionCounter } from '../answerCopy';
  import { answer } from '../stores/answer.svelte';

  const counter = $derived(questionCounter(answer.draft.length));
  const empty = $derived(answer.draft.trim() === '');

  function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!empty && !answer.live) void answer.ask();
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key !== 'Enter' || event.shiftKey || event.isComposing) return;
    event.preventDefault();
    if (!empty && !answer.live) void answer.ask();
  }
</script>

<form class="box" role="search" aria-label="Ask Atlas" onsubmit={submit}>
  <label for="ask-input">{ASK_LABEL}</label>
  <div class="row">
    <textarea
      id="ask-input"
      rows="2"
      maxlength={MAX_QUESTION_CHARS}
      placeholder="For example: what did Machiavelli say about hired soldiers?"
      bind:value={answer.draft}
      aria-describedby="ask-help{counter ? ' ask-counter' : ''}"
      onkeydown={onKeydown}></textarea>
    {#if answer.live}
      <button type="button" class="button" onclick={() => void answer.stop()}>Stop</button>
    {:else}
      <button type="submit" class="button primary" disabled={empty}>Ask</button>
    {/if}
  </div>
  <p id="ask-help" class="muted small">{ASK_HELP}</p>
  {#if counter}<p id="ask-counter" class="muted small" data-testid="ask-counter">{counter}</p>{/if}
</form>

<style>
  .box {
    display: grid;
    gap: var(--space-2);
  }

  .box label {
    font-weight: 500;
  }

  .row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
  }

  textarea {
    flex: 1;
    min-height: 48px;
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    font: inherit;
    font-size: 16px;
    resize: vertical;
  }

  .row .button {
    min-height: 48px;
  }

  p {
    margin: 0;
  }

  .small {
    font-size: 13px;
  }
</style>
