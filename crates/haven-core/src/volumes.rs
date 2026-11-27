//! Mounted volumes and free space.

use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::process::Command;

const MNT_RDONLY: u32 = 0x0000_0001;
const MNT_LOCAL: u32 = 0x0000_1000;
const MNT_DONTBROWSE: u32 = 0x0010_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    pub name: String,
    pub mount: PathBuf,
    pub fs_type: String,
    pub total: u64,
    pub available: u64,
    pub read_only: bool,
    pub local: bool,
    /// Everything except the boot volume can be ejected.
    pub ejectable: bool,
}

impl Volume {
    pub fn used_fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            1.0 - self.available as f32 / self.total as f32
        }
    }
}

fn cstr(buf: &[libc::c_char]) -> String {
    // SAFETY: statfs strings are NUL-terminated within their fixed buffers.
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// Name of the boot volume ("Macintosh HD"), found via the `/Volumes` symlink.
fn root_volume_name() -> String {
    std::fs::read_dir("/Volumes")
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| {
            std::fs::read_link(e.path())
                .map(|t| t == Path::new("/"))
                .unwrap_or(false)
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .unwrap_or_else(|| "Macintosh HD".into())
}

/// User-visible mounted volumes: the boot volume plus anything under `/Volumes`.
pub fn list_volumes() -> Vec<Volume> {
    let mut buf: *mut libc::statfs = std::ptr::null_mut();
    // SAFETY: getmntinfo fills `buf` with a pointer to an array it owns.
    let count = unsafe { libc::getmntinfo(&mut buf, libc::MNT_NOWAIT) };
    if count <= 0 || buf.is_null() {
        return Vec::new();
    }
    // SAFETY: getmntinfo returned `count` valid entries at `buf`.
    let mounts = unsafe { std::slice::from_raw_parts(buf, count as usize) };
    let mut out = Vec::new();
    for m in mounts {
        let mount = cstr(&m.f_mntonname);
        let flags = m.f_flags;
        let is_root = mount == "/";
        if !is_root && (!mount.starts_with("/Volumes/") || flags & MNT_DONTBROWSE != 0) {
            continue;
        }
        let bsize = m.f_bsize as u64;
        let name = if is_root {
            root_volume_name()
        } else {
            Path::new(&mount)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        out.push(Volume {
            name,
            mount: PathBuf::from(&mount),
            fs_type: cstr(&m.f_fstypename),
            total: m.f_blocks * bsize,
            available: m.f_bavail * bsize,
            read_only: flags & MNT_RDONLY != 0,
            local: flags & MNT_LOCAL != 0,
            ejectable: !is_root,
        });
    }
    out.sort_by_key(|v| (v.ejectable, v.name.to_lowercase()));
    out
}

/// Free bytes on the volume containing `path`.
pub fn available_space(path: &Path) -> Option<u64> {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `st` is a writable statfs.
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail * st.f_bsize as u64)
}

fn is_local(mount: &Path) -> bool {
    let Ok(c) = std::ffi::CString::new(mount.as_os_str().as_encoded_bytes()) else {
        return true;
    };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `st` is a writable statfs.
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return true;
    }
    st.f_flags & libc::MNT_LOCAL as u32 != 0
}

/// The ejectable volume holding `path` (the longest mount point that
/// contains it), if any. The boot volume is never returned.
pub fn ejectable_volume_for<'a>(path: &Path, vols: &'a [Volume]) -> Option<&'a Volume> {
    vols.iter()
        .filter(|v| v.ejectable && path.starts_with(&v.mount))
        .max_by_key(|v| v.mount.as_os_str().len())
}

/// Turns `diskutil`'s output into a short, user-facing reason.
pub fn eject_error_message(stderr: &str, stdout: &str) -> String {
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    let lower = text.to_lowercase();
    if lower.contains("dissent") || lower.contains("busy") || lower.contains("in use") {
        return "it's in use by another app".into();
    }
    if lower.contains("not mounted") || lower.contains("could not find") {
        return "it's no longer mounted".into();
    }
    let line = text
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("diskutil failed");
    line.trim_end_matches('.').to_string()
}

fn diskutil(verb: &str, mount: &Path) -> Result<(), String> {
    let out = Command::new("/usr/sbin/diskutil")
        .arg(verb)
        .arg(mount)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(eject_error_message(
            &String::from_utf8_lossy(&out.stderr),
            &String::from_utf8_lossy(&out.stdout),
        ))
    }
}

/// Ejects a volume via `diskutil` (blocking). Network shares (SMB, AFP,
/// NFS) have no disk device to eject, so they are unmounted instead; a
/// local volume whose disk can't be ejected (a second partition of the
/// internal disk) falls back to an unmount too.
pub fn eject(mount: &Path) -> Result<(), String> {
    if mount == Path::new("/") {
        return Err("the startup disk can't be ejected".into());
    }
    // `diskutil` resolves any path to the whole disk holding it: only ever
    // hand it an actual mount point, or a stray path could eject (or try to
    // eject) the disk it lives on.
    if !is_mount_point(mount) {
        return Ok(());
    }
    if !is_local(mount) {
        return diskutil("unmount", mount);
    }
    match diskutil("eject", mount) {
        Ok(()) => Ok(()),
        // Some disks report an error although the volume did go away.
        Err(_) if !is_mount_point(mount) => Ok(()),
        Err(e) => diskutil("unmount", mount).map_err(|_| e),
    }
}

/// True when `path` is the root of a mounted filesystem.
fn is_mount_point(path: &Path) -> bool {
    list_volumes().iter().any(|v| v.mount == path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_volume_is_listed_first() {
        let vols = list_volumes();
        let root = vols.first().expect("at least the boot volume");
        assert_eq!(root.mount, PathBuf::from("/"));
        assert!(!root.ejectable);
        assert!(root.total > 0 && root.available <= root.total);
        assert!(available_space(Path::new("/tmp")).unwrap() > 0);
    }

    fn vol(mount: &str, ejectable: bool) -> Volume {
        Volume {
            name: mount.rsplit('/').next().unwrap_or_default().to_string(),
            mount: PathBuf::from(mount),
            fs_type: "apfs".into(),
            total: 1,
            available: 1,
            read_only: false,
            local: true,
            ejectable,
        }
    }

    #[test]
    fn finds_the_ejectable_volume_holding_a_path() {
        let vols = vec![
            vol("/", false),
            vol("/Volumes/USB", true),
            vol("/Volumes/USB 2", true),
        ];
        let find = |p: &str| ejectable_volume_for(Path::new(p), &vols).map(|v| v.name.clone());
        assert_eq!(find("/Volumes/USB/photos/a.jpg"), Some("USB".into()));
        assert_eq!(find("/Volumes/USB"), Some("USB".into()));
        // A sibling whose name shares a prefix is a different volume.
        assert_eq!(find("/Volumes/USB 2/x"), Some("USB 2".into()));
        // The boot volume is never ejectable.
        assert_eq!(find("/Users/me"), None);
    }

    #[test]
    fn eject_errors_are_short_and_readable() {
        assert_eq!(
            eject_error_message(
                "Volume USB on disk4s1 failed to unmount: dissented by PID 123 (/usr/bin/vim)\n",
                ""
            ),
            "it's in use by another app"
        );
        assert_eq!(
            eject_error_message("", "Unable to find disk for /Volumes/Gone\n"),
            "Unable to find disk for /Volumes/Gone"
        );
        assert_eq!(eject_error_message("", ""), "diskutil failed");
    }

    #[test]
    fn ejecting_an_unmounted_path_is_a_no_op() {
        // Nothing is mounted there, so there is nothing left to eject.
        assert_eq!(
            eject(Path::new("/Volumes/haven-definitely-not-mounted")),
            Ok(())
        );
        // A folder on the boot volume is not a mount point: diskutil must
        // never see it (it would resolve it to the startup disk).
        assert_eq!(eject(&std::env::temp_dir()), Ok(()));
        assert!(eject(Path::new("/")).is_err());
    }

    /// Ejects a real volume. Mount a scratch disk image first:
    ///   hdiutil create -size 20m -fs APFS -volname HavenEjectTest /tmp/t.dmg
    ///   hdiutil attach /tmp/t.dmg
    ///   HAVEN_EJECT_TEST_MOUNT=/Volumes/HavenEjectTest \
    ///     cargo test -p haven-core ejects_a_mounted -- --ignored
    #[test]
    #[ignore = "ejects a real volume; set HAVEN_EJECT_TEST_MOUNT"]
    fn ejects_a_mounted_disk_image() {
        let Some(mount) = std::env::var_os("HAVEN_EJECT_TEST_MOUNT").map(PathBuf::from) else {
            return;
        };
        let listed = |m: &Path| list_volumes().into_iter().any(|v| v.mount == m);
        assert!(listed(&mount), "{} is not a listed volume", mount.display());
        assert!(ejectable_volume_for(&mount.join("x"), &list_volumes()).is_some());
        // An open file keeps the volume busy: a readable error, still mounted.
        let busy = mount.join("haven-busy.txt");
        std::fs::write(&busy, b"busy").unwrap();
        let handle = std::fs::File::open(&busy).unwrap();
        let err = eject(&mount).expect_err("busy volume must not eject");
        assert_eq!(err, "it's in use by another app");
        assert!(listed(&mount));
        drop(handle);
        std::fs::remove_file(&busy).unwrap();
        assert_eq!(eject(&mount), Ok(()));
        assert!(!listed(&mount));
    }
}
