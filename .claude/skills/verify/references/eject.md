# Runbook: drive eject

Code: `crates/haven-core/src/volumes.rs` (`eject`, `ejectable_volume_for`,
`eject_error_message`), `Workspace::eject` / `eject_current`
(`crates/haven/src/workspace.rs`), the sidebar drive rows
(`workspace_view.rs`) and the Start page drive cards (`views/start.rs`).

## Safety invariant (check it in the diff)

`diskutil eject <path>` resolves any path to the whole disk that holds it.
`volumes::eject` must return early for `/` and for anything that isn't a
listed mount point. A regression here could eject, or try to eject, the
startup disk. `ejecting_an_unmounted_path_is_a_no_op` guards it.

## Test volume (scratch disk image)

```bash
D=$(mktemp -d /tmp/haven-eject.XXXXXX)
hdiutil create -size 20m -fs APFS -volname HavenEjectTest "$D/t.dmg"
hdiutil attach "$D/t.dmg"        # NOT -nobrowse: that hides it (MNT_DONTBROWSE)
mount | grep HavenEjectTest
```

## Headless checks

```bash
cargo test -p haven-core volumes
HAVEN_EJECT_TEST_MOUNT=/Volumes/HavenEjectTest \
  cargo test -p haven-core ejects_a_mounted -- --ignored
```

The ignored test ejects the real image. With a file held open, it expects
the error "it's in use by another app" and the volume to stay mounted.
Without it, the eject succeeds and the volume disappears. It unmounts the
image, so re-attach it before the GUI checks.

`cargo test -p haven eject` covers the view side: tabs on the volume go to
the start page, a second eject while one is running is ignored, and ⌘E with
no volume says "Nothing to eject here".

## In the app

1. Launch with the image attached. Sidebar ▸ Drives shows "HavenEjectTest"
   with an eject icon (always visible, Finder-style) and a "Eject" tooltip.
   The Start page drive card has an eject button too. Macintosh HD has
   neither.
2. Open a tab inside the volume. Click the sidebar eject button: a spinner
   replaces the icon, the toast reads `Ejecting “HavenEjectTest”…` then
   `“HavenEjectTest” can be safely removed`, the tab switches to the start
   page, and the drive leaves the sidebar within about 5s
   (`mount | grep HavenEjectTest` is empty).
3. Re-attach. Right-click the drive row: the menu has "Open in New Tab" and
   "Eject" (with ⌘E). Eject from there.
4. Re-attach. Navigate into the volume and press ⌘E (also File ▸ Eject).
5. Busy case: re-attach, run `sleep 600 < /Volumes/HavenEjectTest/busy.txt &`
   (create the file first), eject from Haven. Expect a red toast
   `Couldn't eject “HavenEjectTest”: it's in use by another app`, with the
   drive still listed. Kill the `sleep` and eject again: it succeeds.
6. Clean up: `hdiutil detach /Volumes/HavenEjectTest 2>/dev/null; rm -rf "$D"`.

Headless alternative when the screen is busy: step 2's outcome is covered
by the ignored test plus the gpui tests. Capture the sidebar
(`screencapture -l`) before and after `hdiutil detach` to check the drive
list refreshes.
