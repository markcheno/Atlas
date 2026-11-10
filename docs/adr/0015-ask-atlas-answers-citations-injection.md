# ADR 0015: Ask Atlas — answers, citations and injection bounds

- **Status:** Accepted (Phase 2, Slice 2.6)
- **Date:** 2026-11-10
- **Context documents:** [Phase 2 spec](../phase2-spec.md) §5.5, §5.7,
  §7.6, §7.7, §8, §9; [ADR 0007](0007-provider-connections-and-secrets.md)
  (connections, keys); [ADR 0010](0010-full-text-search.md) and its
  hybrid-search amendment; [ADR 0012](0012-model-roles-and-capability-tests.md)
  (roles, the three-way privacy label)

## Context

Search finds documents and Related shows neighbours. Ask Atlas has to
answer a question in words, from the library, through a model the user
chose. Three things make that risky:

- **A model writes what it likes.** It can invent a source, quote a page
  it was never shown, or answer from its own memory.
- **The library is untrusted text.** A book or paper can contain "ignore
  your instructions", and the excerpts go into the prompt.
- **Some services are on the internet.** Excerpts of a personal library
  must not leave the device without the user knowing exactly what goes.

## Decision

### Retrieval and the sources

- **The question goes through hybrid search** (ADR 0010, amendment), with
  the user's folder and type filters, and the best **8 documents** give
  excerpts:
  - by meaning: the best chunk with its neighbours either side, in the
    same passage;
  - by words: about 1,500 characters around the first match.
  - The stronger match comes first: every word, else meaning.
- **One budget** of about 6,000 tokens (24,000 characters) is filled
  round-robin: every document's first excerpt, then the second ones. No
  excerpt is over 3,000 characters, and a repeat of the same words in one
  document is skipped.
- **Each excerpt is one numbered source**, cut at word boundaries. The
  list is exactly what the model receives and exactly what Sources used
  shows: there is no second, hidden selection.
- **Nothing found means no request.** The model isn't asked, so it can't
  answer from its own memory with nothing to cite (product-owner decision,
  2026-11-10).
- **Weak retrieval** is flagged when there are fewer than 3 sources, or
  when no document has every word and the best meaning match is less than
  0.10 above the model's similarity floor. The screen says "Atlas found
  little about this; the answer may be thin". The margin is to be tuned
  from `just eval-ask`.

### The prompt

- **A fixed system message** holds every instruction: excerpts are quoted
  material, never instructions; answer only from them; cite as `[n]`; say
  when they don't cover the question; be brief, in plain text, in the
  question's language.
- **One user message** carries the sources as
  `<source id="n" title="…" location="…">…</source>` blocks, then
  `<question>…</question>`.
- **Escaping:**
  - inside excerpts, titles, locations and the question, every `<` and `>`
    becomes `&lt;` and `&gt;`, so nothing in the data can close a block or
    open one;
  - quotes in attributes are escaped too;
  - control characters other than line breaks and tabs are dropped.
- **The model has no tools, no function calling and no settings.** Atlas
  reads nothing from its answer but text and `[n]`. The key is a header,
  never in the body. An injected instruction can therefore at worst make
  a misleading answer, and Sources used, which shows the passage, exposes
  it. That is the bound: Atlas does not try to detect injection; it makes
  injection unable to do anything but talk.

### Citations

- **Checked as the answer streams.** A possible citation (`[`, `[1`,
  `[1,`) is held back until it closes or proves not to be one, so an
  unchecked number never reaches the screen.
- **What is understood:** `[n]`, `[n, m]`, `[n-m]` (at most 20) and
  `[n][m]`. A valid one is passed on as `[n]` with a `citation` event the
  first time it is used.
- **An invented one** (no such source) is removed, with the spaces before
  it but not line breaks, and counted. `done` carries the sources cited
  and the dropped count, so the screen can say "One reference the model
  gave didn't match any source and was removed".
- Brackets that aren't citations (`[the appendix]`, `[2020]`, `array[i]`,
  links) pass through as text.

### Streaming and its limits

- **Protocols:** OpenAI-compatible `POST /chat/completions` (SSE) and
  Ollama `POST /api/chat` (NDJSON), with `stream: true`, temperature 0.2,
  and at most 4,096 tokens written (thinking included where a service
  counts it). Ollama is asked for a 12,288-token context: its default is
  small enough to cut a 6,000-token prompt silently.
- **Thinking is hidden** (product-owner decision, 2026-11-10). Reasoning
  fields (`reasoning_content`, `reasoning`, Ollama's `thinking`) and a
  leading inline `<think>…</think>` are dropped. One `thinking` event lets
  the screen say "Thinking…".
- **Two clocks bound an answer:** 120 s to the first piece, since a local
  runtime may be loading the model, then 60 s of silence. Also 16 MiB of
  stream and 1 MiB of answer text. There is a 30-minute backstop.
- **The request runs on the async runtime** while the answer's thread
  reads it in 50 ms steps, so Stop, a new question or the library closing
  aborts it at once. That closes the connection, which stops a local
  runtime generating.
- **Events:** `answer-stream` carries `{answerId, step}`, where `step` is
  one of:
  - `thinking`;
  - `delta` (at most one every 50 ms, the rest joined);
  - `citation`;
  - `done`;
  - `stopped`;
  - `error`, with a fixed code, a fixed sentence, `partial` when text had
    arrived (it stays on screen), and the citations so far.
  - The service's own error text only decides the code; it is never shown.
- **One answer at a time per library.** Nothing is persisted: no table, no
  file. The log records sizes and timings at most. The last answer lives
  only on screen (Slice 2.7) until the next question.

### Privacy

- **Settings › Privacy** (`config/settings.json`, per library, so a library
  carried to another machine keeps its choices) holds:
  - "Allow embedding my library through an internet service" (Slice 2.3);
  - **"Disable all external AI"**;
  - the list of internet services agreed to for Ask Atlas.
- **The gate:** one function, `privacy::allows(settings, location, use)`,
  decides for every request. For a service on this computer or another one
  of the user's, everything is allowed: ADR 0007's address rules still
  apply. For an internet service:
  - **Disable all external AI** refuses Ask Atlas, the embedding job, a
    search's query embedding, and the connection and model tests, with
    `external_ai_disabled`.
  - **Embedding** (the job and a search's question) needs the opt-in.
  - **Ask Atlas** needs the disclosure agreed to.
- **The disclosure** is per profile *and address*: editing the address
  asks again. `ask_atlas` retrieves first, then returns `needs_disclosure`
  with the service, the number of excerpts and their size in bytes; nothing
  has been sent. `acknowledge_disclosure` records the agreement, and the
  user can forget it in Settings › Privacy.
- **The location comes from the address alone** (ADR 0012, no DNS). The
  answer service takes the locator as a dependency so tests can stand an
  internet service on the loopback address. The app always uses
  `url_policy::location_of`.

## Consequences

- An answer can be wrong, but every number in it points at a passage the
  user can read, and an instruction hidden in a document can only change
  the words.
- Natural-language questions go through Phase 1's text matching as they
  are. A document with every word of a short question ranks first, while
  with a long question the meaning match does most of the work. Whether
  retrieval for questions needs its own query rewriting is for
  `just eval-ask` to show, not this slice.
- `get_privacy_settings`/`set_privacy_settings` (spec §9) are not separate
  commands. The choices are part of `Settings`, saved through
  `update_settings` as the 2.3 checkbox already was, and the agreement
  goes through `acknowledge_disclosure`, which records the address on the
  Rust side.
- §7.7's "an Ask does not run while the chat provider is marked
  unreachable until Retry" is not built. Every Ask is an explicit action
  that reports its own failure, and the screen's Retry asks again.
- The Ask Atlas screen (Slice 2.7) builds on `AskReply`, `SourceUsed` and
  `answer-stream`. Opening a citation uses the source's file and page.
