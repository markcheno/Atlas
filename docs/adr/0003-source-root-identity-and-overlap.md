# ADR 0003: Source-root identity, overlap, and status

- **Status:** Accepted
- **Date:** 2026-11-07
- **Context documents:** [Phase 0 spec](../phase0-spec.md) sections 5.3, 6, 6.1 and 7.1;
  [ADR 0002](0002-data-root-creation-locking-and-compatibility.md)

## Context

A library folder, or *source root*, is a folder the user authorizes Atlas to
read. Users pick these folders with a native dialog, and they often live on
external drives. Three problems follow:

1. **One folder, many paths.** The same folder can be reached by several
   paths: through symlinks, through different capitalization on
   case-insensitive volumes, or through different Unicode normalizations.
   Comparing path strings would let one folder be added twice, or let a
   folder slip inside one that is already present.
2. **Lossy names.** A folder name is not guaranteed to be valid Unicode.
3. **Changing availability.** Whether a folder can be read changes over time,
   as drives are unplugged and replugged and permissions are revoked.

The scanner in Slice 0.5 must never enumerate the same files twice, and it
must never scan Atlas's own data.

## Decision

**Store canonical native paths, losslessly.** Atlas canonicalizes the
selected folder (`fs::canonicalize`) and stores it as platform-tagged native
bytes. On Unix that is the raw bytes; on Windows it is UTF-16 code units,
little-endian. A lossy UTF-8 copy is kept for display only. A path tagged
for another platform does not decode, so a catalog moved across operating
systems reports those folders as unavailable and never misreads them. The
native folder picker runs in Rust, so non-Unicode paths never cross IPC.

**Compare folders by filesystem identity when adding.** The `same-file` crate
supplies a filesystem identity for each folder: device and inode on Unix,
volume serial number and file index on Windows. A new folder is refused when
any of these holds:

- it is the same folder as an existing root;
- one of its ancestors is an existing root;
- it is an ancestor of an existing root.

If an existing root's drive is absent, the comparison falls back to path
components of its stored canonical path. The data root follows the same
rules: the root itself and anything inside it are refused. A source that
contains the data root is allowed and flagged.

**Resolve existing overlaps deterministically.** Overlapping roots can exist
only in a catalog that was imported or edited by hand. When listing, Atlas
compares stored canonical paths by component, which does not depend on what
is mounted. The outermost root owns the subtree, and every root inside it
reports `nestedInRootId` pointing to that outermost root. Slice 0.5 scans
only roots without `nestedInRootId`, so no file is enumerated twice. Each
file is attributed to the outermost root.

**Availability is live, not stored.** The spec's `source_roots.status`
column is deliberately omitted, because a stored status goes stale the
moment a drive is unplugged. Each time folders are listed, Atlas probes each
one with a single `read_dir` on its own thread, within a 2 second budget for
the whole list. A folder is reported `available`, `no_access` (permission
denied), or `unavailable` (missing, not responding, or undecodable).
Removing a folder from Atlas deletes only its catalog row. Files on disk are
never touched.

## Consequences

- Adding the same folder twice under different spellings is refused on every
  platform where `same-file` reports identity.
- A probe of a hung network mount can leave a blocked thread behind until
  the OS times out. Atlas holds no lock while it waits, and the UI is never
  blocked.
- Status shown in the UI is correct as of the last time the list was shown.
  Live notification of mount and unmount events is not implemented.
- Folders on a drive that is currently absent are compared by path only.
  That could miss an alias of an absent folder, an acceptable gap because
  inventory never runs on an absent root.
