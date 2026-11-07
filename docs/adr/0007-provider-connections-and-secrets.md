# ADR 0007: Provider connections and secrets

- **Status:** Accepted
- **Date:** 2026-11-07
- **Context documents:** [Phase 0 spec](../phase0-spec.md) sections 5.7 and 10;
  [product spec](../product-spec.md) section 4

## Context

Atlas will use AI services the user chooses, either local (Ollama, oMLX,
LM Studio, llama.cpp, vLLM) or hosted. Phase 0 only connects and tests;
it runs no inference. Even a connection test must avoid four things:
leaking the library, leaking the API key, quietly talking to the network,
and overstating what a service can do.

## Decision

**A test only lists models.** Ollama gets `GET /api/version` and
`GET /api/tags`. OpenAI-compatible services get `GET {base}/models` with a
bearer key when one is set. Requests have no body, so nothing from the
library can be sent. A successful listing is reported as such and never as
chat or embedding support. `data: null`, which Ollama's `/v1` returns when
no models are installed, counts as an empty list.

**Go exactly where the user pointed.** The HTTP client follows no
redirects, uses no system proxies, and has a 3 second connect timeout, a
10 second total timeout, and a 1 MiB response cap. It uses rustls with the
platform certificate verifier. Outcomes are honest:

- reachable;
- no models listed;
- authentication failed (401 or 403);
- protocol mismatch (404, non-JSON, wrong shape, too large, or a
  redirect);
- network error;
- timeout;
- insecure URL rejected.

**Address rules.** Only `http` and `https`, with no user-info, no query, and
no fragment. Plain HTTP is allowed silently only to loopback, matching
`localhost` literally without DNS. Any other plain-HTTP address needs a
per-profile acknowledgement with a visible warning. There is no attempt to
tell LAN from internet. The display-only "This computer" or "Another
computer" label uses the same loopback test.

**Keys live in the OS keychain.** `keyring-core` with the platform stores:
macOS Keychain, Windows Credential Manager, and Linux Secret Service over a
pure-Rust D-Bus client. The service name is `com.altazlabs.atlas`, or
`com.altazlabs.atlas.debug` in debug builds, so development never touches
an installed Atlas's keys. The account is `provider:<profile id>`. The
catalog records only where the key is (`keychain` or `session_only`). If
the keychain is unavailable, the key is held in memory for the session and
the UI says it must be re-entered after a restart. It is never written to
disk unencrypted. Keys are redacted from `Debug` output and never appear in
IPC responses, error messages, or logs.

**No silent network calls.** Atlas contacts a service only when the user
presses Test, or presses "Look for AI services on this computer". The
second probes only loopback ports 11434, 1234, 8000, and 8080, with 1 second
timeouts. oMLX is recognized by its unauthenticated `/health` response.

## Consequences

- Users behind a mandatory HTTP proxy can't reach hosted services yet.
  Proxy support is a later, explicit setting.
- A service that changes its address with a redirect has to be entered
  with its final address.
- Profiles move with the library folder, but keys don't. On another
  computer the user re-enters each key.
- The spec's role assignment (chat versus embeddings) stays in the domain
  model only, until AI features arrive.
