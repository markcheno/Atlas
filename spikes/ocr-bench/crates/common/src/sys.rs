//! Process measurements: peak memory (Slice 0.9's) and CPU time, whose
//! ratio to wall time shows whether an engine kept to one thread.

pub use xbench::peak_rss_bytes;

/// User and system CPU time of this process so far, in milliseconds.
pub fn cpu_ms() -> f64 {
    #[cfg(unix)]
    unsafe {
        let mut u: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut u);
        let ms = |t: libc::timeval| t.tv_sec as f64 * 1e3 + t.tv_usec as f64 / 1e3;
        ms(u.ru_utime) + ms(u.ru_stime)
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        );
        // 100 ns units.
        let ms = |t: FILETIME| {
            ((u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)) as f64 / 1e4
        };
        ms(kernel) + ms(user)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cpu_time_grows_with_work() {
        let before = super::cpu_ms();
        let mut x = 0u64;
        for i in 0..50_000_000u64 {
            x = x.wrapping_mul(31).wrapping_add(i);
        }
        std::hint::black_box(x);
        assert!(super::cpu_ms() > before);
        assert!(super::peak_rss_bytes() > 0);
    }
}
