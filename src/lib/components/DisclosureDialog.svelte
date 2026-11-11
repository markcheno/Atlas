<script lang="ts">
  // Before the first question to an internet service (phase2-spec §5.7,
  // ADR 0015): what would be sent, with the real sizes. Nothing has been
  // sent; Continue records the agreement and asks again.
  import type { DisclosureNeeded } from '../api/bindings/DisclosureNeeded';
  import { DISCLOSURE_NOTE, DISCLOSURE_TITLE, disclosureText } from '../answerCopy';
  import { EXTERNAL_AI_NOTE } from '../privacyCopy';

  let {
    needed,
    oncontinue,
    oncancel,
  }: { needed: DisclosureNeeded; oncontinue: () => void; oncancel: () => void } = $props();

  let dialog = $state<HTMLElement>();
  let continueButton = $state<HTMLButtonElement>();

  $effect(() => {
    continueButton?.focus();
  });

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      oncancel();
      return;
    }
    if (event.key !== 'Tab' || !dialog) return;
    const focusable = [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled)')];
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

<div class="backdrop" aria-hidden="true"></div>
<div
  class="dialog"
  role="dialog"
  aria-modal="true"
  aria-labelledby="disclosure-title"
  aria-describedby="disclosure-text"
  tabindex="-1"
  bind:this={dialog}
  onkeydown={onKeydown}
>
  <h2 id="disclosure-title">{DISCLOSURE_TITLE}</h2>
  <p id="disclosure-text">{disclosureText(needed)}</p>
  <p class="muted small">{EXTERNAL_AI_NOTE}</p>
  <p class="muted small">{DISCLOSURE_NOTE}</p>
  <div class="actions">
    <button type="button" class="button primary" bind:this={continueButton} onclick={oncontinue}>
      Continue
    </button>
    <button type="button" class="button" onclick={oncancel}>Cancel</button>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgb(0 0 0 / 0.35);
  }

  .dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    z-index: 41;
    display: grid;
    gap: var(--space-3);
    width: min(520px, calc(100vw - 2 * var(--space-5)));
    padding: var(--space-5);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    transform: translate(-50%, -50%);
  }

  h2 {
    margin: 0;
    font-size: 18px;
  }

  p {
    margin: 0;
    line-height: 1.5;
  }

  .actions {
    display: flex;
    gap: var(--space-2);
  }

  .small {
    font-size: 13px;
  }
</style>
