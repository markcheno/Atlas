/** Words for Ask Atlas (phase2-spec §5.5, §5.7; ADR 0015). */
import type { AnswerErrorCode } from './api/bindings/AnswerErrorCode';
import type { DisclosureNeeded } from './api/bindings/DisclosureNeeded';
import type { SemanticStatus } from './api/bindings/SemanticStatus';
import type { SourceUsed } from './api/bindings/SourceUsed';
import { formatBytes } from './format';
import { meaningNote } from './searchCopy';

export const MAX_QUESTION_CHARS = 2000;

export const ASK_LABEL = 'Ask your library';
export const ASK_HELP =
  'Atlas finds passages about your question in your library and asks your chat model to answer from them, with numbered references to each passage.';

export const FINDING = 'Finding passages in your library…';
export const WAITING = 'Waiting for the answer…';
export const THINKING = 'Thinking…';
export const ANSWERING = 'Atlas is answering.';
export const STOPPED = 'Stopped. The answer so far is kept.';
export const NOTHING_FOUND =
  'Atlas found nothing about this in your library, so it didn’t ask the model.';
export const WEAK_NOTE = 'Atlas found little about this in your library; the answer may be thin.';
export const CITED_PASSAGE = 'Cited passage';
export const FROM_LAST_QUESTION = 'Sent to the model for your last question.';

/** The note when the model gave references that matched no source. */
export function droppedNote(dropped: number): string | null {
  if (dropped <= 0) return null;
  return dropped === 1
    ? "One reference the model gave didn't match any source and was removed."
    : `${dropped} references the model gave didn't match any source and were removed.`;
}

/** Search's meaning note, said of the sources. */
export function askMeaningNote(s: SemanticStatus): string | null {
  const note = meaningNote(s);
  if (!note) return null;
  return note.replace('Showing text matches only: ', 'Passages were found by their words only: ');
}

/** A chip's accessible name: "Source 2: The Prince, Chapter 17". */
export function sourceName(s: SourceUsed): string {
  return s.location ? `Source ${s.n}: ${s.title}, ${s.location}` : `Source ${s.n}: ${s.title}`;
}

/** Why an answer stopped early, in plain words. */
export function answerError(code: AnswerErrorCode): string {
  switch (code) {
    case 'network_error':
      return 'Atlas lost the connection to the chat service. Is it still running?';
    case 'timeout':
      return 'The chat service stopped answering for too long.';
    case 'authentication_failed':
      return 'The chat service refused the key. Check it in Settings › AI Models.';
    case 'model_not_found':
      return "The chat service doesn't have the model chosen for chat. Choose another in Settings › AI Models.";
    case 'service_error':
      return 'The chat service reported a problem, such as being busy or out of memory. Try again in a moment.';
    case 'protocol_mismatch':
      return "The chat service answered, but not the way Atlas expected. Check that it's OpenAI-compatible or Ollama.";
    case 'too_long':
      return 'The answer grew longer than Atlas keeps.';
    case 'no_answer':
      return 'The model finished without writing an answer. A model that thinks first can use up its whole allowance; try again, or choose a model that answers directly.';
  }
}

export function failureTitle(partial: boolean): string {
  return partial ? 'The answer stopped early' : "Atlas couldn't get an answer";
}

/** "Sources used (14)". */
export function sourcesHeading(n: number): string {
  return `Sources used (${n})`;
}

/** What the disclosure dialog says will be sent. */
export function disclosureText(d: DisclosureNeeded): string {
  const excerpts = d.excerptCount === 1 ? '1 excerpt' : `${d.excerptCount} excerpts`;
  return `Atlas will send your question and ${excerpts} from your library, about ${formatBytes(d.bytes)}, to ${d.profileLabel} at ${d.baseUrl}. It's an internet service: what you send leaves this computer.`;
}

export const DISCLOSURE_TITLE = 'Send excerpts to an internet service?';
export const DISCLOSURE_NOTE =
  'Atlas asks once for each service and address. You can forget the agreement in Settings › Privacy.';

/** "1,950 of 2,000 characters", shown near the limit. */
export function questionCounter(length: number): string | null {
  if (length < MAX_QUESTION_CHARS - 200) return null;
  return `${length.toLocaleString('en-US')} of ${MAX_QUESTION_CHARS.toLocaleString('en-US')} characters`;
}
