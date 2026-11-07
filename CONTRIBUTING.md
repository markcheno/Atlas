# Contributing to Atlas

## Ground rules

These invariants come from the specifications and override convenience:

1. **Original files are read-only.** Never rename, move, modify, or delete a
   source document, change its timestamps, or write sidecar files next to it.
2. **One Atlas data root.** Atlas-owned persistent data lives under the
   user-selected data root. The documented exceptions are OS credential
   storage, a tiny locator file, and OS-managed webview and installer caches.
3. **No server.** Tauri IPC is the only API. Do not add an HTTP server,
   daemon, database server, or Python runtime.
4. **No mandatory AI or account.** Everything except Ask Atlas must work with
   AI disabled.
5. **No fabricated data.** A feature that has not shipped returns
   `not_implemented`; it never returns sample rows, invented totals, or
   guessed hardware readings.

## Workflow

Phase 0 is built in vertical slices (see section 15 of the
[Phase 0 spec](docs/phase0-spec.md)). For each slice:

1. Implement code, tests, and doc updates for that slice only.
2. Run `pnpm check`, then `pnpm check:bindings` if you changed DTOs.
3. Launch the app (`pnpm tauri dev`) and walk through the slice's demo steps.
4. Record platform evidence in [docs/platform-validation.md](docs/platform-validation.md).
   Never mark a platform verified unless the app was actually run there.
5. Commit with the checkpoint name `phase0/slice-NN`.

## Adding an IPC command

1. Define request and response DTOs in `src-tauri/src/domain/dto.rs` with
   `#[derive(TS)] #[ts(export)]`.
2. Add a thin command in `src-tauri/src/commands/`, register it in
   `generate_handler!` in `lib.rs`, and add its name to `COMMAND_NAMES`.
3. Add a method to the facade in `src/lib/api/client.ts`, spelling the
   command name as a single-quoted literal.
4. Run `cargo test` to regenerate `src/lib/api/bindings/` and commit them.

The contract tests in `src-tauri/tests/ipc_contract.rs` fail if steps 2 and 3
disagree.

## Errors

Return `AtlasError`. It serializes to `{ code, message, recovery? }` with a
stable snake_case code. Messages must not contain secrets, document text, or
full user file paths.

## Dependencies

Prefer maintained, permissively licensed libraries. Pin exact npm versions
(pnpm is configured with `saveExact`), commit lockfiles, and run
`pnpm license-report` after adding or upgrading a dependency. Discuss any
GPL, AGPL, or commercial-license dependency before adding it.
