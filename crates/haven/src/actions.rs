use std::path::PathBuf;

use gpui::{Action, App, KeyBinding, actions};

actions!(
    haven,
    [
        Quit,
        NewTab,
        CloseTab,
        ReopenClosedTab,
        NextTab,
        PrevTab,
        FocusPathBar,
        FocusFilter,
        CommandPalette,
        FuzzyFind,
        ToggleHidden,
        TogglePreview,
        ToggleSidebar,
        ViewList,
        ViewGrid,
        ViewColumns,
        GoBack,
        GoForward,
        GoParent,
        GoStart,
        GoHomeFolder,
        Refresh,
        UndoOp,
        SpaceKey,
        CopyPath,
        NewFolder,
        PasteFiles,
        OpenInTerminal,
        OpenInEditor,
        SelectNext,
        SelectPrev,
        SelectNextExtend,
        SelectPrevExtend,
        SelectLeft,
        SelectRight,
        SelectFirst,
        SelectLast,
        PageDown,
        PageUp,
        SelectAllItems,
        ClearSelection,
        OpenSelected,
        OpenInNewTab,
        RevealSelected,
        RenameSelected,
        BulkRename,
        CopyFiles,
        CutFiles,
        TrashSelected,
        DuplicateSelected,
        EditTags,
        PinToSidebar,
        PinToStart,
        CalculateSizes,
        GetInfo,
        ToggleFoldersFirst,
        MakeDefaultViewer,
        RestoreFinder,
        Dismiss,
        ConfirmModal,
        OpenSettings,
        EjectSelected,
    ]
);

/// Navigate the active tab to a path (directory, or a file's parent with it selected).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = haven, no_json)]
pub struct NavigateTo(pub PathBuf);

/// Show everything carrying a tag.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = haven, no_json)]
pub struct ShowTag(pub String);

/// Activate tab N (0-based).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = haven, no_json)]
pub struct ActivateTab(pub usize);

/// Toggle a tag on the current selection.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = haven, no_json)]
pub struct ToggleTag(pub String);

/// Make `key` the primary sort key (flips direction if it already is).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = haven, no_json)]
pub struct SortBy(pub haven_core::SortKey);

/// Add `key` as a secondary sort key.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = haven, no_json)]
pub struct ThenSortBy(pub haven_core::SortKey);

/// Remove a folder from the sidebar pins.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = haven, no_json)]
pub struct UnpinSidebar(pub PathBuf);

/// Eject (or unmount) the volume mounted at this path.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = haven, no_json)]
pub struct EjectVolume(pub PathBuf);

/// Eject confirmed although other volumes on the same disk go too.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = haven, no_json)]
pub struct EjectVolumeConfirmed(pub PathBuf);

/// Confirmed default-viewer change.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = haven, no_json)]
pub struct ConfirmDefaultViewer(pub bool);

/// Open the selection with a specific application bundle.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = haven, no_json)]
pub struct OpenWith(pub PathBuf);

pub const FILE_VIEW: &str = "FileView";
pub const WORKSPACE: &str = "Workspace";

pub fn bind_keys(cx: &mut App) {
    let fv = Some(FILE_VIEW);
    let ws = Some(WORKSPACE);
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-e", EjectSelected, ws),
        KeyBinding::new("escape", Dismiss, ws),
        // Enter takes the primary button of the conflict / confirm dialogs
        // (they hold focus on the workspace, so the file view's Enter is out).
        KeyBinding::new("enter", ConfirmModal, ws),
        KeyBinding::new("cmd-t", NewTab, ws),
        KeyBinding::new("cmd-w", CloseTab, ws),
        KeyBinding::new("cmd-shift-t", ReopenClosedTab, ws),
        KeyBinding::new("ctrl-tab", NextTab, ws),
        KeyBinding::new("ctrl-shift-tab", PrevTab, ws),
        KeyBinding::new("cmd-shift-]", NextTab, ws),
        KeyBinding::new("cmd-shift-[", PrevTab, ws),
        KeyBinding::new("cmd-l", FocusPathBar, ws),
        KeyBinding::new("cmd-g", FocusPathBar, ws),
        KeyBinding::new("cmd-f", FocusFilter, ws),
        KeyBinding::new("cmd-k", CommandPalette, ws),
        KeyBinding::new("cmd-shift-p", CommandPalette, ws),
        KeyBinding::new("cmd-p", FuzzyFind, ws),
        KeyBinding::new("cmd-shift-.", ToggleHidden, ws),
        KeyBinding::new("cmd-alt-p", TogglePreview, ws),
        KeyBinding::new("cmd-alt-s", ToggleSidebar, ws),
        KeyBinding::new("cmd-alt-1", ViewList, ws),
        KeyBinding::new("cmd-alt-2", ViewGrid, ws),
        KeyBinding::new("cmd-alt-3", ViewColumns, ws),
        KeyBinding::new("cmd-[", GoBack, ws),
        KeyBinding::new("cmd-]", GoForward, ws),
        KeyBinding::new("cmd-up", GoParent, ws),
        KeyBinding::new("cmd-shift-h", GoHomeFolder, ws),
        KeyBinding::new("cmd-shift-s", GoStart, ws),
        KeyBinding::new("cmd-r", Refresh, ws),
        KeyBinding::new("cmd-z", UndoOp, ws),
        KeyBinding::new("cmd-alt-c", CopyPath, ws),
        KeyBinding::new("cmd-shift-c", CopyPath, ws),
        KeyBinding::new("cmd-shift-n", NewFolder, ws),
        KeyBinding::new("cmd-v", PasteFiles, fv),
        KeyBinding::new("cmd-alt-t", OpenInTerminal, ws),
        KeyBinding::new("cmd-alt-e", OpenInEditor, ws),
        KeyBinding::new("cmd-1", ActivateTab(0), ws),
        KeyBinding::new("cmd-2", ActivateTab(1), ws),
        KeyBinding::new("cmd-3", ActivateTab(2), ws),
        KeyBinding::new("cmd-4", ActivateTab(3), ws),
        KeyBinding::new("cmd-5", ActivateTab(4), ws),
        KeyBinding::new("cmd-6", ActivateTab(5), ws),
        KeyBinding::new("cmd-7", ActivateTab(6), ws),
        KeyBinding::new("cmd-8", ActivateTab(7), ws),
        KeyBinding::new("cmd-9", ActivateTab(8), ws),
        // File view
        KeyBinding::new("down", SelectNext, fv),
        KeyBinding::new("up", SelectPrev, fv),
        KeyBinding::new("shift-down", SelectNextExtend, fv),
        KeyBinding::new("shift-up", SelectPrevExtend, fv),
        KeyBinding::new("left", SelectLeft, fv),
        KeyBinding::new("right", SelectRight, fv),
        KeyBinding::new("home", SelectFirst, fv),
        KeyBinding::new("end", SelectLast, fv),
        KeyBinding::new("alt-up", SelectFirst, fv),
        KeyBinding::new("alt-down", SelectLast, fv),
        KeyBinding::new("pagedown", PageDown, fv),
        KeyBinding::new("pageup", PageUp, fv),
        KeyBinding::new("cmd-a", SelectAllItems, fv),
        KeyBinding::new("escape", ClearSelection, fv),
        KeyBinding::new("enter", OpenSelected, fv),
        KeyBinding::new("cmd-down", OpenSelected, fv),
        KeyBinding::new("cmd-o", OpenSelected, fv),
        KeyBinding::new("cmd-enter", OpenInNewTab, fv),
        KeyBinding::new("backspace", GoBack, fv),
        KeyBinding::new("f2", RenameSelected, fv),
        KeyBinding::new("shift-enter", RenameSelected, fv),
        KeyBinding::new("cmd-shift-r", BulkRename, fv),
        KeyBinding::new("cmd-c", CopyFiles, fv),
        KeyBinding::new("cmd-x", CutFiles, fv),
        KeyBinding::new("cmd-backspace", TrashSelected, fv),
        KeyBinding::new("cmd-d", DuplicateSelected, fv),
        KeyBinding::new("ctrl-t", EditTags, fv),
        KeyBinding::new("cmd-ctrl-t", PinToSidebar, fv),
        KeyBinding::new("cmd-i", GetInfo, fv),
        // Space continues a type-to-select in progress, else toggles preview.
        KeyBinding::new("space", SpaceKey, fv),
        KeyBinding::new("cmd-alt-r", RevealSelected, fv),
    ]);
}

/// A human-readable shortcut label for the palette.
pub fn shortcut_label(action: &dyn Action, cx: &App) -> Option<String> {
    let _ = cx;
    let name = action.name();
    let label = match name {
        "haven::NewTab" => "⌘T",
        "haven::CloseTab" => "⌘W",
        "haven::ReopenClosedTab" => "⇧⌘T",
        "haven::FocusPathBar" => "⌘L",
        "haven::FocusFilter" => "⌘F",
        "haven::CommandPalette" => "⌘K",
        "haven::FuzzyFind" => "⌘P",
        "haven::ToggleHidden" => "⇧⌘.",
        "haven::TogglePreview" => "Space",
        "haven::ToggleSidebar" => "⌥⌘S",
        "haven::ViewList" => "⌥⌘1",
        "haven::ViewGrid" => "⌥⌘2",
        "haven::ViewColumns" => "⌥⌘3",
        "haven::GoBack" => "⌘[",
        "haven::GoForward" => "⌘]",
        "haven::GoParent" => "⌘↑",
        "haven::GoHomeFolder" => "⇧⌘H",
        "haven::GoStart" => "⇧⌘S",
        "haven::Refresh" => "⌘R",
        "haven::UndoOp" => "⌘Z",
        "haven::CopyPath" => "⇧⌘C",
        "haven::NewFolder" => "⇧⌘N",
        "haven::PasteFiles" => "⌘V",
        "haven::OpenInTerminal" => "⌥⌘T",
        "haven::OpenInEditor" => "⌥⌘E",
        "haven::RenameSelected" => "F2",
        "haven::BulkRename" => "⇧⌘R",
        "haven::CopyFiles" => "⌘C",
        "haven::CutFiles" => "⌘X",
        "haven::TrashSelected" => "⌘⌫",
        "haven::DuplicateSelected" => "⌘D",
        "haven::EditTags" => "⌃T",
        "haven::PinToSidebar" => "⌃⌘T",
        "haven::GetInfo" => "⌘I",
        "haven::OpenSelected" => "↩",
        "haven::OpenInNewTab" => "⌘↩",
        "haven::SelectAllItems" => "⌘A",
        "haven::OpenSettings" => "⌘,",
        "haven::EjectSelected" => "⌘E",
        _ => return None,
    };
    Some(label.to_string())
}
