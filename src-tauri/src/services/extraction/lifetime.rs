//! No extraction worker outlives the app (phase1-spec P1-22, ADR 0009).
//!
//! A normal quit stops the job, and the supervisor kills its worker before
//! the catalog closes. This module covers the app dying without that: a
//! SIGKILL, `TerminateProcess` or a crash. The worker would otherwise only
//! notice at its next pipe read or write, and one hashing a large file,
//! inside a long PDFium call or hung would live on, since the inactivity
//! timeout and the memory cap are enforced by the app.
//!
//! - **Worker side, every OS:** a watchdog thread ends the worker as soon
//!   as its parent is gone ([`watch_parent`]). It runs beside the request
//!   loop, so a busy or hung main thread doesn't delay it, and it ends the
//!   process without running exit handlers, which PDFium may be inside.
//! - **App side, Windows:** each worker joins a Job Object that kills its
//!   processes when the app's last handle to it closes ([`contain`]), so the
//!   kernel ends the worker even if its watchdog never runs.

use std::process::Child;

/// The app's process ID, set on every worker it starts. Without it (a
/// worker started by hand, as `just worker-hello` does) the worker watches
/// the parent it started under.
pub const PARENT_ENV: &str = "ATLAS_WORKER_PARENT";

/// How often the Unix watchdog looks at its parent.
#[cfg(unix)]
const POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// The parent to watch: [`PARENT_ENV`] when set and valid.
fn expected_parent() -> Option<u32> {
    parse_parent(std::env::var(PARENT_ENV).ok().as_deref())
}

fn parse_parent(value: Option<&str>) -> Option<u32> {
    value
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|pid| *pid > 0)
}

/// Starts the worker's parent watchdog. Called first thing in the worker.
pub fn watch_parent() {
    let expected = expected_parent();
    #[cfg(unix)]
    let expected = expected.or_else(|| Some(std::os::unix::process::parent_id()));
    let Some(parent) = expected else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("atlas-worker-watchdog".into())
        .spawn(move || watch(parent));
}

/// Ends this process at once: no exit handlers, no destructors.
fn die() -> ! {
    #[cfg(unix)]
    // SAFETY: `_exit` takes no pointers and never returns.
    unsafe {
        libc::_exit(0)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        // SAFETY: the pseudo-handle for this process is always valid.
        unsafe {
            TerminateProcess(GetCurrentProcess(), 0);
        }
        // TerminateProcess on the current process does not return; this
        // only satisfies the type.
        std::process::abort()
    }
}

/// Reparenting is how Unix reports a dead parent: the worker's parent
/// becomes init, launchd or a subreaper. A parent that died before the
/// worker got here is caught by the first check.
#[cfg(unix)]
fn watch(parent: u32) {
    loop {
        if std::os::unix::process::parent_id() != parent {
            die();
        }
        std::thread::sleep(POLL);
    }
}

/// Waits on the parent's process handle. If the parent can't be opened it
/// is already gone (or the PID belongs to someone else's process, which a
/// worker must not outlive either).
#[cfg(windows)]
fn watch(parent: u32) {
    use windows_sys::Win32::System::Threading::{
        INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    // SAFETY: plain Win32 calls; the handle is only waited on, and the
    // process ends right after.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, parent);
        if handle.is_null() {
            die();
        }
        WaitForSingleObject(handle, INFINITE);
    }
    die();
}

/// Puts a just-started worker in the app's kill-on-close Job Object
/// (Windows). Elsewhere, and if Windows refuses, the worker's own watchdog
/// is the guard.
pub fn contain(child: &Child) {
    #[cfg(windows)]
    windows_job::assign(child);
    #[cfg(not(windows))]
    let _ = child;
}

#[cfg(windows)]
mod windows_job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::sync::OnceLock;

    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    /// The job's handle, kept open for the life of the app on purpose: it
    /// closes when the app dies, and that is what kills the workers. Stored
    /// as an address because raw handles aren't `Send`.
    static JOB: OnceLock<Option<usize>> = OnceLock::new();

    fn job() -> Option<usize> {
        *JOB.get_or_init(|| {
            // SAFETY: plain Win32 calls with a zeroed, correctly sized
            // structure; the handle is checked before use.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return None;
                }
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                (set != 0).then_some(job as usize)
            }
        })
    }

    pub fn assign(child: &Child) {
        if let Some(job) = job() {
            // SAFETY: `child` owns a live process handle for this call.
            unsafe {
                AssignProcessToJobObject(job as _, child.as_raw_handle() as _);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parent_comes_from_the_environment_when_valid() {
        assert_eq!(parse_parent(Some("4242")), Some(4242));
        assert_eq!(parse_parent(Some(" 17\n")), Some(17));
        assert_eq!(parse_parent(Some("0")), None);
        assert_eq!(parse_parent(Some("not a pid")), None);
        assert_eq!(parse_parent(None), None);
    }
}
