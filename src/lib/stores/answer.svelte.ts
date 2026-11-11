/**
 * Ask Atlas's one answer (phase2-spec §5.5, ADR 0015): the question, the
 * sources sent, the streamed text with its checked citations, and how it
 * ended. Nothing is kept beyond the last answer, which stays until the
 * next question, a library change or the app closing.
 *
 * The engine starts streaming before `ask_atlas` returns, so `answer-stream`
 * events can arrive before the answer's id is known: they wait in a buffer
 * and are replayed once it is.
 */
import { isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { atlas, AtlasApiError, toApiError } from '../api/client';
import type { AnswerErrorCode } from '../api/bindings/AnswerErrorCode';
import type { AnswerEvent } from '../api/bindings/AnswerEvent';
import type { AnswerStep } from '../api/bindings/AnswerStep';
import type { DisclosureNeeded } from '../api/bindings/DisclosureNeeded';
import type { SemanticStatus } from '../api/bindings/SemanticStatus';
import type { SourceUsed } from '../api/bindings/SourceUsed';
import { search } from './search.svelte';
import { settings } from './settings.svelte';

export const ANSWER_EVENT = 'answer-stream';

export type AnswerPhase =
  | 'idle'
  /** Asking Atlas to find passages. */
  | 'finding'
  /** Passages sent; nothing from the model yet. */
  | 'waiting'
  | 'thinking'
  | 'streaming'
  | 'done'
  | 'stopped'
  | 'failed'
  /** Nothing in the library on it: the model wasn't asked. */
  | 'nothing'
  /** An internet service needs the user's agreement first. */
  | 'disclosure';

/** Why an answer failed: the stream's own code, or a refused request. */
export type AnswerFailure =
  | { kind: 'stream'; code: AnswerErrorCode; partial: boolean }
  | { kind: 'request'; error: AtlasApiError };

class AnswerStore {
  draft = $state('');
  phase = $state<AnswerPhase>('idle');
  /** The question the shown answer is for. */
  question = $state('');
  answerId = $state<string | null>(null);
  sources = $state<SourceUsed[]>([]);
  weak = $state(false);
  semantic = $state<SemanticStatus>({ state: 'off' });
  text = $state('');
  cited = $state<number[]>([]);
  dropped = $state(0);
  failure = $state<AnswerFailure | null>(null);
  disclosure = $state<DisclosureNeeded | null>(null);

  #unlisten: UnlistenFn | null = null;
  #seq = 0;
  /** Steps for answers whose id `ask_atlas` hasn't returned yet. */
  #early = new Map<string, AnswerStep[]>();

  /** Whether an answer is on its way (Stop applies). */
  get live(): boolean {
    return (
      this.phase === 'finding' ||
      this.phase === 'waiting' ||
      this.phase === 'thinking' ||
      this.phase === 'streaming'
    );
  }

  /** Starts listening for answer steps (once per app session). */
  async subscribe() {
    if (this.#unlisten || !isTauri()) return;
    this.#unlisten = await listen<AnswerEvent>(ANSWER_EVENT, (event) => {
      this.#receive(event.payload);
    });
  }

  #receive(e: AnswerEvent) {
    if (e.answerId === this.answerId) {
      this.#apply(e.step);
    } else if (this.phase === 'finding') {
      const early = this.#early.get(e.answerId) ?? [];
      early.push(e.step);
      this.#early.set(e.answerId, early);
    }
  }

  #apply(step: AnswerStep) {
    switch (step.kind) {
      case 'thinking':
        if (this.phase === 'waiting') this.phase = 'thinking';
        break;
      case 'delta':
        this.text += step.text;
        this.phase = 'streaming';
        break;
      case 'citation':
        if (!this.cited.includes(step.n)) this.cited = [...this.cited, step.n];
        break;
      case 'done':
        this.cited = step.cited;
        this.dropped = step.dropped;
        this.phase = 'done';
        break;
      case 'stopped':
        this.phase = 'stopped';
        break;
      case 'error':
        this.cited = step.cited;
        this.dropped = step.dropped;
        this.failure = { kind: 'stream', code: step.code, partial: step.partial };
        this.phase = 'failed';
        break;
    }
  }

  #clearAnswer() {
    this.answerId = null;
    this.sources = [];
    this.weak = false;
    this.semantic = { state: 'off' };
    this.text = '';
    this.cited = [];
    this.dropped = 0;
    this.failure = null;
    this.disclosure = null;
    this.#early.clear();
  }

  /** Asks the question in the box (or `question`), with Search's filters. */
  async ask(question = this.draft) {
    const text = question.trim();
    if (text === '') return;
    if (this.live && this.answerId) void atlas.cancelAnswer(this.answerId).catch(() => {});
    const seq = ++this.#seq;
    this.#clearAnswer();
    this.question = text;
    this.phase = 'finding';
    try {
      const reply = await atlas.askAtlas({
        question: text,
        sourceRootId: search.sourceRootId,
        fileType: search.fileType,
      });
      if (seq !== this.#seq) return;
      switch (reply.kind) {
        case 'started': {
          this.answerId = reply.answerId;
          this.sources = reply.sources;
          this.weak = reply.weak;
          this.semantic = reply.semantic;
          this.phase = 'waiting';
          const early = this.#early.get(reply.answerId) ?? [];
          this.#early.clear();
          for (const step of early) this.#apply(step);
          break;
        }
        case 'nothing_found':
          this.semantic = reply.semantic;
          this.phase = 'nothing';
          break;
        case 'needs_disclosure':
          this.disclosure = {
            profileId: reply.profileId,
            profileLabel: reply.profileLabel,
            baseUrl: reply.baseUrl,
            excerptCount: reply.excerptCount,
            bytes: reply.bytes,
          };
          this.phase = 'disclosure';
          break;
      }
    } catch (err) {
      if (seq !== this.#seq) return;
      this.failure = {
        kind: 'request',
        error: err instanceof AtlasApiError ? err : toApiError('ask_atlas', err),
      };
      this.phase = 'failed';
    }
  }

  /** Asks the shown question again. */
  retry() {
    return this.ask(this.question);
  }

  /** Stop: the engine ends the answer and says `stopped`. */
  async stop() {
    if (!this.live) return;
    if (!this.answerId) {
      // Still finding passages: forget the request; its answer is ignored.
      this.#seq++;
      this.phase = this.text ? 'stopped' : 'idle';
      return;
    }
    try {
      await atlas.cancelAnswer(this.answerId);
    } catch {
      // The answer may already have ended.
    }
  }

  /** The user agreed to send to this internet service: ask again. */
  async continueDisclosure() {
    const d = this.disclosure;
    if (!d) return;
    try {
      settings.current = await atlas.acknowledgeDisclosure(d.profileId);
    } catch (err) {
      this.failure = {
        kind: 'request',
        error: err instanceof AtlasApiError ? err : toApiError('acknowledge_disclosure', err),
      };
      this.phase = 'failed';
      return;
    }
    await this.ask(this.question);
  }

  /** The user didn't agree: nothing was sent. */
  cancelDisclosure() {
    this.disclosure = null;
    this.phase = 'idle';
  }

  /** On a library change: stops any answer and clears the screen. */
  reset() {
    if (this.live && this.answerId) void atlas.cancelAnswer(this.answerId).catch(() => {});
    this.#seq++;
    this.#clearAnswer();
    this.draft = '';
    this.question = '';
    this.phase = 'idle';
  }
}

export const answer = new AnswerStore();
