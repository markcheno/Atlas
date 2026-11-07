# ADR 0002: Data-root creation, locking, and compatibility

- **Status:** Accepted
- **Date:** 2026-11-07
- **Context documents:** [Phase 0 spec](../phase0-spec.md) sections 5.2, 6 and 6.1;
  [product spec](../product-spec.md) section 8; [ADR 0001](0001-local-ipc-no-server-single-data-root.md)

## Context

ADR 0001 put all Atlas-owned data under one user-chosen data root. That root
may live on an external drive that is sometimes unplugged. It must survive
crashes during setup, and it must never be written by two Atlas processes at
once. It also has to be refused by an older Atlas that does not understand
it. And it must not end up on storage where SQLite is unsafe.

## Decision

**A root is valid only once `atlas.json` exists.** Creation first writes an
`.atlas-init` marker, then takes the lock and creates the layout, catalog and
settings. The manifest is written last, through a staging file that is
fsynced and renamed into place. After that the marker is removed. A crash at
any point before the rename leaves a folder that is not a valid root.
Choosing that folder again finishes the job, and the library ID survives
because it is read back from the catalog's `workspace` row. Opening never
creates a root. A remembered root that is missing is reported, and never
recreated, so an unplugged drive cannot turn into an empty library.

**One process per root, enforced by an OS file lock.** `atlas.lock` is opened
and locked with `std::fs::File::try_lock`, which uses `flock` on Unix and
`LockFileEx` on Windows. The lock is held for as long as the root is open.
The OS releases it when the process exits or crashes, so a leftover lock file
never blocks a later launch. The file's contents are informational only.
Inside one process, the catalog writer is declared before the lock, so
dropping an open root closes SQLite first and releases the lock second.

**The catalog is authoritative for its schema.** The highest
`schema_migrations.version` is the catalog's version. The manifest keeps an
advisory copy, so a library can be assessed without opening SQLite. On open,
the version is read before journal mode is changed or any migration runs. A
version newer than this build supports is refused, and the database file
stays byte-identical. The manifest's `layoutVersion` guards the directory
layout the same way.

**Network and cloud-synced locations are refused.** SQLite WAL needs shared
memory, which network filesystems do not provide reliably, and advisory locks
are unreliable there too. Sync clients can also replace a live database file
with a stale copy. Detection works as follows:

- **macOS:** `statfs` reports the filesystem type, and `smbfs`, `nfs`,
  `afpfs`, `webdav`, `cifs`, `ftp` and `9p` count as network filesystems.
- **Linux:** `statfs` reports a filesystem magic number. NFS, SMB, CIFS,
  SMB2, 9p, AFS, Ceph and Lustre count as network. FUSE is reported as
  unknown and allowed, because it may be local.
- **Windows:** a UNC path, or `GetDriveTypeW` returning `DRIVE_REMOTE`,
  counts as network.
- **Cloud sync, best effort:** paths under `~/Library/Mobile Documents` or
  `~/Library/CloudStorage` on macOS, under the OneDrive environment folders on
  Windows, or under any folder containing a `.dropbox` marker.

If SQLite still cannot enter WAL mode, opening fails with the
network-volume error.

**Low space blocks creation, not opening.** A new root needs free space of at
least `max(2 GiB, min(10 GiB, 2% of capacity))` on its volume. That is the
same threshold the 0.4 and 0.5 storage guard will use. An existing library
on a full drive still opens, so the user can read their catalog.

**Atlas never writes into a folder that holds other files.** A non-empty
folder is refused, and Atlas offers an "Atlas Library" subfolder inside it.
OS clutter such as `.DS_Store` and `Thumbs.db` does not count as content.
A folder inside an existing root is refused.

## Consequences

- The user can unplug and replug a data drive safely, and Atlas tells them
  what happened.
- An unrecognized cloud-sync folder is reported as local. This is a
  documented limitation, and the copy on the data-location screen warns about
  sync folders.
- Data-root paths must be valid Unicode. That restriction does not apply to
  source paths.
- Moving a root is not supported in Phase 0. Choosing a different location
  opens or creates a separate root, and the old one stays intact.
- The locator lives outside the root by necessity. Losing it loses no data:
  the user can choose the root's folder again and Atlas opens it.
