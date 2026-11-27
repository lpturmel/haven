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

/// What `diskutil info` says about the disk behind a volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskInfo {
    /// The physical whole disk ("disk4"). For APFS volumes this is the disk
    /// holding the container's physical store, not the synthesized container.
    pub physical_disk: String,
    pub internal: bool,
    pub ejectable: bool,
}

/// "disk4s2" → "disk4".
fn whole_disk(dev: &str) -> String {
    let digits_end = dev
        .strip_prefix("disk")
        .map(|rest| 4 + rest.chars().take_while(char::is_ascii_digit).count())
        .unwrap_or(dev.len());
    dev[..digits_end].to_string()
}

/// Parses `diskutil info -plist <mount>`.
pub fn parse_disk_info(plist_bytes: &[u8]) -> Option<DiskInfo> {
    let value: plist::Value = plist::from_bytes(plist_bytes).ok()?;
    let dict = value.as_dictionary()?;
    let flag = |k: &str| dict.get(k).and_then(|v| v.as_boolean()).unwrap_or(false);
    let store = dict
        .get("APFSPhysicalStores")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_dictionary())
        .and_then(|d| d.get("APFSPhysicalStore"))
        .and_then(|v| v.as_string());
    let parent = dict.get("ParentWholeDisk").and_then(|v| v.as_string());
    let physical_disk = whole_disk(store.or(parent)?);
    Some(DiskInfo {
        physical_disk,
        internal: flag("Internal"),
        ejectable: flag("Ejectable") || flag("RemovableMediaOrExternalDevice"),
    })
}

/// `diskutil info` for a mount point (blocking).
pub fn disk_info(mount: &Path) -> Option<DiskInfo> {
    let out = Command::new("/usr/sbin/diskutil")
        .args(["info", "-plist"])
        .arg(mount)
        .output()
        .ok()?;
    out.status.success().then(|| parse_disk_info(&out.stdout))?
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EjectMode {
    /// Eject the whole physical disk (and every volume on it).
    Eject,
    /// Only unmount this volume; its disk stays attached (internal disks,
    /// the startup disk, network shares, unknown disks).
    Unmount,
}

/// How to take a volume away, decided before touching anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EjectPlan {
    pub mount: PathBuf,
    pub mode: EjectMode,
    /// Other mounted volumes on the same physical disk: ejecting takes them
    /// too, so the user is asked first and their tabs are moved away.
    pub siblings: Vec<PathBuf>,
}

/// Pure decision for [`eject_plan`]: `disks` maps mounted volumes to their
/// disk, `boot` is the startup volume's disk.
pub fn plan_eject(
    mount: &Path,
    disks: &[(PathBuf, DiskInfo)],
    boot: Option<&DiskInfo>,
) -> EjectPlan {
    let unmount = EjectPlan {
        mount: mount.to_path_buf(),
        mode: EjectMode::Unmount,
        siblings: Vec::new(),
    };
    let Some((_, me)) = disks.iter().find(|(m, _)| m == mount) else {
        return unmount;
    };
    let on_boot_disk = boot.is_some_and(|b| b.physical_disk == me.physical_disk);
    if on_boot_disk || (me.internal && !me.ejectable) {
        return unmount;
    }
    let siblings = disks
        .iter()
        .filter(|(m, d)| m != mount && m != Path::new("/") && d.physical_disk == me.physical_disk)
        .map(|(m, _)| m.clone())
        .collect();
    EjectPlan {
        mount: mount.to_path_buf(),
        mode: EjectMode::Eject,
        siblings,
    }
}

/// Plans ejecting the volume mounted at `mount` (blocking: runs `diskutil
/// info` for each mounted volume). Anything that isn't a listed mount point
/// gets an unmount plan, which [`eject`] turns into a no-op.
pub fn eject_plan(mount: &Path) -> EjectPlan {
    if !is_mount_point(mount) {
        return plan_eject(mount, &[], None);
    }
    if !is_local(mount) {
        return plan_eject(mount, &[], None);
    }
    let vols = list_volumes();
    let boot = disk_info(Path::new("/"));
    let disks: Vec<(PathBuf, DiskInfo)> = vols
        .iter()
        .filter(|v| v.ejectable && v.local)
        .filter_map(|v| disk_info(&v.mount).map(|d| (v.mount.clone(), d)))
        .collect();
    plan_eject(mount, &disks, boot.as_ref())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EjectOutcome {
    /// The disk is gone: it can be unplugged.
    Ejected,
    /// The volume is unmounted; its disk is still attached.
    Unmounted,
}

/// Carries out an [`EjectPlan`] (blocking). Reports `Ejected` only when the
/// volume and all its siblings really went away.
pub fn eject(plan: &EjectPlan) -> Result<EjectOutcome, String> {
    let mount = plan.mount.as_path();
    if mount == Path::new("/") {
        return Err("the startup disk can't be ejected".into());
    }
    // `diskutil` resolves any path to the whole disk holding it: only ever
    // hand it an actual mount point, or a stray path could eject (or try to
    // eject) the disk it lives on.
    if !is_mount_point(mount) {
        return Ok(EjectOutcome::Ejected);
    }
    if plan.mode == EjectMode::Unmount || !is_local(mount) {
        return diskutil("unmount", mount).map(|()| EjectOutcome::Unmounted);
    }
    let result = diskutil("eject", mount);
    let still: Vec<&PathBuf> = std::iter::once(&plan.mount)
        .chain(&plan.siblings)
        .filter(|m| is_mount_point(m))
        .collect();
    match (result, still.first()) {
        // Some disks report an error although everything did go away.
        (_, None) => Ok(EjectOutcome::Ejected),
        (Err(e), Some(m)) if *m == &plan.mount => Err(e),
        (r, Some(m)) => {
            let name = m
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| m.display().to_string());
            let why = r
                .err()
                .unwrap_or_else(|| "it's in use by another app".into());
            Err(format!("“{name}” on the same disk is still mounted: {why}"))
        }
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
        let gone = Path::new("/Volumes/haven-definitely-not-mounted");
        assert_eq!(eject_plan(gone).mode, EjectMode::Unmount);
        assert_eq!(eject(&eject_plan(gone)), Ok(EjectOutcome::Ejected));
        // A folder on the boot volume is not a mount point: diskutil must
        // never see it (it would resolve it to the startup disk).
        let tmp = std::env::temp_dir();
        assert_eq!(eject(&eject_plan(&tmp)), Ok(EjectOutcome::Ejected));
        assert!(eject(&eject_plan(Path::new("/"))).is_err());
    }

    fn disk(physical: &str, internal: bool, ejectable: bool) -> DiskInfo {
        DiskInfo {
            physical_disk: physical.into(),
            internal,
            ejectable,
        }
    }

    #[test]
    fn plans_whole_disk_ejects_with_siblings() {
        let boot = disk("disk0", true, false);
        let disks = vec![
            (PathBuf::from("/Volumes/A"), disk("disk8", false, true)),
            (PathBuf::from("/Volumes/B"), disk("disk8", false, true)),
            (PathBuf::from("/Volumes/USB"), disk("disk9", false, true)),
        ];
        let plan = plan_eject(Path::new("/Volumes/A"), &disks, Some(&boot));
        assert_eq!(plan.mode, EjectMode::Eject);
        assert_eq!(plan.siblings, vec![PathBuf::from("/Volumes/B")]);
        let plan = plan_eject(Path::new("/Volumes/USB"), &disks, Some(&boot));
        assert_eq!((plan.mode, plan.siblings.len()), (EjectMode::Eject, 0));
    }

    #[test]
    fn internal_and_boot_disk_volumes_are_only_unmounted() {
        let boot = disk("disk0", true, false);
        let disks = vec![
            // An extra APFS volume in the startup container.
            (PathBuf::from("/Volumes/Data2"), disk("disk0", true, false)),
            // A second internal disk.
            (PathBuf::from("/Volumes/Inner"), disk("disk2", true, false)),
            // An SD card in the built-in reader: internal but ejectable.
            (PathBuf::from("/Volumes/SD"), disk("disk3", true, true)),
        ];
        let plan = |m: &str| plan_eject(Path::new(m), &disks, Some(&boot));
        assert_eq!(plan("/Volumes/Data2").mode, EjectMode::Unmount);
        assert!(plan("/Volumes/Data2").siblings.is_empty());
        assert_eq!(plan("/Volumes/Inner").mode, EjectMode::Unmount);
        assert_eq!(plan("/Volumes/SD").mode, EjectMode::Eject);
        // Unknown volume: the safe choice.
        assert_eq!(plan("/Volumes/Unknown").mode, EjectMode::Unmount);
    }

    #[test]
    fn parses_diskutil_info() {
        let apfs = br#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>APFSPhysicalStores</key><array><dict><key>APFSPhysicalStore</key><string>disk6s1</string></dict></array>
<key>Ejectable</key><true/><key>Internal</key><false/>
<key>ParentWholeDisk</key><string>disk7</string>
</dict></plist>"#;
        assert_eq!(parse_disk_info(apfs), Some(disk("disk6", false, true)));
        let hfs = br#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>Ejectable</key><false/><key>Internal</key><true/>
<key>RemovableMediaOrExternalDevice</key><false/>
<key>ParentWholeDisk</key><string>disk12</string>
</dict></plist>"#;
        assert_eq!(parse_disk_info(hfs), Some(disk("disk12", true, false)));
        assert_eq!(parse_disk_info(b"not a plist"), None);
        assert_eq!(whole_disk("disk10s2s1"), "disk10");
        // The real startup volume parses and is internal.
        let boot = disk_info(Path::new("/")).expect("diskutil info /");
        assert!(boot.internal && boot.physical_disk.starts_with("disk"));
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
        let plan = eject_plan(&mount);
        assert_eq!(plan.mode, EjectMode::Eject);
        let err = eject(&plan).expect_err("busy volume must not eject");
        assert_eq!(err, "it's in use by another app");
        assert!(listed(&mount));
        drop(handle);
        std::fs::remove_file(&busy).unwrap();
        assert_eq!(eject(&plan), Ok(EjectOutcome::Ejected));
        assert!(!listed(&mount));
    }
}
