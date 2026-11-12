# ADR 0001: Local IPC with no server, and one Atlas data root

- **Status:** Accepted. Its list of exceptions was amended by
  [ADR 0019](0019-export-and-import.md) (Phase 3, Slice 3.8: export folders).
- **Date:** 2026-11-07
- **Context documents:** [product spec](../product-spec.md) sections 6, 8 and 12;
  [Phase 0 spec](../phase0-spec.md) sections 4, 6 and 9

## Context

Atlas must install as one ordinary desktop application with no terminal setup,
Docker, Python, database daemon, or account. Atlas also has to grow into an
open-source self-hosted edition and a hosted service later. And the user's
documents are irreplaceable, while Atlas's indexes are large, rebuildable, and
must stay out of the user's folders.

Two easy shortcuts exist, and both cause problems later:

1. Running a local HTTP server (for example on `127.0.0.1`) and pointing the UI
   at it. Any local process or web page can then try to reach that server.
   It also needs authentication, port management, and a lifecycle of its own.
2. Letting each subsystem keep its state wherever is convenient, such as the OS
   app-data folder, the webview's local storage, or files next to the
   documents. Moving, backing up, or deleting a library then becomes guesswork.

## Decision

**Tauri IPC is Atlas's only API in the desktop edition.** The UI calls
typed commands in the same process. There is no HTTP listener, daemon, or
database server. Commands are thin adapters over services that do not depend
on Tauri, so a future authenticated, versioned HTTP API can wrap the same
services. That API would be opt-in for self-hosting, or server-side for Atlas
Cloud. It is never exposed by default.

**All Atlas-owned persistent data lives under one user-selected Atlas Data
Root.** That covers the manifest, settings, SQLite catalog, extracted text,
vector indexes, thumbnails, logs, exports and backups (the log and
diagnostics reports since Phase 4, [ADR 0021](0021-diagnostics.md)). Source folders are
referenced by path and never written to. The documented exceptions are:

- API keys, which live in OS credential storage;
- a tiny OS-managed locator file recording where the data root is;
- webview, installer and other caches the OS manages outside the root;
- storage belonging to separately installed runtimes such as Ollama;
- export folders the user chooses in a native dialog, and only the `Atlas`
  folder Atlas makes inside one (Phase 3, Slice 3.8;
  [ADR 0019](0019-export-and-import.md)).

Network and cloud-synced volumes are rejected as a data root, because SQLite
WAL and file locking are unreliable there.

## Consequences

- No port to secure, no loopback attack surface, and no process left behind
  after Atlas quits.
- Remote access to a desktop library before sync exists has no built-in path.
  This remains an open product decision.
- Every service signature must stay transport-independent. Commands must hold
  no business logic.
- Backup, migration, quota and cleanup features operate on one directory tree.
- Browser storage is not used for preferences, even trivial ones. Until the
  data root exists (Slice 0.2), preferences such as theme last only for the
  session.
- Integration tests can create a data root in a temporary directory and
  assert that nothing is written elsewhere.
