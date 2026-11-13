# ADR 0022: Automatic rescans, and a rescan that writes only what changed

- **Status:** Accepted (Phase 4, Slice 4.5). Slice 4.6 completes it with
  schedules. Amends [ADR 0005](0005-inventory-durability.md).
- **Date:** 2026-11-13
- **Context documents:** [Phase 4 spec](../phase4-spec.md) §5.3, §5.4,
  §6.3, §7.4, §12 (P4-11, P4-12), §13; [ADR 0004](0004-resource-policy.md)
  (the resource policy); [ADR 0009](0009-jobs-and-extraction.md) (jobs)

## Context

Until Phase 4, a folder went stale until its owner pressed Rescan. A rescan
also cost about what the first scan did, 79 to 99 s at 30,000 files. That
was because the upsert rewrote every row to stamp it with the session
(`last_seen_session_id`), and the stamp was how the end of the scan found
files that had gone.

That stamp did four jobs:

- told replays apart;
- found unseen rows at the end;
- counted the files found;
- counted live progress.

Three more facts shaped the design:

- **Nothing watched for change.** Reachability was probed on demand, and
  Atlas had no timer besides the knowledge store's hourly one.
- **A scan paused for a missing folder held the scan runner.** It waited
  inside the runner for the folder to return, so every other folder's
  scan waited too.
- **Nothing about a scan reached the screen once it ended.** No count of
  new, changed or missing files, and failed folders were deleted with the
  frontier.

## Decision

**A rescan compares, then writes only what differs.** Each listed file is
looked up by `(source_root_id, rel_path)` in the batch's transaction. A row
is left alone when all of these hold:

- the same size and whole-second modification time;
- `available`;
- the same storage;
- not a cloud-skipped file now stored locally, which must still be read
  (P3-01).

New and changed files are upserted as before and counted.

**Gone files are found by missing candidates.** When a folder has been
listed in full, the rows under it are read by a range scan of the
`(source_root_id, rel_path)` index. Each row is checked for whether its
first name below the folder was in the listing. Those that weren't go into
`scan_missing` (session, file).

At the end, the candidates are marked missing by ADR 0005's rules. A
folder that failed to list notes none, so its subtree is protected. The
spec proposed a temporary table of the rows seen. That would be 30,000
writes per rescan, and lost on a kill. Candidates are usually few, and they
persist with the frontier, so a resumed rescan finishes correctly.

**Unchanged work is held, then committed together.** A batch with no new
files writes nothing. The scan holds it in memory: the subfolders found,
the folders completed, the counters, any changed files and any missing
candidates. Held folders are skipped when choosing the next folder.

Held work goes in with the next commit that adds a file, or when any of
these is reached:

- 256 folders;
- 1,000 files or candidates;
- 5 s;
- the end of the frontier in the catalog.

If Atlas stops first, the held folders are still pending, and only their
listing is repeated, as after any kill. New files are written at once, so a
first scan shows rows as it goes. Before holding was added, a no-change
rescan wrote no file row but still made about 650 commits, about 25 MiB of
WAL at 16 KiB pages.

**A check is an ordinary scan job, with an origin.** `jobs.origin` (added,
launch, user, automatic) is set when a scan is queued. It is NULL for jobs
from before 4.5, which finish by ADR 0005's original rule. The job row
counts `discovered_files`, `new_files`, `changed_files` and
`missing_files`. Failed folders become the scan's problems
(`scan_problems`: up to 200 a scan, kept for a folder's last 10 scans).

**The folder checker** (`inventory/auto.rs`) is a thread for each open
library, owned by `OpenRoot` and stopped on drop. It ticks once a minute,
and the first tick runs at open. `check_if_due` reads the workspace `Clock`
and is tested with a `TestClock`, as the knowledge store's ticker is.
Scans and the jobs queued for them are stamped by the same clock, so
"Checked … ago" and due-ness agree.

Each tick probes every folder. It queues an automatic scan for a reachable,
non-nested folder with no active scan when the setting isn't "Only when I
ask" and either:

- its last scan is older than the interval, counting from the later of its
  last completion and its latest scan's creation, so a cancelled or failed
  check isn't retried every minute;
- it was unreachable at the last tick and is reachable now.

A change in reachability is emitted as `inventory-status`.

**A missing folder no longer holds the runner.** A scan whose folder goes
away pauses with `source_root_missing` and returns to the runner.
`next_runnable` skips such jobs, and the checker queues them again when the
folder is back.

**The interval is `background.checkFolders`:** hourly, every 6 hours (the
default), daily, or never. An unknown value reads as the default, rather
than resetting the whole settings file. `update_settings` hands it to the
checker through a shared value.

**The scan UI says what happened** (the product owner's request):

- **Folder rows:**
  - "Checked 2 h ago", then what the last check found ("No changes",
    "3 new · 1 changed · 2 missing"), whether it was stopped or didn't
    finish, and "Couldn't read 2 folders";
  - an away folder says what happens next;
  - the status badge has a symbol beside its word.
- **Per-folder controls:** Pause or Resume and Cancel on a folder being
  cataloged, and **Check all folders now** in the panel. Rescan on a folder
  the user paused resumes that scan.
- **Recent checks and Couldn't read:** disclosures on each folder, the
  last 10 checks and the last check's problems, each problem with its
  reason and what to do.
- **The status bar:** "Checking Books for changes" for Atlas's own check,
  with no file count (a check that finds nothing says nothing), and **All
  activity** listing every folder's cataloging, reading and meaning at
  once.
- **Errors:** a control that fails says why, and stays said after the
  refresh that follows.

## Consequences

- **A no-change rescan of 30,000 files** takes about 4 s at the policy rate
  on the M5 Pro, writes no file row, and writes 1.06 MiB to the catalog,
  in under 100 WAL frames ([benchmarks](../benchmarks.md#rescans-slice-45)).
  The rate limit still charges each directory a full operation and each
  unchanged file a twentieth. Most of the time is the rate limit, not the
  catalog.
- **A rescan that changes 1% of files** writes their rows and the index
  pages each one touches, about 70 KiB a changed file at 16 KiB pages.
- **Held work dropped by a pause** loses up to 5 s of the scan's recorded
  running time; the folders themselves are listed again.
- **A check that finds nothing changes nothing on screen** but its
  "Checked … ago".
- **The folder-check thread probes every folder once a minute.** A probe is
  a `stat` of the folder on its own thread, under the probe timeout the
  folder list already uses. A stalled network mount reads as unavailable
  and delays only that tick, never a scan or the UI.
- **No file watcher.** Changes are found on the interval, at launch and
  when a drive returns, or when asked. A watcher remains out of scope
  (phase4-spec §2).
- **Slice 4.6** adds the schedule (modes, hours, full speed, the busy
  rule). Automatic scans already obey the resource policy.
