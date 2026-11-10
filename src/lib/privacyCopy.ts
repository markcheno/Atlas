/** Wording for Settings › Privacy (phase2-spec §5.7, ADR 0015). */

/** What Ask Atlas and search send, and when. */
export const EXTERNAL_AI_NOTE =
  'Ask Atlas sends your question and up to 16 short excerpts from your library (about 24,000 characters) to your chat model, only when you choose Ask. Before the first question to an internet service, Atlas shows what it will send and asks you. Search sends nothing to an internet service unless you allow embedding below. Services on this computer or another one of yours need no agreement.';

export const DISABLE_ALL_LABEL = 'Disable all external AI';

export const DISABLE_ALL_NOTE =
  'Atlas sends nothing at all to an internet service: no questions, no embedding, no tests. Services on this computer or another one of yours keep working.';

export const AGREED_TITLE = 'Internet services you agreed to for Ask Atlas';

/** "Agreed on 10 Nov 2026" from an RFC 3339 time. */
export function agreedOn(at: string): string {
  const d = new Date(at);
  return Number.isNaN(d.getTime())
    ? 'Agreed'
    : `Agreed on ${d.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })}`;
}
