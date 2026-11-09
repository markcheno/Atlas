# ADR 0012: Model roles, capability tests and recommendations

- **Status:** Accepted (Phase 2, Slice 2.1)
- **Date:** 2026-11-09
- **Context documents:** [Phase 2 spec](../phase2-spec.md) §5.1, §7.1, §8;
  [ADR 0007](0007-provider-connections-and-secrets.md) (connections and
  secrets); [product spec](../product-spec.md) §4

## Context

Phase 2 calls models: one to turn text into embeddings, one to answer
questions. Until now a connection test only listed a service's models
(ADR 0007), and a listing says nothing about what a model can do: oMLX on
the reference machine lists 22 models, chat and embedding alike, under
one OpenAI-compatible address. Atlas needs to know which model does which
job, to be sure it can, and to tell a user with nothing suitable installed
what to get, without downloading anything itself or sending anything from
the library.

## Decision

**Two roles, chat and embedding**, each a saved profile plus a model id, in
the catalog (`model_roles`, migration 0011). Deleting a profile clears its
roles (foreign key cascade). The embedding role records the model's
dimensions. Reranking and vision stay extension points (product spec §4).

**A capability test is one fixed request to one model.** Embeddings:
`{"model": m, "input": ["Atlas capability test"]}` to `{base}/embeddings`
(OpenAI-compatible) or `{base}/api/embed` (Ollama); it passes when the
answer holds a non-empty vector of finite numbers, whose length is the
model's dimensions. Chat: one user message, "Reply with the word OK.", with
`stream: true` and a 16-token limit, to `{base}/chat/completions` (SSE) or
`{base}/api/chat` (NDJSON); it passes on the first streamed token,
including a reasoning model's thought (`reasoning_content`, `thinking`),
since such a model may spend its 16 tokens thinking. It also exercises the
streaming path Ask Atlas will use. The test text is fixed in the code and
asserted by the tests; nothing from the library, no file names, and never
the model's reply cross IPC.

**Outcomes are honest:** works, needs a model id (nothing is sent), model
not found, wrong role (a chat model asked for embeddings or the reverse),
authentication failed, protocol mismatch, network error, timeout, and the
address rejected by ADR 0007's rules. They are classified from the HTTP
status and the service's own error message (OpenAI's `error.message`,
Ollama's `error`, FastAPI's `detail`), which is used for classification
only and not shown. A 404 with no model mentioned means the endpoint is
missing. ADR 0007's transport rules stand: no redirects, no proxies,
bounded bodies (1 MiB for an embedding answer, 64 KiB of stream), the key
only to the profile's own address. The total budget is **two minutes**,
because oMLX, Ollama and LM Studio load a model on its first request,
which can take most of a minute for a large chat model; the UI says so
while it waits.

**A role is assigned only with a passing test** of that model for that
role, at most 24 hours old (`set_model_role` checks the result it is given;
`ModelNotTested` otherwise). The UI offers Use only after the test passes
and hides it again if the model id changes.

**Recommendations are bundled and offline.** `recommendations.json`,
compiled into the app, lists per role a recommended model and faster or
higher-quality alternatives with a download size, a minimum memory, the
dimensions, one line of why, and per runtime (oMLX, Ollama, LM Studio) the
model id, a command where one exists, the steps, and whether that runtime
is known to serve the model. `get_model_recommendations` crosses it with
the device check's total memory (unknown memory gives the smallest entry)
and with the model ids the user's services already listed in this session,
matched with case and separators ignored, so an installed copy is offered
directly. It makes no network call; a "check for newer recommendations"
fetch remains a later, explicit action (product spec §4). Atlas never
downloads a model. Initial entries: Qwen3-Embedding 0.6B (recommended) and
4B for embeddings; Qwen3.5 9B (recommended) and 27B for chat. oMLX's
README lists BERT, BGE-M3, ModernBERT and EmbeddingGemma as embedding
models, not Qwen3. A capability test on the reference machine confirmed
that oMLX serves `mlx-community/Qwen3-Embedding-0.6B-8bit` (listed as
`Qwen3-Embedding-0.6B-8bit`) with 1,024 dimensions, so that entry is
verified. The 4B entry and oMLX's chat entries stay unverified until
someone tests them.

**The privacy label has three values**, from the address alone and never
DNS: This computer (loopback), Another computer (private, link-local and
CGNAT addresses; `.local`, `.lan`, `.home.arpa`, `.internal` and
single-label names), and Internet service (everything else). It is a label
for the user, not a security boundary; ADR 0007's plain-HTTP rule is
unchanged. An Internet-service profile says that Atlas will ask before
sending anything from the library to it; the disclosure itself arrives
with Ask Atlas (Slice 2.6).

## Consequences

- ADR 0007's "a test only lists models" is amended: a capability test
  sends one fixed body, and only when the user presses Test.
- Every later Phase 2 slice reads the roles: the embedding job and the
  index use the embedding role and its dimensions, Ask Atlas the chat role.
- A role is removed only after the user confirms Clear: one press of
  Clear sat too close to Chat's Change during the manual check.
- A model that passes today can be unloaded or removed tomorrow; the jobs
  that use a role report the provider's outcome when that happens rather
  than trusting the stored test.
- The catalog is data: adding a model or correcting a runtime's steps is a
  JSON change with a test that every entry has steps for every runtime.
- Verified by P2-01 (both roles, both kinds, every failure, the fixed
  bodies), P2-02 (roles need a passing test, persist, follow their
  profile), P2-03 (recommendations by memory and installed matching) and
  P2-20 (the key only to its service, never through a redirect, never in
  results, roles or the catalog), in `src-tauri/tests/roles.rs` and the
  unit tests of `http_probe`, `recommend`, `roles` and `url_policy`.
