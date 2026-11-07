//! OS implementation of [`VolumeInspector`].
//!
//! Network detection uses the filesystem type the OS reports. Cloud-sync
//! detection is a best-effort path heuristic: sync clients do not mark
//! their folders in a standard way, so an unrecognized sync folder is
//! reported as local. Both checks are documented in ADR 0002.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::domain::dto::VolumeKind;
use crate::domain::traits::{VolumeInfo, VolumeInspector};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::fsutil::nearest_existing_ancestor;

#[derive(Debug, Default, Clone, Copy)]
pub struct OsVolumeInspector;

impl VolumeInspector for OsVolumeInspector {
    fn inspect(&self, path: &Path) -> AtlasResult<VolumeInfo> {
        let existing = nearest_existing_ancestor(path).ok_or(AtlasError::InvalidPath)?;
        let mut info = platform::inspect(&existing)
            .map_err(AtlasError::io("read information about that drive"))?;
        if info.kind == VolumeKind::Local
            && is_cloud_synced(path, std::env::home_dir().as_deref(), &|k| {
                std::env::var_os(k)
            })
        {
            info.kind = VolumeKind::CloudSynced;
        }
        Ok(info)
    }
}

/// Recognizes common cloud-sync folders by location.
pub fn is_cloud_synced(
    path: &Path,
    home: Option<&Path>,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> bool {
    if let Some(home) = home {
        // macOS iCloud Drive and File Provider clients (Dropbox, OneDrive,
        // Google Drive, Box) live under these two folders.
        for sync_root in ["Library/Mobile Documents", "Library/CloudStorage"] {
            if path.starts_with(home.join(sync_root)) {
                return true;
            }
        }
    }
    // Windows OneDrive publishes its folders in the environment.
    for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
        if let Some(dir) = env(var).filter(|d| !d.is_empty())
            && path.starts_with(PathBuf::from(dir))
        {
            return true;
        }
    }
    // Classic Dropbox keeps a marker folder at its root.
    path.ancestors()
        .any(|dir| dir.join(".dropbox").exists() || dir.join(".dropbox.cache").exists())
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::io;
    use std::os::unix::ffi::OsStrExt;

    const NETWORK_FS: &[&str] = &["smbfs", "nfs", "afpfs", "webdav", "cifs", "ftp", "9p"];

    pub fn inspect(path: &Path) -> io::Result<VolumeInfo> {
        let c_path = CString::new(path.as_os_str().as_bytes())?;
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: c_path is NUL-terminated; statfs fills the struct on success.
        if unsafe { libc::statfs(c_path.as_ptr(), stat.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: statfs returned 0, so the struct is initialized.
        let stat = unsafe { stat.assume_init() };
        // SAFETY: f_fstypename is a NUL-terminated C string from the kernel.
        let fs_type = unsafe { CStr::from_ptr(stat.f_fstypename.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let block = u64::from(stat.f_bsize);
        let kind = if NETWORK_FS.contains(&fs_type.as_str()) {
            VolumeKind::Network
        } else {
            VolumeKind::Local
        };
        Ok(VolumeInfo {
            free_bytes: Some(stat.f_bavail.saturating_mul(block)),
            total_bytes: Some(stat.f_blocks.saturating_mul(block)),
            fs_type: Some(fs_type),
            kind,
        })
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::ffi::CString;
    use std::io;
    use std::os::unix::ffi::OsStrExt;

    // Magic numbers from linux/magic.h and statfs(2).
    const NETWORK_MAGIC: &[(i64, &str)] = &[
        (0x6969, "nfs"),
        (0x517B, "smb"),
        (0xFF53_4D42, "cifs"),
        (0xFE53_4D42, "smb2"),
        (0x0102_1997, "9p"),
        (0x5346_414F, "afs"),
        (0x00C3_6400, "ceph"),
        (0x0BD0_0BD0, "lustre"),
    ];
    const FUSE_MAGIC: i64 = 0x6573_5546;

    // libc field widths differ by architecture, so some casts are no-ops.
    #[allow(clippy::unnecessary_cast)]
    pub fn inspect(path: &Path) -> io::Result<VolumeInfo> {
        let c_path = CString::new(path.as_os_str().as_bytes())?;

        let mut fs = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: c_path is NUL-terminated; statfs fills the struct on success.
        if unsafe { libc::statfs(c_path.as_ptr(), fs.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: statfs returned 0, so the struct is initialized.
        let fs = unsafe { fs.assume_init() };
        let magic = (fs.f_type as i64) & 0xFFFF_FFFF;

        let mut vfs = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: as above, for statvfs.
        if unsafe { libc::statvfs(c_path.as_ptr(), vfs.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: statvfs returned 0, so the struct is initialized.
        let vfs = unsafe { vfs.assume_init() };
        let frag = vfs.f_frsize as u64;

        let network = NETWORK_MAGIC.iter().find(|(m, _)| *m == magic);
        let (kind, fs_type) = match network {
            Some((_, name)) => (VolumeKind::Network, Some((*name).to_owned())),
            // FUSE may be sshfs or rclone (remote) or a local overlay.
            None if magic == FUSE_MAGIC => (VolumeKind::Unknown, Some("fuse".to_owned())),
            None => (VolumeKind::Local, Some(format!("0x{magic:x}"))),
        };
        Ok(VolumeInfo {
            free_bytes: Some((vfs.f_bavail as u64).saturating_mul(frag)),
            total_bytes: Some((vfs.f_blocks as u64).saturating_mul(frag)),
            fs_type,
            kind,
        })
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW,
    };

    const DRIVE_REMOTE: u32 = 4;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn inspect(path: &Path) -> io::Result<VolumeInfo> {
        let path_w = wide(path);

        let mut free_to_caller = 0u64;
        let mut total = 0u64;
        // SAFETY: path_w is NUL-terminated; out-pointers are valid u64s.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                path_w.as_ptr(),
                &mut free_to_caller,
                &mut total,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut root = vec![0u16; 1024];
        // SAFETY: root is a writable buffer of the stated length.
        let have_root =
            unsafe { GetVolumePathNameW(path_w.as_ptr(), root.as_mut_ptr(), root.len() as u32) }
                != 0;

        let is_unc = path.as_os_str().to_string_lossy().starts_with(r"\\")
            && !path.as_os_str().to_string_lossy().starts_with(r"\\?\");
        // SAFETY: root is NUL-terminated when have_root is true.
        let remote = have_root && unsafe { GetDriveTypeW(root.as_ptr()) } == DRIVE_REMOTE;

        let mut fs_name = vec![0u16; 64];
        // SAFETY: buffers are writable with the stated lengths; unused outputs are null.
        let have_fs = have_root
            && unsafe {
                GetVolumeInformationW(
                    root.as_ptr(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    fs_name.as_mut_ptr(),
                    fs_name.len() as u32,
                )
            } != 0;
        let fs_type = have_fs.then(|| {
            let end = fs_name
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(fs_name.len());
            String::from_utf16_lossy(&fs_name[..end]).to_lowercase()
        });

        Ok(VolumeInfo {
            free_bytes: Some(free_to_caller),
            total_bytes: Some(total),
            fs_type,
            kind: if is_unc || remote {
                VolumeKind::Network
            } else {
                VolumeKind::Local
            },
        })
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod platform {
    use super::*;
    pub fn inspect(_path: &Path) -> std::io::Result<VolumeInfo> {
        Ok(VolumeInfo {
            free_bytes: None,
            total_bytes: None,
            fs_type: None,
            kind: VolumeKind::Unknown,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_dir_is_a_local_volume_with_space() {
        let dir = tempfile::tempdir().unwrap();
        let info = OsVolumeInspector
            .inspect(&dir.path().join("not/yet/created"))
            .unwrap();
        assert_eq!(info.kind, VolumeKind::Local);
        assert!(info.total_bytes.unwrap() > 0);
        assert!(info.free_bytes.unwrap() <= info.total_bytes.unwrap());
        assert!(info.fs_type.is_some());
    }

    #[test]
    fn cloud_heuristics() {
        let home = Path::new("/Users/someone");
        let no_env = |_: &str| None;
        assert!(is_cloud_synced(
            Path::new("/Users/someone/Library/Mobile Documents/com~apple~CloudDocs/Atlas"),
            Some(home),
            &no_env,
        ));
        assert!(is_cloud_synced(
            Path::new("/Users/someone/Library/CloudStorage/Dropbox/Atlas"),
            Some(home),
            &no_env,
        ));
        assert!(!is_cloud_synced(
            Path::new("/Users/someone/Library/Application Support/com.altazlabs.atlas/Library"),
            Some(home),
            &no_env,
        ));

        let onedrive = |k: &str| (k == "OneDrive").then(|| OsString::from("/onedrive/me"));
        assert!(is_cloud_synced(
            Path::new("/onedrive/me/Atlas"),
            None,
            &onedrive
        ));
        assert!(!is_cloud_synced(Path::new("/other/Atlas"), None, &onedrive));
    }

    #[test]
    fn dropbox_marker_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".dropbox")).unwrap();
        let no_env = |_: &str| None;
        assert!(is_cloud_synced(
            &dir.path().join("Atlas Library"),
            None,
            &no_env
        ));
    }
}
