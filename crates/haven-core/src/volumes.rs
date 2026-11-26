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

/// Ejects a volume via `diskutil` (blocking).
pub fn eject(mount: &Path) -> Result<(), String> {
    let out = Command::new("/usr/sbin/diskutil")
        .arg("eject")
        .arg(mount)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
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
}
