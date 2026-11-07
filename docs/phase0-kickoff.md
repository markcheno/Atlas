# Atlas Phase 0 — Coding Agent Kickoff

You are implementing **Atlas**, a local-first personal knowledge library. Read both specifications in full:

1. [`product-spec.md`](product-spec.md) (v0.2) — product principles and long-term direction.
2. [`phase0-spec.md`](phase0-spec.md) (v0.1, revision 2) — authoritative Phase 0 implementation scope, acceptance criteria, and tasks.

**Implement only Slice 0.1 in this first run.** Create actual working code, not another design document.

Requirements for Slice 0.1:

- Scaffold a Tauri v2 desktop app with Svelte, TypeScript, Vite, Rust, and reproducible package manifests/lockfiles.
- Support macOS, Windows, and Linux architecture from the beginning; run primary-host validation and create a cross-platform CI skeleton.
- Produce a simple, polished shell with Welcome/Home, Library, and Settings; system light/dark mode and basic keyboard accessibility.
- Create organized Rust modules for catalog, workspace/data-root, inventory, scheduler, system resource probes, model providers, and source opening; **interfaces only**, not speculative implementations.
- Add a typed Tauri IPC client facade with clear `NotImplemented` responses for unavailable actions. Don't fabricate document data.
- Add formatting, linting, typechecking, and test commands; run them, report results.
- Document setup, architecture, constraints, and the deliberate no-server design.
- Copy the three specification files into `docs/` under the stable names given in the implementation spec's repository layout (`product-spec.md`, `phase0-spec.md`, `phase0-kickoff.md`) and fix the cross-links; leave the originals at the repository root untouched.

**Do not implement** indexing, OCR, embeddings, chat, cloud, a backend daemon, or full plugin loading in Slice 0.1. Do not install dependencies at end-user runtime. Do not access or mutate the user's real documents.

The critical architecture invariants are: original source files remain read-only; Atlas-owned generated persistent data belongs under one configurable root; battery-aware indexing will be resumable; no mandatory AI or account; internal services can eventually be exposed via a secure versioned API; extension interfaces exist but third-party plugin loading does not.

After finishing, return:

1. Files changed and implementation summary.
2. Exact commands executed and passing/failing test results.
3. Local launch command.
4. Any cross-platform limitations that were **not** actually verified.
5. What Slice 0.2 should implement next, without starting it.

Do not claim success if the app does not launch. If the environment can't launch a GUI, report the limitation and provide reproducible validation instructions; still finish all headless checks possible.
