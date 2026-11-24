#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod keys;
mod messages;
mod theme;
mod view;
mod bulk_rename;
mod inspector;
mod interface;
mod layout;
mod palette;
mod projects;
mod folder_sizes;
mod quick_look;
mod compare;
mod spring;
mod edge_scroll;
mod smooth_scroll;
mod marked;
mod motion;
mod motion_policy;
mod layout_motion;
mod button_motion;
mod disclosure_motion;
mod grid_motion;
mod row_motion;
mod list_motion;
mod dock_motion;
mod tab_reorder;
mod popup_motion;
mod mode_motion;
mod kind_filter;
mod search_panel;
mod updates;
mod icons;
mod feed;
mod labels;
mod jump;
mod archives;
mod version_rules;
mod thumbs;
mod appearance;
mod directories;
mod transfers;
mod transfer_panel;
mod shell_actions;
mod project_card;
mod inbox;
mod inbox_previews;

use filemanager_core::browser;
use filemanager_core::panes::{Workspace, GroupId, Edge};
use filemanager_core::clipboard;
use filemanager_core::history::{Event, Journal};
use filemanager_core::directory_watch::DirectoryWatch;
use filemanager_core::operations::{Action, CopyControl, DropZone, OperationQueue, Plan, Receipt, TransferPhase};
use filemanager_core::places::{self, Place};
use filemanager_core::search;
use filemanager_core::sort::SortSpec;
use filemanager_core::operation_journal::{InterruptedAction, OperationJournal};
use filemanager_core::workspace::WorkspaceStore;
use gpui::{
    div, prelude::*, px, size, App, Bounds, Context, Entity, FocusHandle, Focusable,
    IntoElement, Pixels, Point, Render, Subscription, TitlebarOptions, UniformListScrollHandle,
    Window, WindowBounds, WindowOptions,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::theme::{Theme, ThemeMode};
use gpui_component::Root;
use theme::rgb;
use std::path::PathBuf;
use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use transfers::Transfer;


struct ColumnResize {
    group: GroupId,
    start_x: Pixels,
    start_width: f32,
}

/// Internal drag-and-drop payload. Dropping only stages a path; no file
/// operation takes place until the user explicitly clicks Copy here.
#[derive(Clone)]
struct FileDragInfo { paths: Vec<PathBuf> }

struct FileDragPreview { name: String, position: Point<Pixels>, mode: Option<DropMode>, picked: std::time::Instant }

impl FileDragPreview {
    fn new(name: String, position: Point<Pixels>, mode: Option<DropMode>) -> Self {
        Self { name, position, mode, picked: std::time::Instant::now() }
    }
}

/// The folder files would land in if released now, shared with the drag's
/// picture so it can say «Копия» or «Перенос».
/// What releasing dragged files over the thing under them does.
#[derive(Clone, Debug, Default, PartialEq)]
enum DropAt {
    /// Nothing takes files there (a tab, the path bar: they only open).
    #[default]
    Nothing,
    /// Copied or moved into this folder (a folder row or tile, a list).
    Folder(PathBuf),
    /// Put aside in «Файлы для переноса» (a place in the sidebar, the zone).
    Stage,
}

#[derive(Default)]
struct DragTarget {
    at: DropAt,
    /// A folder row or place claimed this move; the list's own folder
    /// stands in otherwise.
    claimed: bool,
    /// Something under the pointer named a folder this move.
    touched: bool,
}

type DragTargetCell = std::rc::Rc<std::cell::RefCell<DragTarget>>;

/// What the picture of dragged files needs to name the action.
struct DropMode {
    first: PathBuf,
    /// The files come out of an archive (looked up once, when the drag starts).
    from_archive: bool,
    target: DragTargetCell,
}

impl DropMode {
    fn new(first: &std::path::Path, target: &DragTargetCell) -> Self {
        Self { first: first.to_path_buf(), from_archive: filemanager_core::archive::is_inside(first), target: target.clone() }
    }
}

#[cfg(test)]
mod drop_word_tests {
    use super::drop_word;
    use std::path::Path;

    #[test]
    fn says_what_a_release_does() {
        let file = Path::new("/w/a/report.docx");
        assert_eq!(drop_word(file, false, Path::new("/w/a"), false, false), None);
        assert_eq!(drop_word(file, false, Path::new("/w/a"), true, false), Some("Копия"));
        assert_eq!(drop_word(file, false, Path::new("/w/b"), true, false), Some("Копия"));
        assert_eq!(drop_word(file, false, Path::new("/w/b"), false, true), Some("Перенос"));
        assert_eq!(drop_word(file, true, Path::new("/w/b"), false, false), Some("Извлечь"));
        assert_eq!(drop_word(Path::new("/w/a"), false, Path::new("/w/a"), true, false), None);
        // Into a folder inside an archive: refused, no word.
        let dir = std::env::temp_dir().join(format!("papka-drop-word-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pack.zip"), b"").unwrap();
        assert_eq!(drop_word(file, false, &dir.join("pack.zip").join("inner"), true, false), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The word for a drop of files starting with `first` into `folder`, as
/// `drop_files` decides it: Ctrl copies, Shift moves, otherwise a move on
/// the same drive and a copy across. Out of an archive: extracted. Into the
/// folder they are in, or into an archive: nothing.
fn drop_word(first: &std::path::Path, from_archive: bool, folder: &std::path::Path, control: bool, shift: bool) -> Option<&'static str> {
    use filemanager_core::path_utils::same_path;
    // Staged items are spelled `\\?\C:\…`, the panes' folders are not.
    if same_path(first, folder) || (!control && first.parent().is_some_and(|parent| same_path(parent, folder))) { return None; }
    // A folder inside an archive takes nothing.
    if filemanager_core::archive::is_inside(folder) { return None; }
    if from_archive { return Some("Извлечь"); }
    let copy = if control { true } else if shift { false } else { !filemanager_core::path_utils::same_volume(first, folder) };
    Some(if copy { "Копия" } else { "Перенос" })
}

/// Dragging a tab to reorder it.
#[derive(Clone)]
struct TabDrag { group: GroupId, index: usize, title: String }

impl Render for FileDragPreview {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let modifiers = window.modifiers();
        // Picked up: the label grows out from under the cursor, moves away
        // from it and casts a shadow, as if lifted. A rise alone was lost:
        // the label follows the cursor anyway. Counted from the moment it was
        // taken (GPUI draws a drag's picture afresh on every move, so an
        // element animation would start over each time).
        let lift = if theme::animations() {
            let t = (self.picked.elapsed().as_secs_f32() / 0.25).min(1.);
            if t < 1. { window.request_animation_frame(); }
            motion_policy::ease_out_fraction(t)
        } else { 1. };
        let word = self.mode.as_ref().and_then(|mode| match &mode.target.borrow().at {
            DropAt::Nothing => None,
            DropAt::Stage => Some("Отложить"),
            DropAt::Folder(folder) => drop_word(&mode.first, mode.from_archive, folder, modifiers.control, modifiers.shift),
        });
        div()
            // Off the cursor: from 4 px to 14 px as it is lifted.
            .pl(self.position.x + px(4. + 10. * lift))
            .pt(self.position.y + px(2. + 8. * lift))
            .child(
                div().rounded(px(4.)).bg(rgb(theme::SELECTED))
                    .border_1().border_color(rgb(theme::SELECTED_BORDER))
                    .text_color(rgb(theme::SELECTION_TEXT))
                    // From a small tag to its full size.
                    .px(px(4. + 8. * lift)).py(px(1. + 3. * lift)).text_size(px(11. + 2. * lift))
                    .flex().items_center().gap_2()
                    .opacity(0.5 + 0.5 * lift)
                    .shadow(vec![gpui::BoxShadow {
                        color: gpui::hsla(0., 0., 0., 0.55 * lift),
                        offset: gpui::point(px(0.), px(2. + 6. * lift)),
                        blur_radius: px(4. + 14. * lift),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .child(self.name.clone())
                    // What a release here does; Ctrl and Shift change it at once.
                    .when_some(word, |chip, word| chip.child(div().px_1p5().rounded_sm().text_size(px(11.))
                        .bg(rgb(theme::SELECTED_BORDER)).child(word)))
            )
    }
}


#[derive(Clone, Copy)]
enum ConflictChoice { Skip, KeepBoth, Replace, Cancel }

struct Undo {
    receipts: Vec<Receipt>,
    /// Shown as "Отменить: …".
    label: String,
    unavailable: bool,
}

const UNDO_LIMIT: usize = 50;

/// Files whose preview comes from the Windows Shell thumbnail: documents,
/// video and very large images (decoding those directly costs memory).
pub(crate) fn wants_shell_thumbnail(path: &std::path::Path) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "rtf"
        | "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "m4v" | "mpg" | "mpeg"
        | "heic" | "heif" | "psd" | "svg" | "dwg" | "dxf"
        | "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "ico" => true,
        _ => false,
    }
}

/// A path from an operation receipt as the listings know it (no `\\?\`).
/// Now, in milliseconds since 1970.
fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn normalized(path: Option<PathBuf>) -> Option<PathBuf> {
    path.map(|path| filemanager_core::path_utils::normalize_extended_path(&path))
}

/// Marks that the feature tour was shown once at first start.
const TOUR_SEEN: &str = "tour_seen";
const LIST_WIDTHS: &str = "list_widths";
const COLUMN_WIDTH: &str = "column_width";
const STACK_VERSIONS: &str = "stack_versions";
/// A star before each folder adds it to favourites, and shows that it is one.
const FAVORITE_STARS: &str = "favorite_stars";

#[cfg(windows)]
fn window_hwnd(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    HasWindowHandle::window_handle(window).ok().and_then(|handle| match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    })
}

/// Internal name of the session restored at startup.


/// How «Копировать путь» writes each item.
#[derive(Clone, Copy)]
pub(crate) enum CopyAs { Path, Network, Name }

/// What a right-click was aimed at.
#[derive(Clone)]
enum MenuTarget {
    Entry(PathBuf),
    /// Right-button drop, as in Explorer: copy or move these into the folder.
    Drop(Vec<PathBuf>, PathBuf),
    Folder(PathBuf),
    Tab(GroupId, usize),
    TabList(GroupId),
    Favorite(usize),
    /// A row of «Места», or (None) the section title.
    Place(Option<usize>),
    Recent(PathBuf),
    /// The status bar's «Тип»: the kinds of files in the active pane.
    Kinds,
    /// A › of the path bar: the subfolders of the folder before it, the
    /// next part of the path (if any) marked, to jump to one in the pane.
    Children(GroupId, PathBuf, Option<PathBuf>),
}

struct RightDrag {
    paths: Vec<PathBuf>,
    start: Point<Pixels>,
    position: Point<Pixels>,
    /// Moved far enough to be a drag rather than a click.
    active: bool,
    /// Folder under the cursor; rows claim it before their column does.
    target: Option<PathBuf>,
    claimed: bool,
    /// Opened on release when the button was not dragged.
    click_menu: MenuTarget,
}

#[derive(Clone)]
struct ContextMenu {
    position: Point<Pixels>,
    target: MenuTarget,
    /// Distinct id per opening, so the appear animation replays.
    serial: u64,
    /// Explorer's own commands for the files, read after the menu opens.
    shell: shell_actions::ShellActions,
}

/// Display order of one folder: the listing it was built from (by
/// pointer), and the visible entry indices after sorting and filtering.
struct FolderView {
    listing: Arc<browser::Listing>,
    spec: SortSpec,
    /// Folder sizes known when sorted; a new size re-sorts a size order.
    sizes: u64,
    show_hidden: bool,
    filter: String,
    /// Version stacks as they were when sorted (see `stacks_rev`).
    stacks_rev: u64,
    /// The label shown alone, and the labels as they were when filtered.
    label_filter: Option<filemanager_core::labels::Label>,
    labels_rev: u64,
    /// The kind of file shown alone.
    kind_filter: Option<filemanager_core::kinds::Kind>,
    indices: Arc<Vec<usize>>,
    /// Rows that head or belong to a stack of versions.
    stacks: Arc<HashMap<PathBuf, filemanager_core::versions::Stacked>>,
}

struct Explorer {
    interface_focus_pending: bool,
    marquee: Option<interface::Marquee>,
    palette: Option<interface::Palette>,
    settings: Option<interface::Settings>,
    browser: Workspace,
    selected: Option<PathBuf>,
    // Inspector data is loaded off the UI thread, never during Render.
    inspector: inspector::InspectorState,
    document_preview: Option<filemanager_core::preview_host::PreviewProcess>,
    preview_launch_busy: bool,
    preview_launch_generation: u64,
    zone: DropZone,
    /// The running copy or move, the one waiting for an answer, and those queued behind.
    transfers: transfers::Transfers,
    context_menu: Option<ContextMenu>,
    /// The project card shown in a pane in place of its folders.
    cards: project_card::ProjectCards,
    /// Projects as published for other programs (`projects.json`).
    registry: inbox::Registry,
    /// The window's handle, for menus and dialogs Windows itself shows.
    #[cfg_attr(not(windows), allow(dead_code))]
    hwnd: Option<isize>,
    /// Pressed right button on files: a plain click opens their menu, a
    /// drag ends in the «Копировать сюда / Переместить сюда» menu.
    right_drag: Option<RightDrag>,
    /// The active gpui drag carries files (not a tab).
    file_drag: bool,
    /// What that drag carries: a folder never springs open into itself.
    drag_paths: Vec<PathBuf>,
    spring: Option<spring::Spring>,
    spring_generation: u64,
    /// The folder dragged files are over, for the word on their picture.
    drag_target: DragTargetCell,
    edge_scroll: edge_scroll::EdgeScroll,
    smooth: smooth_scroll::SmoothScroll,
    motion: motion::Motion,
    /// A system drag to another program is running.
    #[cfg_attr(not(windows), allow(dead_code))]
    dragging_out: bool,
    /// The «О программе» panel under its titlebar button.
    about_open: bool,
    updates: updates::Updates,
    icons: icons::Icons,
    feed: feed::Feed,
    labels: labels::LabelState,
    /// Step of the feature tour, while it is shown.
    tour: Option<usize>,
    menu_serial: u64,
    focus_handle: FocusHandle,
    /// «Места» as shown: the user's order, without hidden ones.
    places: Vec<Place>,
    /// Every standard place, for restoring hidden ones.
    all_places: Vec<Place>,
    /// Folders used recently, most recent first.
    recent: Vec<PathBuf>,
    /// How often and how lately folders were stayed in, for Ctrl+P.
    stays: jump::Stays,
    /// Panes showing only files of one kind.
    kinds: kind_filter::KindFilters,
    /// Sidebar sections folded by the user.
    collapsed: HashSet<String>,
    sizes: folder_sizes::FolderSizes,
    quick_look: Option<quick_look::QuickLook>,
    comparison: Option<compare::Comparison>,
    drives: Vec<Place>,
    /// Free and total bytes per local drive, filled in the background.
    drive_space: HashMap<PathBuf, (u64, u64)>,
    /// Sort order of each pane, chosen by its list header.
    sorts: HashMap<GroupId, SortSpec>,
    show_hidden: bool,
    show_sidebar: bool,
    show_inspector: bool,
    folder_views: HashMap<(GroupId, PathBuf), FolderView>,
    scroll_handles: HashMap<(GroupId, PathBuf), UniformListScrollHandle>,
    address_editing: Option<GroupId>,
    /// The name filter field is shown in the bottom bar (Ctrl+F or the magnifier).
    filter_open: bool,
    /// The bottom field searches subfolders (Ctrl+F) rather than filtering.
    search_mode: bool,
    /// Width of one file pane at the last layout, for column dropping.
    pane_width: f32,
    pane_measured_widths: HashMap<GroupId, f32>,
    column_scroll_handles: HashMap<GroupId, gpui::ScrollHandle>,
    /// Sideways scroll of each pane's details table.
    list_scroll_handles: HashMap<GroupId, gpui::ScrollHandle>,
    column_reveal_keys: HashMap<GroupId, String>,
    /// Shared width of Miller columns, changed by dragging any column edge.
    column_width: f32,
    column_resize: Option<ColumnResize>,
    /// Each pane's own column width; `column_width` (the last set) starts new ones.
    pane_column_widths: HashMap<GroupId, f32>,
    /// Widths of the list columns Изменён, Тип, Размер; the name takes the rest.
    list_widths: [f32; 4],
    /// Dragging a list column border: column, start x, start width.
    /// Each pane's own list columns; `list_widths` (the last set) starts new ones.
    pane_list_widths: HashMap<GroupId, [f32; 4]>,
    /// The item the last single click hit (see `is_double_click`).
    last_click: Option<PathBuf>,
    /// Version stacks opened by the user: (folder, family key).
    open_stacks: HashSet<(PathBuf, String)>,
    /// Bumped when stacks open or close, or grouping is switched.
    stacks_rev: u64,
    /// Versions of a document shown as one row (setting).
    stack_versions: bool,
    favorite_stars: bool,
    list_resize: Option<(GroupId, usize, Pixels, f32)>,
    tab_scroll: HashMap<GroupId, gpui::ScrollHandle>,
    tab_reveal: HashMap<GroupId, (usize, usize)>,
    dock_hover: Option<(GroupId, layout::DockZone)>,
    pane_bounds: HashMap<GroupId, Bounds<Pixels>>,
    /// After entering a folder from the keyboard, select its first entry
    /// once the listing arrives.
    select_first_when_loaded: bool,
    rename_input: Entity<InputState>,
    bulk_template_input: Entity<InputState>,
    bulk_rename: Option<bulk_rename::BulkRenameDialog>,
    bulk_rename_errors: Arc<Vec<String>>,
    bulk_rename_generation: u64,
    folder_input: Entity<InputState>,
    creating_folder: bool,
    /// The new-item dialog makes a file rather than a folder.
    creating_file: bool,
    /// «Шаблоны версий» while open; the saved rules below.
    version_editing: Option<version_rules::Editing>,
    version_input: Entity<InputState>,
    version_templates: String,
    version_builtin: bool,
    thumbs: thumbs::Thumbs,
    /// The theme follows Windows; what Windows uses now; the user's own choice.
    system_theme: bool,
    system_light: bool,
    light_preference: bool,
    /// Tiles per row of each pane in the thumbnails view, as last drawn.
    thumb_columns: HashMap<GroupId, usize>,
    renaming: bool,
    undo_stack: Vec<Undo>,
    dirs: directories::DirectoryCache<browser::Listing>,
    workspaces: Option<WorkspaceStore>,
    favorites: Vec<PathBuf>,
    saved_layout: String,
    versions_expanded: bool,
    project_error: Option<String>,
    close_error: Option<String>,
    session_restore_error: Option<String>,
    journal: Option<Arc<Journal>>,
    operation_journal: Option<Arc<OperationJournal>>,
    operation_review: bool,
    operation_review_epoch: u64,
    operation_alerts: Vec<InterruptedAction>,
    watcher: Option<DirectoryWatch>,
    live_watch_started: bool,
    live_watch_error: Option<String>,
    watched_root: Option<PathBuf>,
    /// Set by «Остановить»: history stays off until switched on again.
    history_paused: bool,
    /// Visible folders the OS would not watch (no access, offline share).
    unwatched_folders: std::collections::HashSet<PathBuf>,
    /// Validated Recycle plans awaiting confirmation. Confirming submits
    /// exactly these plans, never re-prepared ones.
    confirm_recycle: Option<Vec<(PathBuf, Plan)>>,
    /// Extra selected items (Ctrl/Shift); empty means only `selected`.
    marked: marked::Marked,
    /// Fixed end of a Shift range.
    anchor: Option<PathBuf>,
    /// Lower-case name filter for the focused folder ("" = none).
    filter: String,
    /// Set when navigation should reset the filter at the next render.
    clear_filter: bool,
    workspace_names: Vec<String>,
    workspace_input: Entity<InputState>,
    saving_workspace: bool,
    status: String,
    search_input: Entity<InputState>,
    address_input: Entity<InputState>,
    comment_input: Entity<InputState>,
    author_input: Entity<InputState>,
    search: search_panel::SearchState,
    selected_history_event: Option<i64>,
    _subscriptions: Vec<Subscription>,
}

impl Explorer {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Фильтр · Shift+Enter — везде"));
        let address_input = cx.new(|cx| InputState::new(window, cx).placeholder("Путь к папке"));
        let comment_input = cx.new(|cx| InputState::new(window, cx).placeholder("Что изменено…"));
        let author_input = cx.new(|cx| InputState::new(window, cx).placeholder("Кто изменил (необязательно)…"));
        let comment_subscription = cx.subscribe_in(&comment_input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) { this.save_feed_comment(cx); }
        });
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("Новое имя"));
        let bulk_template_input = cx.new(|cx| InputState::new(window, cx).placeholder("{stem} ({n}){ext}"));
        let bulk_subscription = cx.subscribe_in(&bulk_template_input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) { this.schedule_bulk_preview(cx); }
        });
        let folder_input = cx.new(|cx| InputState::new(window, cx).placeholder("Имя папки"));
        let version_input = cx.new(|cx| InputState::new(window, cx).placeholder("_изм{N}; _rev{A}; _{дата}"));
        let version_subscription = cx.subscribe_in(&version_input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) { this.save_version_rules(window, cx); }
        });
        let workspace_input = cx.new(|cx| InputState::new(window, cx).placeholder("Например: Проект «Мост»"));
        let workspace_subscription = cx.subscribe_in(&workspace_input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save_named_workspace(cx);
                let handle = this.focus_handle.clone();
                window.focus(&handle, cx);
            }
        });
        let address_subscription = cx.subscribe_in(&address_input, window, |this, _, event: &InputEvent, window, cx| {
            match event {
                InputEvent::PressEnter { .. } => {
                    this.open_address(cx);
                    this.address_editing = None;
                    let handle = this.focus_handle.clone();
                    window.focus(&handle, cx);
                }
                InputEvent::Blur => {
                    this.address_editing = None;
                    cx.notify();
                }
                _ => {}
            }
        });
        let rename_subscription = cx.subscribe_in(&rename_input, window, |this, _, event: &InputEvent, window, cx| {
            match event {
                InputEvent::PressEnter { .. } => {
                    this.commit_rename(cx);
                    let handle = this.focus_handle.clone();
                    window.focus(&handle, cx);
                }
                // Clicking elsewhere ends in-place renaming without changes.
                InputEvent::Blur if this.renaming && !this.transfers.busy => {
                    this.renaming = false;
                    cx.notify();
                }
                _ => {}
            }
        });
        let folder_subscription = cx.subscribe_in(&folder_input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.create_folder(cx);
                let handle = this.focus_handle.clone();
                    window.focus(&handle, cx);
            }
        });
        // Typing filters the current folder instantly; Enter searches the
        // indexed subfolders as well.
        let search_subscription = cx.subscribe_in(&search_input, window, |this, input, event: &InputEvent, _, cx| {
            match event {
                InputEvent::Change => {
                    this.search.query = input.read(cx).value().to_string();
                    // Ctrl+F's search leaves the folder behind unfiltered.
                    this.filter = if this.search_mode { String::new() } else { this.search.query.trim().to_lowercase() };
                    if this.search.query.trim().is_empty() && this.search.active {
                        this.close_search();
                    } else if this.search.active {
                        this.schedule_search(cx);
                    } else if !this.search_mode {
                        this.select_first_match(cx);
                    }
                    cx.notify();
                }
                _ => {}
            }
        });
        let operation_journal = OperationJournal::open(OperationJournal::default_path())
            .ok().map(Arc::new);
        let operation_status = match operation_journal.as_ref() {
            Some(journal) => match journal.unresolved(100) {
                Ok(pending) if !pending.is_empty() => format!(
                    "Внимание: незавершённых операций с файлами: {}. Проверьте затронутые пути, повтор не выполнялся.",
                    pending.len()
                ),
                // Nothing to report: an idle status bar stays quiet.
                Ok(_) => String::new(),
                Err(error) => format!("Диагностика операций недоступна: {error}"),
            },
            None => "Журнал операций недоступен: изменение файлов отключено".into(),
        };
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let (workspaces,prefs,prefs_error)=WorkspaceStore::open_with_preferences(WorkspaceStore::default_path());
        let prefs_error=prefs_error.map(|error|format!("Настройки не прочитаны: {error}"));
        let restored = workspaces.as_ref().map(WorkspaceStore::load_session).transpose();
        let browser = restored.as_ref().ok().and_then(|r|r.clone().flatten()).unwrap_or_else(||
            Workspace::new(home).or_else(|_| Workspace::new(".")).expect("No starting directory"));
        let session_restore_error = restored.err().map(|e|e.to_string());
        let prefs_error = session_restore_error.as_ref().map(|e|format!("Проект не восстановлен: {e}")).or(prefs_error);
        let favorites = workspaces.as_ref().and_then(|s|s.favorites().ok()).unwrap_or_default();
        let stack_versions = workspaces.as_ref().and_then(|store| store.value(STACK_VERSIONS).ok().flatten())
            .is_none_or(|flag| flag != "0");
        let favorite_stars = workspaces.as_ref().and_then(|store| store.value(FAVORITE_STARS).ok().flatten())
            .is_none_or(|flag| flag != "0");
        let system_icons = workspaces.as_ref().and_then(|store| store.value(icons::ENABLED_KEY).ok().flatten())
            .is_none_or(|flag| flag != "0");
        let search_depth = workspaces.as_ref().and_then(|store| store.value(search_panel::DEPTH_KEY).ok().flatten())
            .and_then(|text| text.parse().ok()).filter(|depth| *depth < search_panel::DEPTHS.len()).unwrap_or(0);
        theme::set_light(prefs.light);
        let accent = |key| workspaces.as_ref().and_then(|store| store.value(key).ok().flatten())
            .map(|value| theme::accent_scheme(&value)).unwrap_or(0);
        theme::set_accents(accent(theme::ACCENT_DARK_KEY), accent(theme::ACCENT_LIGHT_KEY));
        Theme::change(if prefs.light { ThemeMode::Light } else { ThemeMode::Dark },Some(window),cx);
        view::apply_component_theme(cx);
        Self {
            interface_focus_pending:false,marquee:None,palette:None,settings:None,
            browser,
            selected: None,
            inspector: Default::default(),
            document_preview: None,
            preview_launch_busy: false,
            preview_launch_generation: 0,
            zone: DropZone::default(),
            transfers: Default::default(),
            context_menu: None,
            cards: project_card::ProjectCards::new(window, cx),
            registry: inbox::Registry { due: true, ..Default::default() },
            hwnd: None,
            right_drag: None, file_drag: false, drag_paths: Vec::new(), spring: None, spring_generation: 0, drag_target: Default::default(), edge_scroll: Default::default(), smooth: Default::default(), motion: Default::default(), dragging_out: false, about_open: false, updates: updates::Updates::load(workspaces.as_ref()), icons: icons::Icons::new(system_icons), feed: feed::Feed::new(), labels: labels::LabelState::open(),
            menu_serial: 0,
            focus_handle,
            places: projects::arrange_places(&places::user_places(),
                workspaces.as_ref().and_then(|store| store.value(projects::PLACES).ok().flatten()).as_deref()),
            all_places: places::user_places(),
            sizes: Default::default(),
            collapsed: workspaces.as_ref().and_then(|store| store.value(projects::COLLAPSED).ok().flatten())
                .map(|text| text.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()).unwrap_or_default(),
            quick_look: None,
            comparison: None,
            recent: workspaces.as_ref().and_then(|store| store.value(projects::RECENT).ok().flatten())
                .map(|text| text.lines().map(PathBuf::from).collect()).unwrap_or_default(),
            stays: jump::Stays::new(workspaces.as_ref().and_then(|store| store.value(projects::VISITS).ok().flatten())
                .map(|text| filemanager_core::jump::Visits::parse(&text)).unwrap_or_default()),
            kinds: Default::default(),
            drives: places::drives(),
            drive_space: HashMap::new(),
            sorts: HashMap::new(),
            show_hidden: prefs.show_hidden,
            show_sidebar: prefs.show_sidebar,
            show_inspector: prefs.show_inspector,
            folder_views: HashMap::new(),
            scroll_handles: HashMap::new(),
            address_editing: None,
            filter_open: false,
            search_mode: false,
            marked: Default::default(),
            anchor: None,
            filter: String::new(),
            clear_filter: false,
            workspace_names: Vec::new(),
            workspace_input,
            saving_workspace: false,
            pane_width: 800.,
            pane_measured_widths: HashMap::new(),
            column_scroll_handles: Default::default(),
            list_scroll_handles: Default::default(),
            column_reveal_keys: HashMap::new(),
            column_width: workspaces.as_ref().and_then(|store| store.value(COLUMN_WIDTH).ok().flatten())
                .and_then(|text| text.parse::<f32>().ok()).filter(|width| width.is_finite())
                .map_or(crate::layout::COLUMN_DEFAULT, |width| width.clamp(crate::layout::COLUMN_MIN, crate::layout::COLUMN_MAX)),
            column_resize: None,
            pane_column_widths: HashMap::new(),
            list_widths: workspaces.as_ref().and_then(|store| store.value(LIST_WIDTHS).ok().flatten())
                .and_then(|text| layout::parse_list_widths(&text)).unwrap_or(layout::LIST_DEFAULT),
            pane_list_widths: HashMap::new(),
            last_click: None,
            open_stacks: HashSet::new(), stacks_rev: 0, stack_versions, favorite_stars,
            list_resize: None,
            tab_scroll: HashMap::new(),
            tab_reveal: HashMap::new(),
            dock_hover: None,
            pane_bounds: HashMap::new(),
            select_first_when_loaded: false,
            rename_input, folder_input,
            bulk_template_input, bulk_rename: None, bulk_rename_errors: Arc::default(), bulk_rename_generation: 0,
            renaming: false,
            creating_folder: false,
            creating_file: false,
            version_editing: None,
            version_input,
            version_templates: String::new(),
            version_builtin: true,
            thumbs: Default::default(),
            system_theme: false,
            system_light: false,
            light_preference: false,
            thumb_columns: HashMap::new(),
            undo_stack: Vec::new(),
            dirs: Default::default(),
            tour: workspaces.as_ref().and_then(|store| store.flag(TOUR_SEEN).ok()).is_some_and(|seen| !seen).then_some(0),
            workspaces, favorites, saved_layout: String::new(), versions_expanded: false, project_error: None, close_error: None, session_restore_error,
            journal: None,
            operation_journal,
            operation_review: false,
            operation_review_epoch: 0,
            operation_alerts: Vec::new(),
            watcher: None, live_watch_started: false, live_watch_error: None,
            watched_root: None, history_paused: false, unwatched_folders: Default::default(), confirm_recycle: None,
            status: prefs_error.unwrap_or_else(|| {
                // The last run ended on an error: said once, with where the log is.
                match filemanager_core::guard::take_pending(&filemanager_core::path_utils::app_data_dir()) {
                    Some(crash) => format!("Прошлый сеанс закрылся из-за ошибки ({}). Подробности: crash.log в папке профиля",
                        crash.split(" · ").nth(2).unwrap_or("без описания")),
                    None => operation_status,
                }
            }),
            search_input, address_input, comment_input, author_input,
            search: search_panel::SearchState {
                depth: search_depth,
                ..Default::default()
            },
            selected_history_event: None,
            _subscriptions: vec![
                // Alt+Tab while dragging files: Explorer's drag goes on into
                // the program brought forward, so ours does too.
                cx.observe_window_activation(window, |this, window, cx| {
                    if !window.is_window_active() { this.drag_out_on_deactivate(window, cx); }
                }),
                search_subscription, address_subscription, rename_subscription, folder_subscription, version_subscription,
                workspace_subscription,
                bulk_subscription,
                comment_subscription,
            ],
        }
    }

    /// One bounded preview and at most eight history rows per selection.
    /// A request number prevents a slow response for a previously selected
    /// file from overwriting the active inspector.
    fn load_selected_details(&mut self, cx: &mut Context<Self>) {
        if self.inspector.path == self.selected && (!self.inspector.dirty || self.inspector.loading) {
            return;
        }
        self.inspector.dirty = false;
        self.inspector.request = self.inspector.request.wrapping_add(1);
        let request = self.inspector.request;
        self.inspector.path = self.selected.clone();
        self.inspector.preview = None;
        self.inspector.thumbnail = None;
        self.inspector.history.clear();
        self.inspector.versions.clear();
        let Some(path) = self.selected.clone() else {
            self.inspector.loading = false;
            return;
        };
        self.inspector.loading = true;
        if self.inspector.details_busy { return; }
        self.inspector.details_busy = true;
        let history_journal = self.journal.clone();
        let observe_history = self.watched_root.is_some();
        let selected_path = path.clone();
        let task = cx.background_spawn(async move {
            // Inside an archive there is nothing on disk to read or record.
            if filemanager_core::archive::is_inside(&path) {
                return (Some(("archive".to_owned(), String::new())), Ok(Vec::new()), Vec::new());
            }
            // Expensive network paths and a busy SQLite database may take
            // seconds. Neither can hold up GPUI painting or scrolling.
            let preview = match search::preview(&path, 1024) {
                Ok(result) => Some((result.kind.to_owned(), result.description)),
                Err(error) => Some(("Файл недоступен".into(), format!("{}: {error}", path.display()))),
            };
            let history = match history_journal.as_ref() {
                Some(journal) => {
                    // The feed shows the history now; selecting a file still
                    // records its state, so the next change of it is noticed.
                    if observe_history { journal.observe(&path).map(|_| Vec::new()) } else { Ok(Vec::new()) }
                }
                None => Ok(Vec::new()),
            };
            let versions = filemanager_core::versions::family(&path).unwrap_or_default();
            (preview, history, versions)
        });
        cx.spawn(async move |weak, cx| {
            let (preview, history, versions) = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.inspector.details_busy = false;
                if this.inspector.request != request || this.selected.as_ref() != Some(&selected_path) {
                    this.inspector.loading = false;
                    this.inspector.dirty = true;
                    this.load_selected_details(cx);
                    return;
                }
                this.inspector.preview = preview;
                this.inspector.versions = versions;
                this.load_selected_thumbnail(cx);
                match history {
                    Ok(history) => this.inspector.history = history,
                    // One unreadable file is not a lasting fault of the journal.
                    Err(error) => this.status = format!("История этого файла недоступна: {error}"),
                }
                this.inspector.loading = false;
                cx.notify();
            });
        }).detach();
    }

    fn close_search(&mut self) {
        self.stop_search();
        self.search.generation = self.search.generation.wrapping_add(1);
        self.search.active = false;
        self.search.busy = false;
        self.search.root = None;
        self.search.hits.clear();
        self.search.selected = None;
    }

    fn add_tab(&mut self, cx: &mut Context<Self>) {
        // The new tab is shown: a project card over the pane gives way.
        self.leave_project_card(self.browser.active, cx);
        let path = self.browser.active().active().path.clone();
        match self.browser.new_tab(self.browser.active, path) {
            Ok(_) => self.motion.tab_opened(self.browser.active, self.browser.active_group().active_tab),
            Err(e) => self.status = e.to_string(),
        }
        self.selected = None; self.marked.clear(); self.anchor = None;
        self.close_search();
        cx.notify();
    }

    /// A flash drive or card plugged in (or pulled out) after start shows
    /// in «Диски» within a couple of seconds.
    fn start_drive_watch(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, cx| loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let drives = cx.background_spawn(async { places::drives() }).await;
            let alive = weak.update(cx, |this, cx| {
                let same = this.drives.len() == drives.len()
                    && this.drives.iter().zip(&drives).all(|(a, b)| a.path == b.path && a.kind == b.kind);
                if !same {
                    this.drives = drives;
                    this.load_drive_space(cx);
                    cx.notify();
                }
            }).is_ok();
            if !alive { break; }
        }).detach();
    }

    /// Reads free space of local drives off the UI thread. Network drives
    /// are skipped: a sleeping server must not stall anything.
    fn load_drive_space(&mut self, cx: &mut Context<Self>) {
        let roots: Vec<PathBuf> = self.drives.iter()
            .filter(|drive| drive.kind != places::PlaceKind::NetworkDrive)
            .map(|drive| drive.path.clone())
            .collect();
        let task = cx.background_spawn(async move {
            roots.into_iter()
                .filter_map(|root| places::disk_space(&root).map(|space| (root, space)))
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |weak, cx| {
            let spaces = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.drive_space = spaces.into_iter().collect();
                cx.notify();
            });
        }).detach();
    }

    fn stage_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if paths.iter().any(|path| !path.exists()) {
            self.status = "Файлы из архива сюда не добавить: перетащите их прямо в папку — архиватор распакует их туда".into();
            cx.notify();
            return;
        }
        let mut error = None;
        for path in paths {
            if self.zone.items().contains(path) { continue; }
            if let Err(e) = self.zone.add(path) {
                error.get_or_insert(format!("{}: {e}", browser::display_name(path)));
            }
        }
        self.status = match error {
            Some(error) => format!("Не всё добавлено для переноса: {error}"),
            None => format!("Для переноса: {}", self.zone.items().len()),
        };
        cx.notify();
    }

    fn open_in_new_tab(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        self.leave_project_card(self.browser.active, cx);
        match self.browser.new_tab(self.browser.active, path) {
            Ok(_) => self.motion.tab_opened(self.browser.active, self.browser.active_group().active_tab),
            Err(error) => self.status = format!("Не удалось открыть вкладку: {error}"),
        }
        self.selected = None;
        self.close_search();
        cx.notify();
    }

    fn open_in_other_pane(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let active = self.browser.active;
        let other = match self.browser.other_group(active) {
            Some(id) => id,
            None => match self.browser.split(active, if self.pane_width<480. {Edge::Bottom} else {Edge::Right}, None) {
                Ok(id) => id, Err(error) => { self.status=error.to_string(); cx.notify(); return; }
            },
        };
        self.navigate_side(other, path, None, cx);
    }

    fn close_other_tabs(&mut self, keep: usize, cx: &mut Context<Self>) {
        self.browser.close_other_tabs(self.browser.active, keep);
        self.selected=None; self.marked.clear(); self.context_menu=None; self.close_search(); cx.notify();
    }

    /// Ctrl+C / Ctrl+X: the selection goes to the Windows clipboard, so it
    /// can be pasted here or in Explorer. Nothing on disk changes yet.
    fn clipboard_put(&mut self, cut: bool, cx: &mut Context<Self>) {
        let paths = self.selection();
        if paths.is_empty() {
            return;
        }
        if Self::any_in_archive(&paths) {
            // Nothing is cut out of an archive: it is copied.
            self.clipboard_from_archive(paths, cx);
            return;
        }
        self.status = match clipboard::write_files(&paths, cut) {
            Ok(()) if cut => format!("Вырезано: {} — вставьте в папке назначения (Ctrl+V)", paths.len()),
            Ok(()) => format!("Скопировано в буфер: {}", paths.len()),
            Err(error) => format!("Буфер обмена недоступен: {error}"),
        };
        cx.notify();
    }

    /// Ctrl+V: files from the clipboard (ours or Explorer's) are copied, or
    /// moved after a cut, into the focused pane via the audited queue.
    fn clipboard_paste(&mut self, cx: &mut Context<Self>) {
        let files = match clipboard::read_files() {
            Ok(Some(files)) => files,
            Ok(None) => {
                self.status = "В буфере обмена нет файлов".into();
                cx.notify();
                return;
            }
            Err(error) => {
                self.status = format!("Буфер обмена недоступен: {error}");
                cx.notify();
                return;
            }
        };
        let target = self.browser.active().active().path.clone();
        if self.refuse_in_archive(&target, cx) { return; }
        let Some(zone) = self.zone_of(&files.paths, cx) else { return };
        if files.cut {
            // A cut is consumed once, as in Explorer.
            let _ = clipboard::write_files(&[], false);
        }
        self.request_transfer(Transfer { zone, target, copy: !files.cut, keep_both: false, replace: false, restore_failed: false }, cx);
    }

    /// Files dropped on a folder: copied, or moved when `copy` is false.
    /// A left-button drop inside the app, decided like Explorer: Ctrl copies,
    /// Shift moves, otherwise moves within one volume and copies across.
    /// Hands dragged files to Windows once the cursor leaves the window, so
    /// they can be dropped into Explorer or any program that takes files.
    /// The window lost the focus with files still held by the mouse: they
    /// go on as a Windows drag, to be dropped in the other program.
    fn drag_out_on_deactivate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(windows)]
        {
            if self.dragging_out { return; }
            if self.file_drag && cx.has_active_drag() && !self.drag_paths.is_empty()
                && filemanager_core::shell_drag::mouse_button_down(false) {
                let paths = self.drag_paths.clone();
                self.drag_out(paths, window, cx);
            } else if let Some(paths) = self.right_drag.as_ref().filter(|drag| drag.active).map(|drag| drag.paths.clone()) {
                if filemanager_core::shell_drag::mouse_button_down(true) { self.drag_out(paths, window, cx); }
            }
        }
        #[cfg(not(windows))]
        let _ = (window, cx);
    }

    fn drag_out(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        if Self::any_in_archive(&paths) {
            if !self.dragging_out {
                self.status = "Из архива перетаскивайте в папку внутри Papka или копируйте Ctrl+C → Ctrl+V".into();
                cx.notify();
            }
            return;
        }
        #[cfg(windows)]
        {
            if self.dragging_out || paths.is_empty() { return; }
            let Some(hwnd) = window_hwnd(window) else { return };
            cx.stop_active_drag(window);
            self.right_drag = None;
            self.dragging_out = true;
            cx.notify();
            // The system drag runs its own message loop: start it from the
            // executor, outside this event handler.
            cx.spawn(async move |this, cx| {
                let result = filemanager_core::shell_drag::drag_out(hwnd, &paths);
                let _ = this.update(cx, |this, cx| {
                    this.dragging_out = false;
                    if let Err(error) = result { this.status = format!("Не удалось перетащить: {error}"); }
                    cx.notify();
                });
            }).detach();
        }
        #[cfg(not(windows))]
        let _ = (paths, window, cx);
    }

    /// The Explorer context menu for `paths`, with "Open with", "Send to"
    /// and the handlers installed on this computer. `extended` adds the
    /// Shift+right-click verbs. Commands run by Windows, past the journal.
    pub(crate) fn explorer_menu(&mut self, paths: Vec<PathBuf>, extended: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.explorer_menu_for(paths, None, extended, window, cx);
    }

    /// The Explorer menu of a folder's empty space ("New", "Paste", …).
    pub(crate) fn explorer_folder_menu(&mut self, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.explorer_menu_for(vec![folder.clone()], Some(folder), true, window, cx);
    }

    fn explorer_menu_for(&mut self, paths: Vec<PathBuf>, background: Option<PathBuf>, extended: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.context_menu = None;
        self.right_drag = None;
        cx.notify();
        if Self::any_in_archive(&paths) {
            self.status = "Внутри архива меню Проводника нет: извлеките файлы".into();
            return;
        }
        #[cfg(windows)]
        {
            let Some(hwnd) = window_hwnd(window) else { return };
            if paths.is_empty() { return; }
            // The menu runs its own message loop: start it from the executor.
            cx.spawn(async move |this, cx| {
                let result = match &background {
                    Some(folder) => filemanager_core::shell_menu::show_background(hwnd, folder, extended),
                    None => filemanager_core::shell_menu::show(hwnd, &paths, extended),
                };
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(true) => for path in &paths { this.refresh_parent_of(path, cx); },
                        Ok(false) => {}
                        Err(error) => this.status = format!("Меню Проводника недоступно: {error}"),
                    }
                    cx.notify();
                });
            }).detach();
        }
        #[cfg(not(windows))]
        {
            let _ = (paths, background, extended, window);
            self.status = "Меню Проводника есть только в Windows".into();
        }
    }

    fn drop_files(&mut self, paths: &[PathBuf], folder: PathBuf, modifiers: gpui::Modifiers, cx: &mut Context<Self>) {
        // Released over the folder they came from: nothing to do.
        if paths.is_empty() || paths.iter().any(|path| path == &folder)
            || (!modifiers.control && paths.iter().all(|path| path.parent() == Some(folder.as_path()))) {
            return;
        }
        let copy = if modifiers.control { true } else if modifiers.shift { false }
            else { !filemanager_core::path_utils::same_volume(&paths[0], &folder) };
        self.drop_into(paths, folder, copy, cx);
    }

    fn drop_into(&mut self, paths: &[PathBuf], folder: PathBuf, copy: bool, cx: &mut Context<Self>) {
        if self.refuse_in_archive(&folder, cx) { return; }
        // Out of an archive shown as a folder: extracted, never moved.
        if Self::any_in_archive(paths) {
            self.extract_into(paths.to_vec(), folder, cx);
            return;
        }
        // A placeholder from an archiver (7-Zip drags out a folder it fills
        // only when Windows copies it): Explorer's copy engine handles it.
        #[cfg(windows)]
        if paths.iter().any(|path| !path.exists()) {
            self.shell_copy_drop(paths.to_vec(), folder, cx);
            return;
        }
        let Some(zone) = self.zone_of(paths, cx) else { return };
        // Dragged out of «Файлы для переноса»: they leave it, and come back
        // if the transfer does not finish.
        let staged = paths.iter().all(|path| self.zone.items().contains(path));
        if staged { self.zone.remove(paths); }
        self.request_transfer(Transfer { zone, target: folder, copy, keep_both: false, replace: false, restore_failed: staged }, cx);
    }

    #[cfg(windows)]
    fn shell_copy_drop(&mut self, paths: Vec<PathBuf>, folder: PathBuf, cx: &mut Context<Self>) {
        self.status = "Копирование через Windows (распаковка из архива)…".into();
        let target = folder.clone();
        let task = cx.background_spawn(async move { filemanager_core::shell_drag::shell_copy(&paths, &target) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.status = match result {
                    Ok(_) => format!("Передано Windows: файлы появятся в «{}»", browser::display_name(&folder)),
                    Err(error) => format!("Не удалось скопировать: {error}"),
                };
                this.refresh_visible_directories(cx);
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    /// Validated transfer list for external paths (clipboard, drops).
    fn zone_of(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) -> Option<DropZone> {
        let mut zone = DropZone::default();
        for path in paths {
            if let Err(error) = zone.add(path) {
                self.status = format!("Нельзя перенести «{}»: {error}", browser::display_name(path));
                cx.notify();
                return None;
            }
        }
        Some(zone)
    }

    fn stage(&mut self, cx: &mut Context<Self>) {
        let paths = self.selection();
        self.stage_paths(&paths, cx);
    }

    /// Drop Zone → focused pane, copying.
    fn paste(&mut self, cx: &mut Context<Self>) {
        self.transfer_staged(true, cx);
    }

    /// Drop Zone → focused pane, moving within one volume.
    fn move_staged(&mut self, cx: &mut Context<Self>) {
        self.transfer_staged(false, cx);
    }

    fn transfer_staged(&mut self, copy: bool, cx: &mut Context<Self>) {
        if self.zone.items().is_empty() {
            self.status = "Нет файлов для переноса".into();
            cx.notify();
            return;
        }
        let zone = std::mem::take(&mut self.zone);
        let target = self.browser.active().active().path.clone();
        self.request_transfer(Transfer { zone, target, copy, keep_both: false, replace: false, restore_failed: true }, cx);
    }

    /// Checks a transfer before running it: copying into the items' own
    /// folder makes numbered copies (as Explorer does); occupied names
    /// elsewhere ask the user. Nothing is ever replaced.
    fn request_transfer(&mut self, mut transfer: Transfer, cx: &mut Context<Self>) {
        if self.refuse_in_archive(&transfer.target.clone(), cx) {
            self.return_to_staging(&transfer);
            return;
        }
        // One operation changes files at a time; the rest wait their turn.
        if self.transfers.occupied() {
            self.transfers.queue.push_back(transfer);
            self.status = format!("Поставлено в очередь ({}): начнётся после текущей операции", self.transfers.queue.len());
            cx.notify();
            return;
        }
        let items = transfer.zone.items().to_vec();
        let target_plain = filemanager_core::path_utils::normalize_extended_path(&transfer.target);
        if items.iter().any(|path| target_plain.starts_with(filemanager_core::path_utils::normalize_extended_path(path))) {
            self.status = "Нельзя переместить или скопировать папку внутрь самой себя".into();
            self.return_to_staging(&transfer);
            cx.notify();
            return;
        }
        // A folder and something inside it: the folder brings it along.
        // A move into the folder the item is in: nothing to do for it.
        let pruned = transfer.zone.prune_for(&transfer.target, !transfer.copy);
        let skipped = crate::transfers::pruned_note(&pruned);
        if transfer.zone.items().is_empty() {
            self.status = if pruned.nested.is_empty() { "Объекты уже находятся в этой папке".into() } else { skipped };
            if transfer.restore_failed { for path in pruned.nested.iter().chain(&pruned.in_place) { let _ = self.zone.add(path); } }
            cx.notify();
            return;
        }
        // Left-out items of the Drop Zone stay staged; the note is repeated
        // when the transfer ends (its progress replaces the status).
        if transfer.restore_failed { for path in pruned.in_place.iter() { let _ = self.zone.add(path); } }
        self.transfers.skipped_note = (!skipped.is_empty()).then(|| skipped.clone());
        if !skipped.is_empty() { self.status = skipped; }
        let items = transfer.zone.items().to_vec();
        // Staged paths are canonical (\\?\C:\…), the pane's are not.
        if items.iter().all(|path| path.parent().is_some_and(|parent| filemanager_core::path_utils::same_path(parent, &transfer.target))) {
            if !transfer.copy {
                self.status = "Объекты уже находятся в этой папке".into();
                self.return_to_staging(&transfer);
                cx.notify();
                return;
            }
            transfer.keep_both = true;
        }
        // A replacing transfer was already decided on (conflict dialog, sync).
        let conflicts = if transfer.keep_both || transfer.replace { Vec::new() } else { transfer.zone.conflict_details(&transfer.target, 4) };
        if conflicts.is_empty() {
            self.execute_transfer(transfer, cx);
        } else {
            self.transfers.pending = Some((transfer, conflicts));
            cx.notify();
        }
    }

    /// Answer from the name-conflict dialog.
    fn resolve_transfer(&mut self, choice: ConflictChoice, cx: &mut Context<Self>) {
        let Some((mut transfer, conflicts)) = self.transfers.pending.take() else { return };
        match choice {
            ConflictChoice::Cancel => {
                self.transfers.skipped_note = None;
                self.return_to_staging(&transfer);
                self.status = "Отменено, файлы не изменены".into();
            }
            ConflictChoice::Skip => {
                let conflicts: Vec<PathBuf> = conflicts.into_iter().map(|c| c.source).collect();
                transfer.zone.remove(&conflicts);
                if transfer.restore_failed {
                    for path in &conflicts {
                        let _ = self.zone.add(path);
                    }
                }
                if transfer.zone.items().is_empty() {
                    self.transfers.skipped_note = None;
                    self.status = "Все объекты пропущены".into();
                } else {
                    self.execute_transfer(transfer, cx);
                }
            }
            ConflictChoice::KeepBoth => {
                transfer.keep_both = true;
                self.execute_transfer(transfer, cx);
            }
            ConflictChoice::Replace => {
                transfer.replace = true;
                self.execute_transfer(transfer, cx);
            }
        }
        // Nothing started (cancelled, all skipped): the queue moves on.
        self.start_queued(cx);
        cx.notify();
    }

    /// Starts queued transfers once nothing else changes files. One that is
    /// refused (its target is gone, say) does not hold up those behind it.
    pub(crate) fn start_queued(&mut self, cx: &mut Context<Self>) {
        while let Some(next) = self.transfers.next() {
            let said = std::mem::take(&mut self.status);
            self.request_transfer(next, cx);
            // The result of what just ended stays readable beside the new start.
            if !said.is_empty() && !self.transfers.occupied() { self.status = format!("{said} · {}", self.status); }
        }
    }

    /// A cancelled Drop Zone transfer keeps its items staged.
    fn return_to_staging(&mut self, transfer: &Transfer) {
        if transfer.restore_failed {
            for path in transfer.zone.items() {
                let _ = self.zone.add(path);
            }
        }
    }

    fn execute_transfer(&mut self, transfer: Transfer, cx: &mut Context<Self>) {
        let Transfer { zone, target, copy, keep_both, replace, restore_failed } = transfer;
        if copy {
            self.run_copy(zone, target, keep_both, replace, restore_failed, cx);
        } else {
            self.run_move(zone, target, keep_both, replace, restore_failed, cx);
        }
    }

    fn start_transfer_progress(&mut self, action: &'static str, target: PathBuf, cx: &mut Context<Self>) {
        let progress = self.transfers.control.as_ref().unwrap().clone();
        self.transfers.fraction = Some(0.0);
        self.transfers.begin(action, target);
        cx.spawn(async move |weak, cx| {
            // Time spent copying, without pauses: the basis for "осталось".
            let mut active = Duration::ZERO;
            let mut last = std::time::Instant::now();
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let now = std::time::Instant::now();
                if !progress.is_paused() { active += now - last; }
                last = now;
                let keep_going = weak.update(cx, |this, cx| {
                    if !this.transfers.copying || !this.transfers.control.as_ref()
                        .is_some_and(|active| Arc::ptr_eq(active, &progress)) { return false; }
                    let detail = progress.progress();
                    let fraction = progress.fraction().unwrap_or(0.0);
                    this.transfers.fraction = Some(fraction);
                    let phase: String = if progress.is_paused() { "На паузе".into() } else { match progress.phase() {
                        TransferPhase::Verifying => "Проверка копии…".into(),
                        TransferPhase::Recycling => "Завершение переноса…".into(),
                        TransferPhase::Copying => String::new(),
                    } };
                    // An estimate only once there is something to go by.
                    let left = (fraction > 0.02 && active.as_secs() >= 3 && !progress.is_paused())
                        .then(|| remaining(active.as_secs_f64() * (1.0 - fraction) / fraction));
                    // The line below stays short; the panel has the rest.
                    this.status = format!("{}… {}/{}{}", if phase.is_empty() { action } else { phase.trim_end_matches('…') },
                        detail.processed, detail.total, this.transfers.queue_note());
                    if let Some(live) = &mut this.transfers.live {
                        live.phase = phase;
                        live.processed = detail.processed;
                        live.total = detail.total;
                        live.bytes = progress.bytes_copied();
                        live.total_bytes = progress.total_bytes();
                        live.left = left;
                        live.current = detail.source.clone();
                    }
                    cx.notify();
                    true
                }).unwrap_or(false);
                if !keep_going { break; }
            }
        }).detach();
    }

    /// A copy or move ended: one short line below, and the panel with
    /// every error in full. A clean result leaves the screen by itself.
    fn finish_transfer(&mut self, done: &'static str, ok: usize, cancelled: usize, target: PathBuf,
        failures: Vec<transfers::Failure>, cx: &mut Context<Self>) {
        let title = transfers::report_title(done, ok, failures.len(), cancelled);
        // Items that did not get through flash red where they still are.
        self.motion.flash_as(motion::Flash::Failed, failures.iter().map(|failure| failure.path.clone()).collect::<Vec<_>>());
        self.status = if !failures.is_empty() {
            format!("{title}. Ошибки — в окне переноса справа внизу.")
        } else if ok > 0 {
            format!("{title}. Ctrl+Z — отменить.")
        } else {
            format!("{title}.")
        };
        if let Some(note) = self.transfers.skipped_note.take() { self.status = format!("{} {note}.", self.status); }
        if let Some(id) = self.transfers.finish(title, target, failures) {
            cx.spawn(async move |weak, cx| {
                cx.background_executor().timer(Duration::from_secs(6)).await;
                let _ = weak.update(cx, |this, cx| { this.transfers.dismiss(id); cx.notify(); });
            }).detach();
        }
    }

    fn run_copy(&mut self, mut zone: DropZone, target: PathBuf, keep_both: bool, replace: bool, restore_failed: bool, cx: &mut Context<Self>) {
        let Some(audit) = self.operation_journal.as_ref().cloned() else {
            self.status = "Копирование отклонено: журнал операций недоступен".into();
            cx.notify();
            return;
        };
        self.transfers.copying = true;
        let control = Arc::new(CopyControl::default());
        self.transfers.control = Some(Arc::clone(&control));
        self.status = "Копирование…".into();
        let target_shown = target.clone();
        let task = cx.background_spawn(async move {
            let results = if replace {
                zone.transfer_to_audited_replace(Action::Copy, &target, &control, &audit)
            } else if keep_both {
                zone.copy_to_audited_keep_both(&target, &control, &audit)
            } else {
                zone.copy_to_audited(&target, &control, &audit)
            };
            let ok = results.iter().filter(|(_, r)| r.is_ok()).count();
            let errors = results.len() - ok;
            let (failures, cancelled) = transfer_failures(&results, control.is_cancelled());
            let created: Vec<Receipt> = results.iter()
                .filter_map(|(_, result)| result.as_ref().ok().cloned())
                .collect();
            (zone, target, ok, errors, failures, cancelled, created)
        });
        self.start_transfer_progress("Копирование", target_shown, cx);
        cx.spawn(async move |weak, cx| {
            let (zone, target, ok, errors, failures, cancelled, created) = task.await;
            let _ = weak.update(cx, |this, cx| {
                // Failed Drop Zone items stay staged; staging done meanwhile is kept.
                for pending in zone.items().iter().filter(|_| restore_failed) {
                    if let Err(error) = this.zone.add(pending) {
                        this.status = format!("Не удалось вернуть элемент в зону переноса: {error}");
                    }
                }
                this.transfers.copying = false;
                this.transfers.control = None;
                if errors > 0 && !zone.items().is_empty() {
                    this.transfers.retries.push(transfers::Retry { items: zone.items().to_vec(), target: target.clone(), copy: true, keep_both, replace });
                }
                this.load_directory(target.clone(), true, cx);
                this.finish_transfer("Скопировано", ok, cancelled, target, failures, cx);
                if !created.is_empty() {
                    this.motion.flash(created.iter().filter_map(|receipt| receipt.destination.clone()));
                    let label = format!("копирование ({})", created.len());
                    this.push_undo(created, label);
                }
                this.start_queued(cx);
                cx.notify();
            });
        }).detach();
    }

    /// Same-volume rename or verified cross-volume transfer. Each item is prepared again and
    /// journaled before any disk change.
    fn run_move(&mut self, mut zone: DropZone, target: PathBuf, keep_both: bool, replace: bool, restore_failed: bool, cx: &mut Context<Self>) {
        let Some(audit) = self.operation_journal.as_ref().cloned() else {
            self.status = "Перемещение отклонено: журнал операций недоступен".into();
            cx.notify();
            return;
        };
        self.transfers.busy = true;
        self.transfers.copying = true;
        let control = Arc::new(CopyControl::default());
        self.transfers.control = Some(Arc::clone(&control));
        self.start_transfer_progress("Перемещение", target.clone(), cx);
        self.status = "Перемещение…".into();
        let task = cx.background_spawn(async move {
            let results = if replace {
                zone.transfer_to_audited_replace(Action::Move, &target, &control, &audit)
            } else if keep_both {
                zone.move_to_audited_keep_both(&target, &control, &audit)
            } else {
                zone.move_to_audited(&target, &control, &audit)
            };
            let moved = results.iter().filter(|(_, result)| result.is_ok()).count();
            let failed = results.len() - moved;
            let (failures, cancelled) = transfer_failures(&results, control.is_cancelled());
            let sources = results.iter()
                .filter(|(_, result)| result.is_ok())
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            // Every completed move can be reversed, newest first.
            let receipts: Vec<Receipt> = results.iter()
                .filter_map(|(_, result)| result.as_ref().ok().cloned())
                .collect();
            (zone, target, moved, failed, failures, cancelled, sources, receipts)
        });
        cx.spawn(async move |weak, cx| {
            let (zone, target, moved, failed, failures, cancelled, sources, receipts) = task.await;
            let _ = weak.update(cx, |this, cx| {
                for path in zone.items().iter().filter(|_| restore_failed) {
                    if let Err(error) = this.zone.add(path) {
                        this.status = format!("Не удалось вернуть элемент в зону переноса: {error}");
                    }
                }
                this.transfers.busy = false;
                this.transfers.copying = false;
                this.transfers.control = None;
                if failed > 0 && !zone.items().is_empty() {
                    this.transfers.retries.push(transfers::Retry { items: zone.items().to_vec(), target: target.clone(), copy: false, keep_both, replace });
                }
                this.load_directory(target.clone(), true, cx);
                for source in &sources {
                    this.refresh_parent_of(source, cx);
                }
                this.finish_transfer("Перемещено", moved, cancelled, target, failures, cx);
                if moved > 0 {
                    this.motion.flash(receipts.iter().filter_map(|receipt| receipt.destination.clone()));
                    this.selected = normalized(receipts.last().and_then(|receipt| receipt.destination.clone()));
                    let label = if moved == 1 {
                        format!("перенос «{}»", browser::display_name(&receipts[0].source))
                    } else {
                        format!("перенос ({moved})")
                    };
                    this.push_undo(receipts, label);
                }
                this.start_queued(cx);
                cx.notify();
            });
        }).detach();
    }

    fn create_folder(&mut self, cx: &mut Context<Self>) {
        if self.transfers.copying || self.transfers.busy {
            self.status = "Дождитесь завершения текущей операции".into();
            cx.notify();
            return;
        }
        let file = self.creating_file;
        let what = if file { "файл" } else { "папку" };
        let name = self.folder_input.read(cx).value().to_string();
        if let Err(error) = filemanager_core::operations::validate_leaf_name(name.trim()) {
            self.status = format!("Недопустимое имя: {error}");
            cx.notify();
            return;
        }
        let parent = self.browser.active().active().path.clone();
        let destination = parent.join(name.trim());
        let action = if file { Action::CreateFile } else { Action::CreateFolder };
        let plan = match Plan::prepare(action, &parent, Some(&destination)) {
            Ok(plan) => plan,
            Err(error) => {
                self.status = format!("Не удалось создать {what}: {error}");
                cx.notify();
                return;
            }
        };
        self.transfers.busy = true;
        let Some(audit) = self.operation_journal.as_ref().cloned() else {
            self.transfers.busy = false;
            self.start_queued(cx);
            self.status = "Операция отклонена: журнал SQLite недоступен".into();
            cx.notify();
            return;
        };
        let task = cx.background_spawn(async move {
            let mut queue = OperationQueue::default();
            queue.submit(plan);
            queue.run_all_audited(&CopyControl::default(), &audit).remove(0).1
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.transfers.busy = false;
                this.start_queued(cx);
                match result {
                    Ok(receipt) => {
                        this.creating_folder = false;
                        if let Some(ref dest) = receipt.destination {
                            this.refresh_parent_of(dest, cx);
                        }
                        this.status = if file { "Файл создан" } else { "Папка создана" }.into();
                        this.motion.flash(receipt.destination.clone());
                        if let Some(dest) = receipt.destination.clone() {
                            let label = format!("создание {} «{}»", if file { "файла" } else { "папки" }, browser::display_name(&dest));
                            this.push_undo(vec![receipt.clone()], label);
                        }
                        this.selected = normalized(receipt.destination);
                    }
                    Err(error) => this.status = format!("Не удалось создать {what}: {error}"),
                }
                cx.notify();
            });
        }).detach();
    }

    fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if Self::any_in_archive(&self.selection()) {
            self.status = archives::READ_ONLY.into();
            cx.notify();
            return;
        }
        if self.selection().len() > 1 { self.begin_bulk_rename(window, cx); return; }
        self.context_menu = None;
        let Some(path) = &self.selected else {
            self.status = "Сначала выберите файл или папку".into();
            cx.notify();
            return;
        };
        let current = browser::display_name(path);
        // As Explorer: a file's name is selected without its extension.
        let is_file = self.dirs.cache.get(path.parent().unwrap_or(path))
            .and_then(|listing| listing.entries.iter().find(|e| &e.path == path).map(|e| !e.is_directory))
            .unwrap_or(false);
        let stem = current.rfind('.').filter(|dot| is_file && *dot > 0);
        self.rename_input.update(cx, |input, cx| {
            input.set_value(current.clone(), window, cx);
            match stem {
                Some(dot) => input.set_selected_range(0..dot, cx),
                None => input.select_all(window, cx),
            }
        });
        self.renaming = true;
        let focus = self.rename_input.focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        if self.transfers.copying || self.transfers.busy {
            self.status = "Дождитесь завершения текущей операции".into();
            cx.notify();
            return;
        }
        let Some(source) = self.selected.clone() else { return; };
        let name = self.rename_input.read(cx).value().to_string();
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." ||
            name.contains('/') || name.contains('\\') {
            self.status = "Недопустимое имя (без разделителей пути)".into();
            cx.notify();
            return;
        }
        let dest = source.with_file_name(name);
        let plan = match Plan::prepare(Action::Rename, &source, Some(&dest)) {
            Ok(plan) => plan,
            Err(error) => {
                self.status = format!("Переименование невозможно: {error}");
                self.motion.flash_as(motion::Flash::Failed, [source.clone()]);
                cx.notify();
                return;
            }
        };
        self.transfers.busy = true;
        self.status = "Переименование…".into();
        let Some(audit) = self.operation_journal.as_ref().cloned() else {
            self.transfers.busy = false;
            self.start_queued(cx);
            self.status = "Операция отклонена: журнал SQLite недоступен".into();
            cx.notify();
            return;
        };
        let task = cx.background_spawn(async move {
            let mut queue = OperationQueue::default();
            queue.submit(plan);
            queue.run_all_audited(&CopyControl::default(), &audit).remove(0).1
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.transfers.busy = false;
                this.start_queued(cx);
                match result {
                    Ok(receipt) => {
                        this.refresh_parent_of(&receipt.source, cx);
                        if let Some(ref dest) = receipt.destination {
                            this.refresh_parent_of(dest, cx);
                        }
                        this.selected = normalized(receipt.destination.clone());
                        this.motion.flash(receipt.destination.clone());
                        let label = format!("переименование «{}»", browser::display_name(&receipt.source));
                        this.status = "Переименовано. Ctrl+Z — отменить.".into();
                        this.push_undo(vec![receipt], label);
                        this.renaming = false;
                    }
                    Err(error) => {
                        this.status = format!("Переименование отклонено: {error}");
                        this.motion.flash_as(motion::Flash::Failed, this.selected.clone());
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn push_undo(&mut self, receipts: Vec<Receipt>, label: String) {
        self.follow_labels(&receipts, false);
        let all_reversible = receipts.iter().all(Receipt::can_undo);
        let receipts: Vec<_> = receipts.into_iter().filter(Receipt::can_undo).collect();
        if !all_reversible {
            self.status = self.status.replace(". Ctrl+Z — отменить.", ".").replace(". Ctrl+Z — восстановить.", ".");
            self.status.push_str(" Безопасная отмена доступна не для всех файлов. Проверьте их вручную; удалённые — в Корзине Windows.");
        }
        // Do not silently skip an operation without a safe Undo receipt.
        self.undo_stack.push(Undo { receipts, label, unavailable: !all_reversible });
        if self.undo_stack.len() > UNDO_LIMIT {
            self.undo_stack.remove(0);
        }
    }

    /// Ctrl+Z: reverses the newest action. Moves go back through the
    /// audited queue (refused if anything changed since), copies and new
    /// folders go to the Recycle Bin, deletions come back from it.
    fn undo_last(&mut self, cx: &mut Context<Self>) {
        if self.transfers.copying || self.transfers.busy {
            self.status = "Дождитесь завершения текущей операции".into();
            cx.notify();
            return;
        }
        let Some(undo) = self.undo_stack.pop() else {
            self.status = "Нечего отменять".into();
            cx.notify();
            return;
        };
        if undo.receipts.is_empty() && undo.unavailable {
            self.status = format!("Безопасная отмена недоступна: {}. Проверьте результат вручную.", undo.label);
            self.undo_stack.push(undo);
            cx.notify();
            return;
        }
        let Some(audit) = self.operation_journal.as_ref().cloned() else {
            self.undo_stack.push(undo);
            self.status = "Отмена отклонена: журнал операций недоступен".into();
            cx.notify();
            return;
        };
        self.transfers.busy = true;
        self.status = format!("Отмена: {}…", undo.label);
        let label = undo.label.clone();
        let task = cx.background_spawn(async move {
            let mut touched: Vec<PathBuf> = Vec::new();
            let mut errors: Vec<String> = Vec::new();
            let mut warnings: Vec<String> = Vec::new();
            let queue = OperationQueue::default();
            let mut remaining = Vec::new();
            let mut undone = Vec::new();
            for receipt in undo.receipts.iter().rev() {
                match queue.undo_completed_audited(receipt, &audit) {
                    Ok(outcome) => {
                        undone.push(receipt.clone());
                        touched.push(receipt.source.clone());
                        touched.extend(receipt.destination.clone());
                        touched.extend(outcome.restored_path);
                        warnings.extend(outcome.warning);
                    }
                    Err(error) => {
                        errors.push(format!("{}: {error}", browser::display_name(&receipt.source)));
                        remaining.push(receipt.clone());
                    }
                }
            }
            // Original order, so another Ctrl+Z retries only failed items,
            // in reverse execution order. Successful items are never replayed.
            remaining.reverse();
            (touched, errors, remaining, warnings, undo.unavailable, undone)
        });
        cx.spawn(async move |weak, cx| {
            let (touched, errors, remaining, warnings, unavailable, undone) = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.transfers.busy = false;
                this.start_queued(cx);
                this.follow_labels(&undone, true);
                let removed = |path: &std::path::Path| undone.iter().any(|receipt| {
                    matches!(receipt.action, Action::Copy | Action::Move | Action::Rename | Action::CreateFile | Action::CreateFolder)
                        && receipt.destination.as_ref().is_some_and(|destination|
                            filemanager_core::path_utils::same_path(path, destination)
                            || filemanager_core::path_utils::is_inside(path, destination))
                });
                if this.selected.as_deref().is_some_and(&removed) { this.selected = None; this.selected_history_event = None; }
                if this.anchor.as_deref().is_some_and(&removed) { this.anchor = None; }
                this.marked.retain(|path| !removed(path));
                // What came back (a move or rename undone, a deletion
                // restored) lights up where it is again.
                this.motion.flash(undone.iter()
                    .filter(|receipt| matches!(receipt.action, Action::Move | Action::Rename | Action::Recycle | Action::Delete))
                    .map(|receipt| receipt.source.clone()));
                for path in &touched {
                    this.refresh_parent_of(path, cx);
                }
                if !remaining.is_empty() || unavailable {
                    this.undo_stack.push(Undo { receipts: remaining, label: label.clone(), unavailable });
                }
                this.status = match errors.first() {
                    None => format!("Отменено: {label}"),
                    Some(error) => format!("Отмена выполнена не полностью ({label}): {error}"),
                };
                if let Some(warning) = warnings.first() {
                    this.status.push_str(&format!(". {warning}"));
                }
                if unavailable {
                    this.status.push_str(". Для части файлов безопасная отмена недоступна — проверьте их вручную.");
                }
                cx.notify();
            });
        }).detach();
    }

    pub(crate) fn toggle_copy_pause(&mut self, cx: &mut Context<Self>) {
        if let Some(control) = &self.transfers.control {
            let paused = !control.is_paused();
            control.set_paused(paused);
            self.status = if paused { "Передача на паузе".into() } else { "Передача продолжается…".into() };
        }
        cx.notify();
    }

    /// Runs the items that failed last time again, through the same safe
    /// copy or move; items that already arrived are not touched.
    pub(crate) fn retry_failed(&mut self, cx: &mut Context<Self>) {
        let retries = std::mem::take(&mut self.transfers.retries);
        if let Some(id) = self.transfers.report.as_ref().map(|report| report.id) { self.transfers.dismiss(id); }
        let mut missing = 0;
        for retry in retries {
            let mut zone = DropZone::default();
            missing += retry.items.iter().filter(|path| zone.add(path).is_err()).count();
            if zone.items().is_empty() { continue; }
            self.request_transfer(Transfer { zone, target: retry.target, copy: retry.copy,
                keep_both: retry.keep_both, replace: retry.replace, restore_failed: false }, cx);
        }
        if missing > 0 { self.status = format!("Недоступно и пропущено: {missing}"); }
        cx.notify();
    }

    fn cancel_copy(&mut self, cx: &mut Context<Self>) {
        // «Отмена» stops everything asked for, not only the running item.
        let queued = self.transfers.drain();
        for transfer in &queued { self.return_to_staging(transfer); }
        if let Some(control) = &self.transfers.control {
            control.set_paused(false);
            control.cancel();
            self.status = if queued.is_empty() { "Отмена запрошена. Уже завершённые действия сохраняются.".into() }
                else { format!("Отмена запрошена, из очереди снято: {}. Уже завершённые действия сохраняются.", queued.len()) };
        } else if !queued.is_empty() {
            self.status = format!("Из очереди снято: {}", queued.len());
        } else {
            self.status = "Копирование не выполняется".into();
        }
        cx.notify();
    }

    fn recycle(&mut self, cx: &mut Context<Self>) {
        if Self::any_in_archive(&self.selection()) {
            self.status = archives::READ_ONLY.into();
            cx.notify();
            return;
        }
        if self.transfers.copying || self.transfers.busy {
            self.status = "Дождитесь завершения текущей операции".into();
            cx.notify();
            return;
        }
        let paths = self.selection();
        if paths.is_empty() {
            self.status = "Выберите файл".into();
            cx.notify();
            return;
        }

        // The first call validates and freezes the intended sources. The
        // confirmation submits those SAME plans, not newly-prepared commands
        // which might point to different files.
        let plans = match self.confirm_recycle.take() {
            Some(pending) if pending.iter().map(|(path, _)| path).eq(paths.iter()) => pending,
            _ => {
                let mut prepared = Vec::new();
                for path in &paths {
                    // Like Explorer: where there is no Recycle Bin (network
                    // shares) the confirmed deletion is permanent.
                    let action = if filemanager_core::path_utils::has_recycle_bin(path) { Action::Recycle } else { Action::Delete };
                    match Plan::prepare(action, path, None) {
                        Ok(plan) => prepared.push((path.clone(), plan)),
                        Err(error) => {
                            self.status = format!("Удаление невозможно ({}): {error}", browser::display_name(path));
                            cx.notify();
                            return;
                        }
                    }
                }
                let permanent = prepared.iter().any(|(_, plan)| plan.action == Action::Delete);
                self.confirm_recycle = Some(prepared);
                self.status = if permanent { "Подтвердите безвозвратное удаление" } else { "Подтвердите удаление в Корзину" }.into();
                cx.notify();
                return;
            }
        };
        self.selected = None;
        self.marked.clear();
        self.selected_history_event = None;
        self.run_recycle(plans, cx);
    }

    /// Deletes what `plans` name, already confirmed: to the Recycle Bin
    /// where there is one, through the journal, undone with Ctrl+Z.
    pub(crate) fn run_recycle(&mut self, plans: Vec<(PathBuf, Plan)>, cx: &mut Context<Self>) {
        if self.transfers.copying || self.transfers.busy {
            self.status = "Дождитесь завершения текущей операции".into();
            cx.notify();
            return;
        }
        let Some(audit) = self.operation_journal.as_ref().cloned() else {
            self.status = "Операция отклонена: журнал SQLite недоступен".into();
            cx.notify();
            return;
        };
        self.transfers.busy = true;
        self.status = "Удаление…".into();
        let task = cx.background_spawn(async move {
            let mut queue = OperationQueue::default();
            for (_, plan) in plans {
                queue.submit(plan);
            }
            queue.run_all_audited(&CopyControl::default(), &audit)
        });
        cx.spawn(async move |weak, cx| {
            let results = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.transfers.busy = false;
                this.start_queued(cx);
                let mut done = 0;
                let mut deleted = 0;
                let mut first_error = None;
                let mut recycled = Vec::new();
                for (plan, result) in &results {
                    match result {
                        Ok(receipt) if receipt.action == Action::Delete => {
                            deleted += 1;
                            this.refresh_parent_of(&receipt.source, cx);
                        }
                        Ok(receipt) => {
                            done += 1;
                            recycled.push(receipt.clone());
                            this.refresh_parent_of(&receipt.source, cx);
                        }
                        Err(error) => {
                            first_error.get_or_insert(format!("{}: {error}", plan.source.display()));
                        }
                    }
                }
                this.status = match (first_error, deleted) {
                    (None, 0) if done == 1 => "Перемещено в Корзину. Ctrl+Z — восстановить.".into(),
                    (None, 0) => format!("Перемещено в Корзину: {done}"),
                    (None, _) if done == 0 => format!("Удалено безвозвратно: {deleted}"),
                    (None, _) => format!("В Корзину: {done}, удалено безвозвратно: {deleted}"),
                    (Some(error), 0) => format!("В Корзину: {done}, ошибка: {error}"),
                    (Some(error), _) => format!("В Корзину: {done}, удалено безвозвратно: {deleted}, ошибка: {error}"),
                };
                if !recycled.is_empty() {
                    let label = if recycled.len() == 1 {
                        format!("удаление «{}»", browser::display_name(&recycled[0].source))
                    } else {
                        format!("удаление ({})", recycled.len())
                    };
                    this.push_undo(recycled, label);
                }
                // A deleted inbox entry leaves the card at once.
                this.project_card_registry_changed(cx);
                cx.notify();
            });
        }).detach();
    }

    fn configure_live_watch(&self) {
        if let Some(watcher) = &self.watcher {
            let mut folders = if self.search.active { Vec::new() } else { self.visible_directories() };
            // Archives are not folders on disk: nothing to watch there.
            folders.retain(|folder| filemanager_core::archive::split(folder).is_none());
            let journal = self.watched_root.as_ref().and(self.journal.clone());
            watcher.configure(folders, journal);
        }
    }

    fn ensure_live_watch(&mut self, cx: &mut Context<Self>) {
        if self.live_watch_started {
            self.configure_live_watch();
            return;
        }
        self.live_watch_started = true;
        // SQLite startup and notify registration both happen off the UI.
        let journal_task = cx.background_spawn(async { Journal::open(Journal::default_path()).map(Arc::new) });
        cx.spawn(async move |weak, cx| {
            let result = journal_task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(journal) => {
                        this.journal = Some(journal);
                        this.inspector.dirty = true;
                        // History is recorded by default; «Остановить» pauses it.
                        if !this.history_paused && this.watched_root.is_none() {
                            this.watched_root = Some(this.browser.active().active().path.clone());
                            this.configure_live_watch();
                        }
                    }
                    Err(error) => {
                        let reason = format!("История недоступна: {error}");
                        this.status = reason.clone();
                        this.live_watch_error = Some(reason);
                    }
                }
                cx.notify();
            });
        }).detach();
        match DirectoryWatch::start(self.visible_directories(), None) {
            Ok(watch) => self.watcher = Some(watch),
            Err(error) => {
                let reason = format!("Автообновление папок недоступно: {error}");
                self.status = reason.clone();
                self.live_watch_error = Some(reason);
                return;
            }
        }
        cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let running = weak.update(cx, |this, cx| {
                    let Some(watcher) = &this.watcher else { return false; };
                    let changes = watcher.drain();
                    // A passing problem (a burst of changes, one unreadable
                    // file) is reported once, never left on screen.
                    if let Some(error) = changes.error {
                        this.status = format!("Автообновление: {error}");
                        cx.notify();
                    }
                    let unwatched = watcher.unwatched();
                    if unwatched != this.unwatched_folders {
                        this.unwatched_folders = unwatched;
                        cx.notify();
                    }
                    if changes.refresh_all || !changes.paths.is_empty() { this.feed.dirty = true; }
                    if changes.refresh_all || this.selected.as_ref().is_some_and(|selected| {
                        changes.paths.iter().any(|path| selected == path || selected.starts_with(path))
                    }) {
                        this.inspector.dirty = true;
                        cx.notify();
                    }
                    let visible = this.visible_directories();
                    // Files another program or person changed light up softly where
                    // they are on screen. A burst (a sync, an unpacked archive) and
                    // the folder Papka itself is copying into stay unlit.
                    let copying_into = this.transfers.live.as_ref().map(|live| live.target.clone());
                    if changes.paths.len() <= 50 {
                        this.motion.flash_as(motion::Flash::Changed, changes.paths.iter()
                            .filter(|path| path.parent().is_some_and(|parent| visible.iter().any(|folder| folder == parent)
                                && !copying_into.as_deref().is_some_and(|target| filemanager_core::path_utils::same_path(target, parent))))
                            .cloned().collect::<Vec<_>>());
                    }
                    // Invalidating non-visible cache entries prevents an old
                    // listing reappearing when the user navigates back.
                    let dirty = if changes.refresh_all { visible.clone() } else { changes.directories.into_iter().collect() };
                    for folder in dirty {
                        if visible.contains(&folder) && !this.search.active {
                            // A refresh replaces the snapshot when ready; do not
                            // briefly tear down the list (or its scroll layout).
                            this.load_directory(folder, true, cx);
                        } else {
                            this.dirs.cache.remove(&folder);
                            this.folder_views.retain(|(_,path),_|path != &folder);
                        }
                    }
                    true
                }).unwrap_or(false);
                if !running { break; }
            }
        }).detach();
    }

    /// Only currently visible columns are requested. Directory enumeration
    /// runs on a background thread; re-rendering never re-reads the disk.
    fn visible_directories(&self) -> Vec<PathBuf> {
        let mut folders = Vec::new();
        for id in self.browser.leaves() {
            for folder in self.pane_columns(id) {
                if !folders.contains(&folder) { folders.push(folder); }
            }
        }
        folders
    }

    fn load_directory(&mut self, folder: PathBuf, force: bool, cx: &mut Context<Self>) {
        if force { self.forget_thumbs(&folder); }
        let Some(request) = self.dirs.begin(&folder, force) else { return };
        let task_path = folder.clone();
        let task = cx.background_spawn(async move {
            // A zip or 7z archive, or a folder in one, reads as a folder.
            match filemanager_core::archive::split(&task_path) {
                Some((archive, inner)) => filemanager_core::archive::listing(&archive, &inner),
                None => browser::scan_directory(&task_path, 100_000),
            }
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await.map_err(|error| error.to_string());
            let _ = weak.update(cx, |this, cx| {
                let visible: HashSet<PathBuf> = this.visible_directories().into_iter().collect();
                match this.dirs.finish(&folder, request, result, &visible) {
                    directories::Finished::Stale => {}
                    directories::Finished::Reload => this.load_directory(folder, true, cx),
                    directories::Finished::Stored => {
                        // F5's «Обновление…» gives way once its folders are read.
                        if this.dirs.refresh_ended(&folder) && this.status == "Обновление…" {
                            this.status = match this.dirs.errors.get(&folder) {
                                Some(_) => "Обновлено; часть папок не прочитана".into(),
                                None => "Обновлено".into(),
                            };
                        }
                        cx.notify();
                    }
                }
            });
        }).detach();
    }

    fn refresh_parent_of(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        // Operation receipts carry verbatim \\?\ paths (kept for long-path
        // Win32 calls); the listing cache is keyed by normal paths.
        let path = filemanager_core::path_utils::normalize_extended_path(path);
        if let Some(parent) = path.parent() {
            self.load_directory(parent.to_path_buf(), true, cx);
        }
    }

    fn load_visible_directories(&mut self, cx: &mut Context<Self>) {
        for folder in self.visible_directories() {
            self.load_directory(folder, false, cx);
        }
    }

    fn refresh_visible_directories(&mut self, cx: &mut Context<Self>) {
        let folders = self.visible_directories();
        self.dirs.start_refresh(folders.iter().cloned());
        for folder in folders {
            self.load_directory(folder, true, cx);
        }
        self.inspector.path = None;
        self.status = "Обновление…".into();
        cx.notify();
    }

    fn toggle_operation_review(&mut self, cx: &mut Context<Self>) {
        self.operation_review = !self.operation_review;
        self.operation_review_epoch = self.operation_review_epoch.wrapping_add(1);
        if !self.operation_review { cx.notify(); return; }
        let Some(journal) = self.operation_journal.as_ref().cloned() else {
            self.status = "Журнал операций недоступен".into();
            cx.notify(); return;
        };
        let epoch = self.operation_review_epoch;
        self.operation_alerts.clear();
        cx.spawn(async move |weak, cx| {
            loop {
                let worker_journal = journal.clone();
                let result = cx.background_spawn(async move { worker_journal.recent(30) }).await;
                let active = weak.update(cx, |this, cx| {
                    if !this.operation_review || this.operation_review_epoch != epoch { return false; }
                    match result {
                        Ok(jobs) => this.operation_alerts = jobs,
                        Err(error) => this.status = format!("Не удалось прочитать журнал операций: {error}"),
                    }
                    cx.notify(); true
                }).unwrap_or(false);
                if !active { break; }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        }).detach();
        cx.notify();
    }

    fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.browser.active().active().path.display().to_string();
        self.address_input.update(cx, |input, cx| {
            input.set_value(path, window, cx);
        });
        let focus = self.address_input.focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn open_address(&mut self, cx: &mut Context<Self>) {
        let path = self.address_input.read(cx).value().to_string();
        let entered = path.trim().trim_matches('"');
        if entered.is_empty() {
            self.status = "Введите путь к существующей папке".into();
            cx.notify();
            return;
        }
        let candidate = PathBuf::from(entered);
        if !candidate.is_absolute() {
            self.status = "Нужен полный путь, например C:\\Work".into();
            cx.notify();
            return;
        }
        if let Some(group)=self.address_editing {self.browser.focus(group);}
        let tab = self.browser.active_mut();
        // Navigating in the currently focused pane retains the other pane
        // and its independent history, as in a conventional file manager.
        self.status = match tab.navigate(&candidate) {
            Ok(()) => {
                self.selected = None;
                self.selected_history_event = None;
                self.confirm_recycle = None;
                format!("Открыта папка {}", candidate.display())
            }
            Err(err) => format!("Не удалось открыть папку: {err}"),
        };
        self.close_search();
        cx.notify();
    }

}


impl Focusable for Explorer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn main() {
    let helper_args: Vec<_> = std::env::args_os().skip(1).collect();
    if let Some(code) = filemanager_core::preview_host::helper_main(&helper_args) {
        std::process::exit(code);
    }
    filemanager_core::guard::install(filemanager_core::path_utils::app_data_dir());
    // Files rescued from archivers' drops (vendor/gpui_ce_windows): kept a
    // day in case a copy from them was still going, then removed.
    std::thread::spawn(|| {
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
        for entry in entries.flatten() {
            let old = entry.metadata().and_then(|metadata| metadata.modified()).ok()
                .and_then(|time| time.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(24 * 60 * 60));
            let name = entry.file_name().to_string_lossy().into_owned();
            if old && (name.starts_with("papka-drop-") || name.starts_with(archives::TEMP_PREFIX)) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    });
    gpui_platform::application().with_assets(assets::Assets).run(|cx: &mut App| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        view::apply_component_theme(cx);
        keys::bind(cx);
        // Fit small screens (laptops, remote sessions) instead of
        // opening a window larger than the display.
        let preferred = size(px(1360.), px(860.));
        let window_size = cx.primary_display()
            .map(|display| {
                let area = display.visible_bounds().size;
                size(preferred.width.min(area.width * 0.92), preferred.height.min(area.height * 0.92))
            })
            .unwrap_or(preferred);
        let bounds = Bounds::centered(None, window_size, cx);
        // The title bar is drawn by the app (tabs live in it); the system
        // still provides resizing and, on Windows, the caption buttons.
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Papka".into()),
                ..gpui_component::TitleBar::title_bar_options()
            }),
            window_min_size: Some(size(px(760.), px(480.))),
            app_id: Some("filemanager".into()),
            ..gpui_component::TitleBar::window_options()
        };
        cx.open_window(options, |window, cx| {
            let explorer = cx.new(|cx| {
                let mut explorer = Explorer::new(window, cx);
                explorer.load_version_rules();
                explorer.load_thumb_panes();
                explorer.load_look(window, cx);
                explorer
            });
            explorer.update(cx, |this, cx| {
                this.load_drive_space(cx);
                this.start_drive_watch(cx);
                this.refresh_workspace_names();
                this.start_project_autosave(cx);
                this.start_update_checks(cx);
            });
            let saver = explorer.downgrade();
            // Tabs, splits and view mode come back on the next start.
            window.on_window_should_close(cx, move |_, cx| {
                saver.update(cx, |this, cx| {
                    this.prepare_window_close(cx)
                }).unwrap_or(true)
            });
            // gpui_ce_components require Root as the outer window view.
            cx.new(|cx| Root::new(explorer, window, cx))
        })
        .expect("GPUI window failed");
        cx.activate(true);
    });
}

/// "осталось" for a transfer: rounded the way people say it.
/// Every item of a transfer that failed, with its whole error text, and
/// how many were cancelled. Every planned item has a result (the queue
/// answers the rest with `Interrupted` once cancelled), so the counts add
/// up; cancelled items are counted, not listed as errors.
fn transfer_failures(results: &[(PathBuf, std::io::Result<Receipt>)], cancelled: bool) -> (Vec<transfers::Failure>, usize) {
    let mut failures = Vec::new();
    let mut stopped = 0;
    for (path, result) in results {
        let Err(error) = result else { continue };
        if cancelled && transfers::is_cancellation(error) { stopped += 1; continue; }
        failures.push(transfers::Failure {
            path: filemanager_core::path_utils::normalize_extended_path(path),
            error: messages::localize(&error.to_string()),
        });
    }
    (failures, stopped)
}

fn remaining(seconds: f64) -> String {
    let seconds = seconds.round() as u64;
    match seconds {
        0..=9 => "несколько секунд".into(),
        10..=59 => format!("{} с", seconds.div_ceil(5) * 5),
        60..=3599 => format!("{} мин", seconds.div_ceil(60)),
        _ => format!("{} ч {} мин", seconds / 3600, (seconds % 3600) / 60),
    }
}
