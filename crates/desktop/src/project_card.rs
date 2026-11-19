//! The project card: the project's own page, shown in its pane in place of
//! the file list. It opens from the project's name at the head of the tab
//! strip. The user's sections (free text: notes, fields, links to folders)
//! are edited right there and saved with the project, as is the project's
//! own list of folders. Papka fills in the rest — the inbox (what Notch
//! sent), the object passport (`object.json`, read only, shown only when
//! there is one) and the latest changes — and keeps it up to date by itself.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use filemanager_core::browser;
use filemanager_core::history::{self, Event};
use filemanager_core::inbox::{self, Entry, Kind};
use filemanager_core::panes::GroupId;
use filemanager_core::passport::{self, Passport, Raw};
use filemanager_core::project_card::{self as model, Card, Line};
use gpui::{AnyElement, AppContext, Context, Div, ElementId, Entity, Focusable, ObjectFit, Stateful, Window, div, img, prelude::*, px};
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_component::tooltip::Tooltip;

use crate::Explorer;
use crate::theme::*;
use crate::view::{ButtonKind, icon_button, small_icon, text_button};

fn display_text(text: &str) -> String {
    if text.chars().count() <= 400 { text.to_string() }
    else { format!("{}…", text.chars().take(400).collect::<String>()) }
}

/// Changes are read this far back and this many are shown.
const DAYS: i64 = 30;
const CHANGES: usize = 12;
/// The inbox is looked at this often while the card is open…
const INBOX_EVERY: std::time::Duration = std::time::Duration::from_secs(4);
/// …and the passport and the changes every this many looks.
const RELOAD_EVERY: u32 = 4;

#[derive(Default)]
pub(crate) enum PassportState {
    #[default]
    Loading,
    /// No `object.json` in the project's folders or above them.
    Absent,
    /// `raw` and `modified` are what is changed and written back.
    Found { file: PathBuf, page: Option<PathBuf>, passport: Passport, raw: Raw, modified: Option<std::time::SystemTime> },
    Unreadable { file: PathBuf, error: String },
}

/// What is being edited in place.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Target {
    /// A section of the card, by its index.
    Section(usize),
    /// The object's name in the passport.
    PassportTitle,
    /// A passport block by its key (`""`: the loose values on top).
    PassportBlock(String),
    /// A block being added to the passport.
    NewPassportBlock,
    PassportWarnings,
    PassportJournal,
    /// The caption of an inbox entry, by its `.json`.
    Caption(PathBuf),
}

impl Target {
    /// Only a name is typed: Enter saves, there is no text field.
    fn title_only(&self) -> bool { matches!(self, Target::PassportTitle | Target::Caption(_)) }
}

pub(crate) struct Editing {
    target: Target,
    /// Just added: cancelling removes it again.
    fresh: bool,
    original_title: String,
    original_text: String,
}

pub(crate) struct OpenCard {
    pub(crate) group: GroupId,
    pub(crate) name: String,
    card: Card,
    editing: Option<Editing>,
    /// The section whose bin was clicked once: the second click deletes.
    doomed: Option<usize>,
    /// The same for a passport block, by its key.
    doomed_block: Option<String>,
    passport: PassportState,
    /// `None` while they are being read.
    changes: Option<Vec<Event>>,
    /// What each link of the card is on disk: a folder, a file, or (None)
    /// not found. Read off the window's thread; a link not looked at yet
    /// is drawn by its name.
    kinds: HashMap<PathBuf, Option<bool>>,
    /// What other programs sent: `None` while it is read, an error text
    /// when the inbox cannot be read (a share offline).
    inbox: Option<Result<Vec<Entry>, String>>,
    /// The inbox entry whose bin was clicked once: the second click deletes.
    doomed_entry: Option<PathBuf>,
    /// The field for a folder's path is open under the folder list.
    adding_folder: bool,
    request: u64,
}

pub(crate) struct ProjectCards {
    pub(crate) open: Option<OpenCard>,
    title_input: Entity<InputState>,
    text_input: Entity<TextareaState>,
    folder_input: Entity<InputState>,
    journal_input: Entity<InputState>,
    requests: u64,
    /// Enter in the title or the path field acts like its button.
    _subscriptions: Vec<gpui::Subscription>,
}

/// Whether two readings of the passport show the same thing (no redraw).
fn same_passport(a: &PassportState, b: &PassportState) -> bool {
    match (a, b) {
        (PassportState::Absent, PassportState::Absent) => true,
        (PassportState::Found { passport: x, file: f, modified: m, .. }, PassportState::Found { passport: y, file: g, modified: n, .. }) => x == y && f == g && m == n,
        (PassportState::Unreadable { error: x, .. }, PassportState::Unreadable { error: y, .. }) => x == y,
        _ => false,
    }
}

/// A passport value that names a file: absolute, or relative to the
/// passport's folder (`lira/ostrov.txt`). Only the part before the first
/// " · " is looked at.
pub(crate) fn passport_link(passport: &Path, value: &str) -> Option<PathBuf> {
    let first = value.split(" · ").next()?.trim();
    if let Some(path) = model::as_path(first) { return Some(path); }
    let relative = Path::new(first);
    let looks_like_file = first.contains(['/', '\\']) && relative.extension().is_some() && !first.contains("://") && !first.contains(' ');
    looks_like_file.then(|| passport.parent().unwrap_or(Path::new("")).join(relative))
}

/// The newest events of several folders in one list, without the repeats
/// that folders lying inside one another give.
pub(crate) fn merge_changes(lists: Vec<Vec<Event>>, limit: usize) -> Vec<Event> {
    let mut all: Vec<Event> = lists.into_iter().flatten().collect();
    all.sort_by(|a, b| b.observed_ms.cmp(&a.observed_ms).then(b.id.cmp(&a.id)));
    all.dedup_by_key(|event| event.id);
    all.truncate(limit);
    all
}

impl ProjectCards {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Explorer>) -> ProjectCards {
        let title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Название раздела"));
        let folder_input = cx.new(|cx| InputState::new(window, cx).placeholder("Путь к папке, например Z:\\Объекты\\044\\ИГИ"));
        let journal_input = cx.new(|cx| InputState::new(window, cx).placeholder("Новая запись журнала — Enter"));
        let enter = |event: &InputEvent| matches!(event, InputEvent::PressEnter { .. });
        let subscriptions = vec![
            // Enter in a title goes on to the text; where there is no text
            // (a caption, the object's name) it saves.
            cx.subscribe_in(&title_input, window, move |this, _, event: &InputEvent, window, cx| {
                if !enter(event) { return; }
                let title_only = this.cards.open.as_ref().and_then(|open| open.editing.as_ref()).is_some_and(|editing| editing.target.title_only());
                if title_only { this.save_card_section(cx); }
                else { let focus = this.cards.text_input.focus_handle(cx); window.focus(&focus, cx); }
            }),
            cx.subscribe_in(&journal_input, window, move |this, _, event: &InputEvent, window, cx| {
                if enter(event) { this.add_journal_entry(window, cx); }
            }),
            cx.subscribe_in(&folder_input, window, move |this, _, event: &InputEvent, _, cx| {
                if enter(event) { this.add_typed_folder(cx); }
            }),
        ];
        ProjectCards {
            open: None,
            title_input,
            // Keep the editor inside a normal pane; longer text scrolls inside it.
            text_input: cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 12)
                .placeholder("Текст. «Метка: значение» станет полем, путь к папке или файлу — ссылкой")),
            folder_input,
            journal_input,
            requests: 0,
            _subscriptions: subscriptions,
        }
    }

    /// The card shown in pane `group`, if any.
    pub(crate) fn shown_in(&self, group: GroupId) -> bool {
        self.open.as_ref().is_some_and(|open| open.group == group)
    }
}

impl Explorer {
    /// The project's name in the tab strip was clicked: its card takes the
    /// pane, or gives it back to the folders.
    pub(crate) fn toggle_project_card(&mut self, group: GroupId, cx: &mut Context<Self>) {
        if self.cards.shown_in(group) { self.close_project_card(cx); } else { self.open_project_card(group, cx); }
    }

    fn open_project_card(&mut self, group: GroupId, cx: &mut Context<Self>) {
        self.close_project_card(cx);
        let Some(name) = self.browser.group(group).and_then(|pane| pane.project.as_ref()).map(|project| project.name.clone()) else { return };
        let saved = self.workspaces.as_ref().and_then(|store| store.value(&model::storage_key(&name)).ok().flatten());
        self.cards.requests = self.cards.requests.wrapping_add(1);
        let mut card = Card::load(saved.as_deref());
        // The first opening keeps the project's tabs as its folders.
        let tabs: Vec<PathBuf> = self.project_tabs(group).into_iter().map(|(_, path)| path).collect();
        let started = card.start_folders(tabs.iter().map(PathBuf::as_path));
        self.cards.open = Some(OpenCard { group, name, card, editing: None, doomed: None,
            doomed_block: None, passport: PassportState::Loading, changes: None, kinds: HashMap::new(), inbox: None, doomed_entry: None,
            adding_folder: false, request: self.cards.requests });
        if started { self.store_card(); }
        self.context_menu = None;
        self.focus_group(group, cx);
        // Nothing stays chosen in the list hidden under the card.
        self.selected = None; self.marked.clear(); self.anchor = None; self.marquee = None;
        self.confirm_recycle = None; self.address_editing = None; self.filter_open = false;
        self.close_search();
        self.reload_project_card(false, cx);
        self.watch_project_card(cx);
        cx.notify();
    }

    /// The status bar's line while the card covers the list:
    /// «Проект «X» · папок: 3 · во входящих: 2».
    pub(crate) fn card_status(&self) -> Option<String> {
        let open = self.cards.open.as_ref()?;
        let mut parts = vec![format!("Проект «{}»", open.name), format!("папок: {}", open.card.folders.len())];
        if let Some(Ok(entries)) = &open.inbox { parts.push(format!("во входящих: {}", entries.len())); }
        Some(parts.join(" · "))
    }

    /// The project's inbox, as published for other programs.
    fn card_inbox(&self) -> Option<PathBuf> {
        let open = self.cards.open.as_ref()?;
        self.registry.projects.get(&open.name).map(|published| published.project.inbox.clone())
    }

    /// Reads the inbox of the open card off the window's thread.
    fn read_project_inbox(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.cards.open.as_ref().map(|open| open.name.clone()) else { return };
        let Some(folder) = self.card_inbox() else { return };
        let task = cx.background_spawn(async move {
            match inbox::read(&folder) {
                Ok(entries) => Ok(entries),
                // Nothing sent yet: the folder appears with the first entry.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
                Err(error) => Err(error.to_string()),
            }
        });
        cx.spawn(async move |this, cx| {
            let entries = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(open) = this.cards.open.as_mut().filter(|open| open.name == name) else { return };
                if open.inbox.as_ref() != Some(&entries) { open.inbox = Some(entries); cx.notify(); }
            });
        }).detach();
    }

    /// While the card is open it keeps itself up to date: the inbox every
    /// few seconds (a screenshot sent from Notch shows up by itself), the
    /// passport and the changes a little less often. Nothing is redrawn
    /// unless something changed.
    fn watch_project_card(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.cards.open.as_ref().map(|open| open.name.clone()) else { return };
        cx.spawn(async move |this, cx| {
            let mut ticks = 0u32;
            loop {
                cx.background_executor().timer(INBOX_EVERY).await;
                ticks = ticks.wrapping_add(1);
                let alive = this.update(cx, |this, cx| {
                    if !this.cards.open.as_ref().is_some_and(|open| open.name == name) { return false; }
                    this.read_project_inbox(cx);
                    // Not under the user's hands: a passport being edited stays as read.
                    let editing = this.cards.open.as_ref().is_some_and(|open| open.editing.is_some());
                    if ticks % RELOAD_EVERY == 0 && !editing { this.reload_project_card(true, cx); }
                    true
                }).unwrap_or(false);
                if !alive { break; }
            }
        }).detach();
    }

    /// `projects.json` was written again: the inbox may have moved.
    pub(crate) fn project_card_registry_changed(&mut self, cx: &mut Context<Self>) {
        if self.cards.open.is_some() { self.read_project_inbox(cx); }
    }

    /// «В заметки»: the entry's caption, text or link goes to the end of
    /// the first section of the card.
    fn inbox_to_notes(&mut self, entry: Entry, cx: &mut Context<Self>) {
        let Some(open) = self.cards.open.as_mut() else { return };
        if open.editing.is_some() { return; }
        let mut lines = vec![format!("{} — {}", entry.when(), entry.title())];
        if let Some(text) = entry.sidecar.text.as_deref().filter(|_| entry.sidecar.kind == Kind::Text) {
            lines.extend(text.lines().map(str::to_string).filter(|line| !line.trim().is_empty() && *line.trim() != entry.title()));
        }
        if let Some(image) = &entry.image { lines.push(image.display().to_string()); }
        if let Some(file) = &entry.sidecar.file { lines.push(file.display().to_string()); }
        if open.card.sections.is_empty() && open.card.add("Заметки").is_none() { return; }
        let (title, text) = (open.card.sections[0].title.clone(), open.card.sections[0].text.clone());
        let joined = if text.trim().is_empty() { lines.join("\n") } else { format!("{text}\n\n{}", lines.join("\n")) };
        open.card.set(0, &title, &joined);
        let section = open.card.sections[0].title.clone();
        self.store_card();
        self.look_at_card_links(cx);
        self.status = format!("Добавлено в «{section}»");
        cx.notify();
    }

    /// The bin of an inbox entry: the first click asks, the second sends
    /// its picture and its `.json` to the Recycle Bin through the journal
    /// (Ctrl+Z brings them back). On a share without one it is permanent,
    /// and the question says so.
    fn delete_inbox_entry(&mut self, entry: Entry, cx: &mut Context<Self>) {
        let Some(open) = self.cards.open.as_mut() else { return };
        if open.doomed_entry.as_ref() != Some(&entry.sidecar_path) {
            open.doomed_entry = Some(entry.sidecar_path.clone());
            cx.notify();
            return;
        }
        open.doomed_entry = None;
        let mut plans = Vec::new();
        for path in entry.image.iter().chain(std::iter::once(&entry.sidecar_path)) {
            let action = if filemanager_core::path_utils::has_recycle_bin(path) {
                filemanager_core::operations::Action::Recycle
            } else {
                filemanager_core::operations::Action::Delete
            };
            match filemanager_core::operations::Plan::prepare(action, path, None) {
                Ok(plan) => plans.push((path.clone(), plan)),
                Err(error) => {
                    self.status = format!("Запись не удалена: {error}");
                    cx.notify();
                    return;
                }
            }
        }
        self.run_recycle(plans, cx);
    }

    /// Adds a folder to project `name`'s card, open or not. From the
    /// folder's context menu, or the field under the card's folder list.
    pub(crate) fn add_folder_to_project(&mut self, name: &str, path: PathBuf, cx: &mut Context<Self>) {
        let added = if self.cards.open.as_ref().is_some_and(|open| open.name == name) {
            let open = self.cards.open.as_mut().unwrap();
            let added = open.card.add_folder(&path);
            self.store_card();
            self.look_at_card_links(cx);
            added
        } else {
            let Some(store) = &self.workspaces else { return };
            let mut card = Card::load(store.value(&model::storage_key(name)).ok().flatten().as_deref());
            let added = card.add_folder(&path);
            if let Err(error) = store.set_value(&model::storage_key(name), &card.to_json()) {
                self.status = format!("Папка не добавлена: {error}");
                cx.notify();
                return;
            }
            added
        };
        self.status = if added { format!("«{}» — в папках проекта «{name}»", browser::display_name(&path)) }
            else { format!("«{}» уже в папках проекта «{name}»", browser::display_name(&path)) };
        self.registry.due = true;
        cx.notify();
    }

    /// The path typed under the folder list: checked off the window's
    /// thread (a share may be slow), then added.
    fn add_typed_folder(&mut self, cx: &mut Context<Self>) {
        let typed = self.cards.folder_input.read(cx).value().trim().trim_matches('"').to_string();
        let Some(name) = self.cards.open.as_ref().map(|open| open.name.clone()) else { return };
        if typed.is_empty() { return; }
        let path = PathBuf::from(&typed);
        let probe = path.clone();
        let task = cx.background_spawn(async move { probe.is_dir() });
        cx.spawn(async move |this, cx| {
            let is_dir = task.await;
            let _ = this.update(cx, |this, cx| {
                if !is_dir {
                    this.status = format!("Папки «{typed}» нет или она недоступна");
                    cx.notify();
                    return;
                }
                if let Some(open) = this.cards.open.as_mut() { open.adding_folder = false; this.interface_focus_pending = true; }
                this.add_folder_to_project(&name, path, cx);
            });
        }).detach();
    }

    fn open_folder_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.cards.open.as_mut() else { return };
        open.adding_folder = true;
        self.cards.folder_input.update(cx, |input, cx| input.set_value("", window, cx));
        let focus = self.cards.folder_input.focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// «Взять открытые вкладки»: the project's tabs join its folder list.
    fn add_tabs_to_folders(&mut self, cx: &mut Context<Self>) {
        let Some(group) = self.cards.open.as_ref().map(|open| open.group) else { return };
        let tabs = self.project_tabs(group);
        let Some(open) = self.cards.open.as_mut() else { return };
        let added = tabs.iter().filter(|(_, path)| open.card.add_folder(path)).count();
        self.store_card();
        self.look_at_card_links(cx);
        self.registry.due = true;
        self.status = if added > 0 { format!("Добавлено папок: {added}") } else { "Все открытые вкладки уже в списке".into() };
        cx.notify();
    }

    /// Reads the passport and the changes again. Everything on disk (a
    /// passport on a network drive, the journal) is read off the window's
    /// thread; the card is on screen meanwhile.
    /// `quiet`: a background refresh — what is shown stays until the new
    /// answer, so nothing blinks.
    fn reload_project_card(&mut self, quiet: bool, cx: &mut Context<Self>) {
        let paths = self.card_folders();
        if paths.is_empty() && self.cards.open.is_none() { return; }
        self.cards.requests = self.cards.requests.wrapping_add(1);
        let request = self.cards.requests;
        let Some(open) = self.cards.open.as_mut() else { return };
        open.request = request;
        if !quiet {
            open.passport = PassportState::Loading;
            open.changes = None;
        }
        self.look_at_card_links(cx);
        let journal = self.journal.clone();
        let task = cx.background_spawn(async move {
            let passport = match passport::find(&paths) {
                None => PassportState::Absent,
                Some(file) => match passport::read_raw(&file) {
                    Ok((raw, modified)) => PassportState::Found { page: passport::page_of(&file), passport: passport::from_raw(&raw), file, raw, modified },
                    Err(error) => PassportState::Unreadable { file, error: error.to_string() },
                },
            };
            let since = Some(history::days_ago_ms(DAYS));
            let changes = journal.map(|journal| merge_changes(
                paths.iter().filter_map(|folder| journal.folder_events(folder, since, CHANGES).ok()).collect(), CHANGES))
                .unwrap_or_default();
            (passport, changes)
        });
        cx.spawn(async move |this, cx| {
            let (passport, changes) = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(open) = this.cards.open.as_mut().filter(|open| open.request == request) else { return };
                let same = open.changes.as_ref() == Some(&changes) && same_passport(&open.passport, &passport);
                open.passport = passport;
                open.changes = Some(changes);
                if !same { cx.notify(); }
            });
        }).detach();
        cx.notify();
    }

    /// Finds out what the card's links are: folder, file or missing. A link
    /// on a network drive may take seconds, so it is done in the background.
    fn look_at_card_links(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.cards.open.as_ref() else { return };
        let links = model::links(&open.card);
        let name = open.name.clone();
        let task = cx.background_spawn(async move {
            links.into_iter().map(|path| {
                let kind = std::fs::metadata(&path).ok().map(|metadata| metadata.is_dir());
                (path, kind)
            }).collect::<HashMap<_, _>>()
        });
        cx.spawn(async move |this, cx| {
            let kinds = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(open) = this.cards.open.as_mut().filter(|open| open.name == name) else { return };
                let changed = kinds.iter().any(|(path, kind)| open.kinds.get(path) != Some(kind));
                open.kinds.extend(kinds);
                if changed { cx.notify(); }
            });
        }).detach();
    }

    /// Where the card looks for the passport and the changes: the
    /// project's saved folders and the tabs open in it now.
    fn card_folders(&self) -> Vec<PathBuf> {
        let Some(open) = self.cards.open.as_ref() else { return Vec::new() };
        let mut folders: Vec<PathBuf> = open.card.folders.iter().map(|folder| folder.path.clone()).collect();
        for (_, path) in self.project_tabs(open.group) {
            if !folders.contains(&path) { folders.push(path); }
        }
        folders
    }

    /// The project's tabs: title and folder.
    fn project_tabs(&self, group: GroupId) -> Vec<(String, PathBuf)> {
        self.browser.group(group).map(|pane| pane.tabs.iter().map(|tab| (tab.title.clone(), tab.pane.path.clone())).collect())
            .unwrap_or_default()
    }

    /// Gives the pane back to its folders. Text being edited is kept: a
    /// click on a tab must not lose a note.
    pub(crate) fn close_project_card(&mut self, cx: &mut Context<Self>) {
        if self.cards.open.as_ref().is_some_and(|open| open.editing.is_some()) { self.save_card_section(cx); }
        if self.cards.open.take().is_some() { self.interface_focus_pending = true; cx.notify(); }
    }

    /// The pane moves on (another tab, another folder): its card closes.
    pub(crate) fn leave_project_card(&mut self, group: GroupId, cx: &mut Context<Self>) {
        if self.cards.shown_in(group) { self.close_project_card(cx); }
    }

    /// Esc: out of the edit first (what was typed is kept, as everywhere in
    /// Windows), then out of the card. Whether it did anything.
    pub(crate) fn dismiss_project_card(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(open) = self.cards.open.as_mut() else { return false };
        if open.doomed.take().is_some() || open.doomed_entry.take().is_some() || open.doomed_block.take().is_some() { cx.notify(); return true; }
        if std::mem::take(&mut open.adding_folder) { self.interface_focus_pending = true; cx.notify(); return true; }
        if open.editing.is_some() { self.save_card_section(cx); } else { self.close_project_card(cx); }
        true
    }

    /// «Отмена» while editing: the section goes back to what it was; a
    /// section just added goes away.
    fn cancel_card_edit(&mut self, cx: &mut Context<Self>) {
        self.interface_focus_pending = true;
        let Some(open) = self.cards.open.as_mut() else { return };
        if let Some(Editing { target: Target::Section(index), fresh: true, .. }) = open.editing.take() {
            open.card.remove(index);
        }
        cx.notify();
    }

    fn store_card(&mut self) {
        let Some(open) = &self.cards.open else { return };
        let result = self.workspaces.as_ref().ok_or_else(|| std::io::Error::other("база настроек недоступна"))
            .and_then(|store| store.set_value(&model::storage_key(&open.name), &open.card.to_json()));
        if let Err(error) = result { self.status = format!("Карточка проекта не сохранена: {error}"); }
    }

    fn edit_card_section(&mut self, index: usize, fresh: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(section) = self.cards.open.as_ref().and_then(|open| open.card.sections.get(index)) else { return };
        // A new section starts with an empty name to type over.
        let (title, text) = (if fresh { String::new() } else { section.title.clone() }, section.text.clone());
        self.begin_edit(Target::Section(index), fresh, title, text, window, cx);
    }

    /// Puts `title` and `text` into the fields and the caret where typing
    /// starts: the name for something new or with no text, else the text.
    fn begin_edit(&mut self, target: Target, fresh: bool, title: String, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.cards.open.as_ref().is_some_and(|open| open.editing.is_some()) { self.save_card_section(cx); }
        let Some(open) = self.cards.open.as_mut() else { return };
        self.interface_focus_pending = false;
        let title_first = fresh || target.title_only();
        open.editing = Some(Editing { target, fresh, original_title: title.clone(), original_text: text.clone() });
        open.doomed = None;
        open.doomed_block = None;
        // The caret goes to the end, where one goes on writing.
        let end = |text: &str| {
            let lines = text.split('\n').collect::<Vec<_>>();
            gpui_component::input::Position { line: lines.len().saturating_sub(1) as u32, character: lines.last().map_or(0, |l| l.encode_utf16().count()) as u32 }
        };
        let (title_end, text_end) = (end(&title), end(&text));
        self.cards.title_input.update(cx, |input, cx| input.set_value(title, window, cx));
        self.cards.text_input.update(cx, |input, cx| input.set_value(text, window, cx));
        if title_first {
            self.cards.title_input.update(cx, |input, cx| input.set_cursor_position(title_end, window, cx));
        } else {
            self.cards.text_input.update(cx, |input, cx| input.set_cursor_position(text_end, window, cx));
        }
        cx.notify();
    }

    fn add_card_section(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cards.open.as_ref().is_some_and(|open| open.editing.is_some()) { self.save_card_section(cx); }
        let Some(open) = self.cards.open.as_mut() else { return };
        match open.card.add("Новый раздел") {
            Some(index) => self.edit_card_section(index, true, window, cx),
            None => { self.status = "В карточке слишком много разделов".into(); cx.notify(); }
        }
    }

    fn save_card_section(&mut self, cx: &mut Context<Self>) {
        let title = self.cards.title_input.read(cx).value().to_string();
        let text = self.cards.text_input.read(cx).value().to_string();
        let Some(open) = self.cards.open.as_mut() else { return };
        let Some(editing) = open.editing.take() else { return };
        self.interface_focus_pending = true;
        if !editing.fresh && title == editing.original_title && text == editing.original_text { cx.notify(); return; }
        match editing.target {
            Target::Section(index) => {
                if editing.fresh && title.trim().is_empty() && text.trim().is_empty() {
                    // Added and left empty: nothing to keep.
                    open.card.remove(index);
                    cx.notify();
                    return;
                }
                open.card.set(index, &title, &text);
                self.store_card();
                self.look_at_card_links(cx);
            }
            Target::Caption(sidecar) => self.write_caption(sidecar, title, cx),
            Target::PassportTitle => self.change_passport(move |raw| passport::set_title(raw, &title), cx),
            Target::PassportBlock(key) => self.change_passport(move |raw| passport::set_block(raw, Some(&key), &title, &text), cx),
            Target::NewPassportBlock => {
                if !(title.trim().is_empty() && text.trim().is_empty()) {
                    self.change_passport(move |raw| passport::set_block(raw, None, &title, &text), cx);
                }
            }
            Target::PassportWarnings => self.change_passport(move |raw| passport::set_warnings(raw, &text), cx),
            Target::PassportJournal => self.change_passport(move |raw| passport::set_journal(raw, &text), cx),
        }
        cx.notify();
    }

    /// Writes the caption into the entry's `.json` off the window's thread.
    fn write_caption(&mut self, sidecar: PathBuf, caption: String, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move { inbox::set_caption(&sidecar, &caption) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result { this.status = format!("Подпись не сохранена: {error}"); }
                this.read_project_inbox(cx);
                cx.notify();
            });
        }).detach();
    }

    /// Changes the passport and writes it, off the window's thread. If the
    /// file was changed elsewhere since it was read, nothing is written and
    /// the card reads it again.
    fn change_passport(&mut self, change: impl FnOnce(&mut Raw) + Send + 'static, cx: &mut Context<Self>) {
        let Some(PassportState::Found { file, raw, modified, .. }) = self.cards.open.as_ref().map(|open| &open.passport) else { return };
        let (file, mut raw, modified) = (file.clone(), raw.clone(), *modified);
        let name = self.cards.open.as_ref().map(|open| open.name.clone()).unwrap_or_default();
        let task = cx.background_spawn(async move {
            change(&mut raw);
            passport::touch(&mut raw, &passport::today(), "Papka");
            passport::write(&file, &raw, modified).map(|written| (raw, written))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(open) = this.cards.open.as_mut().filter(|open| open.name == name) else { return };
                match result {
                    Ok((raw, written)) => {
                        if let PassportState::Found { passport: shown, raw: kept, modified, .. } = &mut open.passport {
                            *shown = passport::from_raw(&raw);
                            *kept = raw;
                            *modified = written;
                        }
                        this.status = "Паспорт объекта сохранён".into();
                    }
                    Err(error) => {
                        this.status = format!("Паспорт не сохранён: {error}");
                        this.reload_project_card(true, cx);
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    /// «Создать паспорт объекта»: `object.json` in the object's folder,
    /// with the project's name and empty blocks to fill.
    fn create_passport(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.cards.open.as_ref().map(|open| open.name.clone()) else { return };
        let Some(root) = self.registry.projects.get(&name).and_then(|published| published.project.root.clone()) else {
            self.status = "Сначала укажите папку объекта: домик у папки в «Папках проекта»".into();
            cx.notify();
            return;
        };
        let file = root.join(passport::FILE_NAME);
        let task = cx.background_spawn(async move {
            let mut raw = passport::template(&name);
            passport::touch(&mut raw, &passport::today(), "Papka");
            passport::write(&file, &raw, None).map(|_| file)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.status = match result {
                    Ok(file) => format!("Паспорт создан: {}", file.display()),
                    Err(error) => format!("Паспорт не создан: {error}"),
                };
                this.reload_project_card(false, cx);
                this.registry.due = true;
            });
        }).detach();
    }

    /// Enter in the journal's field: a dated line at its end.
    fn add_journal_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.cards.journal_input.read(cx).value().trim().to_string();
        if text.is_empty() { return; }
        self.cards.journal_input.update(cx, |input, cx| input.set_value("", window, cx));
        let date = passport::today();
        self.change_passport(move |raw| passport::add_journal(raw, &date, &text), cx);
    }

    /// The bin of a passport block: the first click asks, the second removes it.
    fn delete_passport_block(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(open) = self.cards.open.as_mut() else { return };
        if open.doomed_block.as_ref() == Some(&key) {
            open.doomed_block = None;
            self.change_passport(move |raw| passport::remove_block(raw, &key), cx);
        } else {
            open.doomed_block = Some(key);
        }
        cx.notify();
    }

    /// The bin: the first click asks, the second deletes.
    fn delete_card_section(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(open) = self.cards.open.as_mut() else { return };
        if open.doomed == Some(index) {
            open.doomed = None;
            open.editing = None;
            open.card.remove(index);
            self.store_card();
        } else {
            open.doomed = Some(index);
        }
        cx.notify();
    }

    fn change_card(&mut self, change: impl FnOnce(&mut Card), cx: &mut Context<Self>) {
        let Some(open) = self.cards.open.as_mut() else { return };
        if open.editing.is_some() { return; }
        open.doomed = None;
        change(&mut open.card);
        self.store_card();
        cx.notify();
    }

    /// A link of the card: a folder opens in a new tab of the pane, a
    /// file — its folder in a new tab, with the file selected. The disk is
    /// asked off the window's thread: a link may point to a slow share.
    fn open_card_link(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(group) = self.cards.open.as_ref().map(|open| open.group) else { return };
        let probe = path.clone();
        let task = cx.background_spawn(async move { std::fs::metadata(&probe).map(|metadata| metadata.is_dir()) });
        cx.spawn(async move |this, cx| {
            let kind = task.await;
            let _ = this.update(cx, |this, cx| {
                let (folder, select) = match kind {
                    Ok(true) => (path.clone(), None),
                    Ok(false) => (path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.clone()), Some(path.clone())),
                    Err(_) => {
                        if let Some(open) = this.cards.open.as_mut() { open.kinds.insert(path.clone(), None); }
                        this.status = format!("«{}» не найдено: папка недоступна или переименована", path.display());
                        cx.notify();
                        return;
                    }
                };
                this.close_project_card(cx);
                this.focus_group(group, cx);
                this.open_in_new_tab(&folder, cx);
                if let Some(file) = select {
                    this.motion.flash_as(crate::motion::Flash::Found, [file.clone()]);
                    this.navigate_side(group, &folder, Some(file), cx);
                }
            });
        }).detach();
    }

    fn open_outside(&mut self, path: &Path, cx: &mut Context<Self>) {
        if let Err(error) = open::that(path) { self.status = format!("Не удалось открыть «{}»: {error}", browser::display_name(path)); }
        cx.notify();
    }

    fn link_row(&self, id: impl Into<ElementId>, path: &Path, title: Option<String>, cx: &mut Context<Self>) -> Stateful<Div> {
        let full = path.display().to_string();
        let target = path.to_path_buf();
        // What the background look found; by the name until it answers
        // (the card is drawn every frame, the disk may be a slow share).
        let kind = self.cards.open.as_ref().and_then(|open| open.kinds.get(path).copied());
        let missing = kind == Some(None);
        let folder = match kind { Some(Some(is_dir)) => is_dir, _ => path.extension().is_none() };
        let tip = if missing { format!("Не найдено: {full}") } else { format!("Открыть в новой вкладке: {full}") };
        div().id(id).min_h(px(24.)).flex().items_center().gap_2().px_1().rounded(px(4.)).cursor_pointer()
            .hover(|style| style.bg(rgb(HOVER)))
            .child(if folder { small_icon("fm/folder-fill.svg", if missing { TEXT_DIM } else { FOLDER }) } else { small_icon("fm/file.svg", TEXT_MUTED) })
            .child(div().flex_none().max_w(px(260.)).truncate().text_color(rgb(if missing { TEXT_DIM } else { ACCENT }))
                .when(missing, |name| name.line_through())
                .child(title.unwrap_or_else(|| browser::display_name(path))))
            .child(div().flex_1().min_w_0().truncate().text_size(px(11.)).text_color(rgb(TEXT_DIM)).child(full.clone()))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| this.open_card_link(target.clone(), cx)))
    }

    /// The fields of what is edited, in its place: a name (or a fixed
    /// `heading`) with «Отмена» and «Готово», and the text under it. The
    /// fields' own padding is taken back so nothing moves.
    fn edit_fields(&self, heading: Option<String>, with_title: bool, with_text: bool, cx: &mut Context<Self>) -> Div {
        div().flex().flex_col().gap_1()
            .child(div().h(px(28.)).flex().items_center().gap_2()
                .child(match heading {
                    Some(heading) => div().flex_1().min_w_0().text_size(px(12.)).text_color(rgb(TEXT_MUTED)).child(heading),
                    None if with_title => div().flex_1().min_w_0().ml(px(-10.)).text_size(px(14.)).font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(Input::new(&self.cards.title_input).appearance(false)),
                    None => div().flex_1(),
                })
                .child(text_button("card-cancel", "Отмена", ButtonKind::Ghost)
                    .tooltip(|window, cx| Tooltip::new("Вернуть как было").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_card_edit(cx))))
                .child(text_button("card-save", "Готово", ButtonKind::Primary)
                    .tooltip(|window, cx| Tooltip::new("Сохранить (Esc тоже сохраняет)").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.save_card_section(cx)))))
            .when(with_text, |fields| fields.child(div().mx(px(-12.)).rounded(px(4.)).bg(rgb(HOVER)).text_size(px(13.))
                .child(Textarea::new(&self.cards.text_input).appearance(false))))
    }

    /// One thing another program sent: picture, caption, when and from where.
    fn inbox_row(&self, index: usize, entry: &Entry, busy: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        // "notch.exe · Полка снимков" reads as "Notch · Полка снимков".
        let app = match entry.sidecar.source.app.as_str() { "notch.exe" => "Notch", other => other };
        let source = [app, entry.sidecar.source.window.as_str()]
            .into_iter().filter(|part| !part.trim().is_empty()).collect::<Vec<_>>().join(" · ");
        let shown = entry.image.clone().unwrap_or_else(|| entry.sidecar_path.clone());
        let doomed = self.cards.open.as_ref().is_some_and(|open| open.doomed_entry.as_ref() == Some(&entry.sidecar_path));
        let permanent = !filemanager_core::path_utils::has_recycle_bin(&entry.sidecar_path);
        // A click on the caption writes or changes it, in place.
        let captioning = self.cards.open.as_ref().and_then(|open| open.editing.as_ref())
            .is_some_and(|editing| editing.target == Target::Caption(entry.sidecar_path.clone()));
        let (sidecar, current) = (entry.sidecar_path.clone(), entry.sidecar.caption.clone());
        let unsigned = entry.sidecar.caption.trim().is_empty();
        let mut text = div().flex_1().min_w_0().flex().flex_col().gap_0p5()
            .child(if captioning {
                div().flex().items_center().gap_2()
                    .child(div().flex_1().min_w_0().ml(px(-10.)).text_size(px(13.)).child(Input::new(&self.cards.title_input).appearance(false)))
                    .child(text_button("card-caption-save", "Готово", ButtonKind::Primary)
                        .on_click(cx.listener(|this, _, _, cx| this.save_card_section(cx))))
                    .into_any_element()
            } else {
                div().id(("card-inbox-title", index)).text_size(px(13.)).cursor_text()
                    .text_color(rgb(if unsigned && entry.sidecar.kind == Kind::Image { TEXT_MUTED } else { TEXT }))
                    .child(if unsigned && entry.sidecar.kind == Kind::Image { "Снимок без подписи — щёлкните, чтобы подписать".to_string() } else { entry.title() })
                    .tooltip(|window, cx| Tooltip::new("Щёлкните, чтобы изменить подпись").build(window, cx))
                    .on_click(cx.listener(move |this, _, window, cx|
                        this.begin_edit(Target::Caption(sidecar.clone()), false, current.clone(), String::new(), window, cx)))
                    .into_any_element()
            })
            .child(div().text_size(px(11.)).text_color(rgb(TEXT_DIM))
                .child(if source.is_empty() { entry.when() } else { format!("{} · {source}", entry.when()) }));
        if entry.sidecar.kind == Kind::Text {
            // Without the line already shown as the title.
            let title = entry.title();
            let preview: Vec<&str> = entry.sidecar.text.as_deref().unwrap_or_default().lines()
                .filter(|line| !line.trim().is_empty() && line.trim() != title).take(4).collect();
            text = text.child(div().text_size(px(12.)).text_color(rgb(TEXT_MUTED)).child(preview.join("\n")));
        }
        if let Some(file) = &entry.sidecar.file {
            text = text.child(self.link_row(("card-inbox-file", index), file, None, cx));
        }
        if doomed {
            text = text.child(div().text_size(px(12.)).text_color(rgb(DANGER))
                .child(if permanent { "Удалить безвозвратно (здесь нет Корзины)? Нажмите корзину ещё раз" } else { "Удалить в Корзину? Нажмите корзину ещё раз" }));
        }
        let (notes, reveal, gone) = (entry.clone(), shown.clone(), entry.clone());
        let group: gpui::SharedString = format!("card-inbox-{index}").into();
        div().id(("card-inbox", index)).group(group.clone()).flex().items_start().gap_3().py_1()
            .when_some(entry.image.clone(), |row, picture| row.child(div().id(("card-inbox-image", index)).flex_none().w(px(140.)).h(px(84.))
                .rounded(px(4.)).overflow_hidden().border_1().border_color(rgb(BORDER)).cursor_pointer()
                .child(img(picture.clone()).size_full().object_fit(ObjectFit::Contain))
                .tooltip(|window, cx| Tooltip::new("Открыть снимок").build(window, cx))
                .on_click(cx.listener(move |this, _, _, cx| this.open_outside(&picture, cx)))))
            .child(text)
            // Actions show on the row under the mouse; a question stays.
            .child(div().flex_none().flex().items_center().gap_1().when(!doomed, |row| row.invisible().group_hover(group, |style| style.visible()))
                .child(icon_button(("card-inbox-notes", index), "fm/file-text.svg", "В заметки: добавить в первый раздел карточки", false, !busy)
                    .when(!busy, |button| button.on_click(cx.listener(move |this, _, _, cx| this.inbox_to_notes(notes.clone(), cx)))))
                .child(icon_button(("card-inbox-reveal", index), "fm/folder-fill.svg", "Показать в папке входящих", false, true)
                    .on_click(cx.listener(move |this, _, _, cx| this.open_card_link(reveal.clone(), cx))))
                .child(icon_button(("card-inbox-delete", index), "fm/trash.svg", if entry.image.is_some() { "Удалить запись (снимок и подпись)" } else { "Удалить запись" }, doomed, !busy)
                    .when(!busy, |button| button.on_click(cx.listener(move |this, _, _, cx| this.delete_inbox_entry(gone.clone(), cx))))))
    }

    /// The card in place of the pane's address row and file list.
    pub(crate) fn project_card_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(open) = self.cards.open.as_ref() else { return div().into_any_element() };
        let group = open.group;
        let target = open.editing.as_ref().map(|editing| editing.target.clone());
        let editing = match &target { Some(Target::Section(index)) => Some(*index), _ => None };
        let busy = target.is_some();
        let dim = |text: String| div().text_size(px(12.)).text_color(rgb(TEXT_MUTED)).child(text);
        let heading = |title: String| div().flex_1().min_w_0().truncate().text_size(px(14.)).font_weight(gpui::FontWeight::SEMIBOLD).child(title);
        let block = |key: String| div().group(gpui::SharedString::from(key)).flex().flex_col().gap_1().pb_3().border_b_1().border_color(rgb(BORDER));
        // A section's buttons show while the mouse is over it.
        let tools = |key: String| div().flex_none().h(px(28.)).flex().items_center().gap_1().invisible()
            .group_hover(gpui::SharedString::from(key), |style| style.visible());
        let mut page = div().w_full().max_w(px(860.)).flex().flex_col().gap_3();

        // The user's own sections, in their order. Editing happens in place:
        // the heading becomes a field, the text becomes a field, nothing moves.
        for (index, section) in open.card.sections.iter().enumerate() {
            let key = format!("card-section-{index}");
            if editing == Some(index) {
                page = page.child(block(key).child(self.edit_fields(None, true, true, cx)));
                continue;
            }
            let doomed = open.doomed == Some(index);
            let mut body = block(key.clone()).child(div().h(px(28.)).flex().items_center().gap_1()
                .child(heading(section.title.clone()))
                .when(doomed, |row| row.child(div().flex_none().text_size(px(12.)).text_color(rgb(DANGER)).child("Удалить раздел? Нажмите ещё раз")))
                .child(tools(key).when(doomed, |row| row.visible())
                    .child(icon_button(("card-edit", index), "fm/pencil.svg", "Изменить (или щёлкните по тексту)", false, true)
                        .on_click(cx.listener(move |this, _, window, cx| this.edit_card_section(index, false, window, cx))))
                    .child(icon_button(("card-up", index), "fm/arrow-up.svg", "Поднять выше", false, index > 0 && !busy)
                        .when(index > 0 && !busy, |button| button.on_click(cx.listener(move |this, _, _, cx| this.change_card(|card| card.move_up(index), cx)))))
                    .child(icon_button(("card-delete", index), "fm/trash.svg", "Удалить раздел", doomed, !busy)
                        .when(!busy, |button| button.on_click(cx.listener(move |this, _, _, cx| this.delete_card_section(index, cx)))))));
            if section.text.is_empty() {
                body = body.child(div().id(("card-empty", index)).text_size(px(13.)).text_color(rgb(TEXT_DIM)).cursor_text()
                    .child("Пусто — щёлкните, чтобы написать")
                    .on_click(cx.listener(move |this, _, window, cx| this.edit_card_section(index, false, window, cx))));
            }
            for (row, line) in model::read_lines(&section.text).into_iter().enumerate() {
                let id = index * 10_000 + row;
                // A click on plain text edits the section; links open.
                let edit = cx.listener(move |this, _, window, cx| this.edit_card_section(index, false, window, cx));
                body = match line {
                    Line::Blank => body.child(div().h(px(6.))),
                    Line::Text(text) => body.child(div().id(("card-text", id)).text_size(px(13.)).cursor_text().child(text).on_click(edit)),
                    Line::Link(path) => body.child(self.link_row(("card-link", id), &path, None, cx)),
                    Line::Field { label, value, link } => body.child(div().flex().items_start().gap_3().text_size(px(13.))
                        .child(div().w(px(150.)).flex_none().text_color(rgb(TEXT_MUTED)).child(label))
                        .child(match link {
                            Some(path) => self.link_row(("card-field", id), &path, None, cx).flex_1().min_w_0().into_any_element(),
                            None => div().id(("card-value", id)).flex_1().min_w_0().cursor_text().child(value).on_click(edit).into_any_element(),
                        })),
                };
            }
            page = page.child(body);
        }

        // Papka's own sections: hidden with the eye, brought back below.
        let built_in = |key: &'static str, title: &'static str, extra: Option<AnyElement>, cx: &mut Context<Self>| div().h(px(28.)).flex().items_center().gap_1()
            .child(heading(title.into()))
            .child(tools(format!("card-{key}"))
                .when_some(extra, |row, extra| row.child(extra))
                .child(icon_button(gpui::SharedString::from(format!("card-hide-{key}")), "fm/eye-off.svg", "Скрыть раздел", false, !busy)
                    .when(!busy, |button| button.on_click(cx.listener(move |this, _, _, cx| this.change_card(|card| card.set_hidden(key, true), cx))))));
        let published = self.registry.projects.get(&open.name).cloned();
        if !open.card.is_hidden("inbox") {
            let mut body = block("card-inbox".into()).child(built_in("inbox", "Входящие", None, cx));
            match (&published, &open.inbox) {
                (None, _) | (_, None) => body = body.child(dim("Читаются…".into())),
                (Some(_), Some(Err(error))) => body = body.child(div().text_size(px(12.)).text_color(rgb(DANGER)).child(format!("Папка входящих недоступна: {error}"))),
                (Some(published), Some(Ok(entries))) if entries.is_empty() => body = body.child(dim(format!(
                    "Пока пусто. В Notch: снимок, текст или файл → «В проект…» → «{}». Записи попадут в {}.",
                    open.name, published.project.inbox.display()))),
                (Some(_), Some(Ok(entries))) => for (index, entry) in entries.iter().take(40).enumerate() {
                    body = body.child(self.inbox_row(index, entry, busy, cx));
                },
            }
            page = page.child(body);
        }
        if !open.card.is_hidden("folders") {
            let add = div().flex().items_center().gap_1()
                .child(icon_button("card-folder-add", "fm/folder-plus.svg", "Добавить папку по пути", false, !busy)
                    .when(!busy, |button| button.on_click(cx.listener(|this, _, window, cx| this.open_folder_field(window, cx)))))
                .child(icon_button("card-folder-tabs", "fm/plus.svg", "Добавить все открытые вкладки проекта", false, !busy)
                    .when(!busy, |button| button.on_click(cx.listener(|this, _, _, cx| this.add_tabs_to_folders(cx)))))
                .into_any_element();
            let mut body = block("card-folders".into()).child(built_in("folders", "Папки проекта", Some(add), cx));
            let name = open.name.clone();
            if let Some(published) = &published {
                let root_line = match &published.project.root {
                    Some(root) => format!("Папка объекта: {} — {}", root.display(), published.source.label()),
                    None => format!("Папка объекта {}", published.source.label()),
                };
                let chosen = published.source == crate::inbox::RootSource::Chosen;
                body = body.child(div().flex().items_center().gap_2().text_size(px(12.)).text_color(rgb(TEXT_MUTED))
                    .child(div().flex_1().min_w_0().child(root_line))
                    .when(chosen, |row| row.child(div().id("card-root-reset").px_2().rounded_sm().cursor_pointer().text_color(rgb(ACCENT))
                        .hover(|style| style.bg(rgb(HOVER))).child("определять самой")
                        .on_click(cx.listener({ let name = name.clone(); move |this, _, _, cx| this.set_project_root(&name, None, cx) })))));
            }
            if open.card.folders.is_empty() && !open.adding_folder {
                body = body.child(dim("Здесь хранятся папки проекта для быстрого перехода. Добавьте их кнопками справа от заголовка или командой «В папки проекта» в меню папки.".into()));
            }
            for (index, folder) in open.card.folders.iter().enumerate() {
                let is_root = published.as_ref().and_then(|published| published.project.root.as_ref()) == Some(&folder.path);
                let (name, root) = (name.clone(), folder.path.clone());
                let row_key = format!("card-folder-row-{index}");
                let title = (!folder.title.is_empty()).then(|| folder.title.clone());
                body = body.child(div().group(gpui::SharedString::from(row_key.clone())).flex().items_center().gap_1()
                    .child(self.link_row(("card-folder", index), &folder.path, title, cx).flex_1().min_w_0())
                    .when(is_root, |row| row.child(div().flex_none().text_size(px(11.)).text_color(rgb(TEXT_DIM)).child("папка объекта")))
                    .child(tools(row_key)
                        .when(!is_root, |row| row.child(icon_button(("card-root", index), "fm/home.svg", "Сделать папкой объекта: сюда будут приходить входящие", false, !busy)
                            .when(!busy, |button| button.on_click(cx.listener(move |this, _, _, cx| this.set_project_root(&name, Some(root.clone()), cx))))))
                        .child(icon_button(("card-folder-up", index), "fm/arrow-up.svg", "Выше", false, index > 0 && !busy)
                            .when(index > 0 && !busy, |button| button.on_click(cx.listener(move |this, _, _, cx| { this.change_card(|card| card.move_folder_up(index), cx); }))))
                        .child(icon_button(("card-folder-remove", index), "fm/x.svg", "Убрать из списка (папка остаётся на диске)", false, !busy)
                            .when(!busy, |button| button.on_click(cx.listener(move |this, _, _, cx| {
                                this.change_card(|card| card.remove_folder(index), cx);
                                this.registry.due = true;
                            }))))));
            }
            if open.adding_folder {
                body = body.child(div().flex().items_center().gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.cards.folder_input)))
                    .child(text_button("card-folder-cancel", "Отмена", ButtonKind::Ghost)
                        .on_click(cx.listener(|this, _, _, cx| { if let Some(open) = this.cards.open.as_mut() { open.adding_folder = false; this.interface_focus_pending = true; } cx.notify(); })))
                    .child(text_button("card-folder-save", "Добавить", ButtonKind::Primary)
                        .on_click(cx.listener(|this, _, _, cx| this.add_typed_folder(cx)))));
            }
            page = page.child(body);
        }
        // The object passport: Papka keeps it (no calculation program does
        // yet). Everything here is edited in place, like the sections above.
        if !open.card.is_hidden("passport") {
            let found = matches!(open.passport, PassportState::Found { .. });
            let tools_extra = found.then(|| div().flex().items_center().gap_1()
                .child(icon_button("card-passport-add", "fm/plus.svg", "Добавить блок паспорта", false, !busy)
                    .when(!busy, |button| button.on_click(cx.listener(|this, _, window, cx|
                        this.begin_edit(Target::NewPassportBlock, true, String::new(), String::new(), window, cx)))))
                .child(icon_button("card-passport-warn", "fm/alert.svg", "Предупреждения: что требует внимания", false, !busy)
                    .when(!busy, |button| button.on_click(cx.listener(|this, _, window, cx| {
                        let text = match this.cards.open.as_ref().map(|open| &open.passport) {
                            Some(PassportState::Found { passport, .. }) => passport.warnings.join("\n"),
                            _ => String::new(),
                        };
                        this.begin_edit(Target::PassportWarnings, false, "Предупреждения".into(), text, window, cx);
                    }))))
                .into_any_element());
            let mut body = block("card-passport".into()).child(built_in("passport", "Паспорт объекта", tools_extra, cx));
            match &open.passport {
                PassportState::Loading => body = body.child(dim("Читается…".into())),
                PassportState::Absent => {
                    let root = published.as_ref().and_then(|published| published.project.root.clone());
                    body = body.child(div().flex().items_center().gap_3()
                        .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(rgb(TEXT_MUTED)).child(match &root {
                            Some(root) => format!("Паспорта ещё нет. Он будет лежать в {} — общий для всех, кто работает с объектом.", root.join(passport::FILE_NAME).display()),
                            None => "Паспорта ещё нет. Сначала укажите папку объекта — домик у папки в «Папках проекта»: паспорт ляжет туда.".into(),
                        }))
                        .when(root.is_some(), |row| row.child(text_button("card-passport-create", "Создать паспорт", ButtonKind::Primary)
                            .on_click(cx.listener(|this, _, _, cx| this.create_passport(cx))))));
                }
                PassportState::Unreadable { file, error } => {
                    body = body.child(div().text_size(px(12.)).text_color(rgb(DANGER)).child(format!("Не прочитан: {error}")))
                        .child(self.link_row("card-passport-file", file, None, cx));
                }
                PassportState::Found { passport, file, page: html, .. } => {
                    // The object's name: a click renames it.
                    if target == Some(Target::PassportTitle) {
                        body = body.child(self.edit_fields(None, true, false, cx));
                    } else {
                        let name = passport.title.clone().unwrap_or_default();
                        body = body.child(div().flex().items_baseline().gap_2().text_size(px(13.))
                            .child(div().id("card-passport-title").flex_1().min_w_0().font_weight(gpui::FontWeight::SEMIBOLD).cursor_text()
                                .text_color(rgb(if name.is_empty() { TEXT_DIM } else { TEXT }))
                                .child(if name.is_empty() { "Название объекта — щёлкните, чтобы написать".to_string() } else { name.clone() })
                                .on_click(cx.listener(move |this, _, window, cx| this.begin_edit(Target::PassportTitle, false, name.clone(), String::new(), window, cx))))
                            .when_some(passport.updated.clone(), |row, updated| row.child(div().flex_none().text_size(px(11.)).text_color(rgb(TEXT_DIM))
                                .child(format!("обновлено {updated}")))));
                    }
                    if target == Some(Target::PassportWarnings) {
                        body = body.child(self.edit_fields(Some("Предупреждения — одно в строке".into()), false, true, cx));
                    } else {
                        let warnings_text = Arc::new(passport.warnings.join("\n"));
                        for (index, warning) in passport.warnings.iter().take(60).enumerate() {
                            let text = warnings_text.clone();
                            body = body.child(div().id(("card-warning", index)).flex().items_start().gap_2().px_2().py_1().rounded(px(4.)).bg(rgb(ACCENT_SOFT)).cursor_text()
                                .child(div().mt(px(3.)).child(small_icon("fm/alert.svg", DANGER)))
                                .child(div().flex_1().min_w_0().text_size(px(12.)).child(display_text(warning)))
                                .on_click(cx.listener(move |this, _, window, cx| this.begin_edit(Target::PassportWarnings, false, "Предупреждения".into(), text.as_ref().clone(), window, cx))));
                        }
                        if passport.warnings.len() > 60 { body = body.child(dim(format!("Ещё {} предупреждений — откройте редактор", passport.warnings.len() - 60))); }
                    }
                    let mut row_id = 0usize;
                    for section in &passport.sections {
                        let key = section.key.clone();
                        if target == Some(Target::PassportBlock(key.clone())) {
                            body = body.child(div().mt_1().child(self.edit_fields(None, true, true, cx)));
                            continue;
                        }
                        let doomed = open.doomed_block.as_ref() == Some(&key);
                        let group_key = format!("card-pblock-{row_id}");
                        let (title, text) = (section.title.clone(), Arc::new(passport::block_text(section)));
                        let (edit_key, edit_title, edit_text, gone) = (key.clone(), title.clone(), text.clone(), key.clone());
                        let start = cx.listener(move |this, _, window, cx|
                            this.begin_edit(Target::PassportBlock(edit_key.clone()), false, edit_title.clone(), edit_text.as_ref().clone(), window, cx));
                        let mut part = div().group(gpui::SharedString::from(group_key.clone())).flex().flex_col().gap_1()
                            .child(div().h(px(24.)).mt_1().flex().items_center().gap_1()
                                .child(div().flex_1().min_w_0().text_size(px(11.)).text_color(rgb(TEXT_DIM)).child(section.title.to_uppercase()))
                                .when(doomed, |row| row.child(div().flex_none().text_size(px(12.)).text_color(rgb(DANGER)).child("Удалить блок? Нажмите ещё раз")))
                                .child(tools(group_key).when(doomed, |row| row.visible())
                                    .child(icon_button(("card-pblock-edit", row_id), "fm/pencil.svg", "Изменить блок (или щёлкните по нему)", false, !busy)
                                        .when(!busy, |button| button.on_click(start)))
                                    .child(icon_button(("card-pblock-delete", row_id), "fm/trash.svg", "Удалить блок", doomed, !busy)
                                        .when(!busy, |button| button.on_click(cx.listener(move |this, _, _, cx| this.delete_passport_block(gone.clone(), cx)))))));
                        if section.rows.is_empty() {
                            let (k, t) = (key.clone(), title.clone());
                            part = part.child(div().id(("card-pblock-empty", row_id)).text_size(px(12.)).text_color(rgb(TEXT_DIM)).cursor_text()
                                .child("Пусто — щёлкните, чтобы заполнить")
                                .on_click(cx.listener(move |this, _, window, cx| this.begin_edit(Target::PassportBlock(k.clone()), false, t.clone(), String::new(), window, cx))));
                        }
                        for (label, value) in section.rows.iter().take(60) {
                            row_id += 1;
                            let (k, t, x) = (key.clone(), title.clone(), text.clone());
                            let edit = cx.listener(move |this, _, window, cx| this.begin_edit(Target::PassportBlock(k.clone()), false, t.clone(), x.as_ref().clone(), window, cx));
                            // A model or base file named in the passport opens like any link.
                            let value_view = match passport_link(file, value) {
                                Some(path) => {
                                    let rest = value.split_once(" · ").map(|(_, rest)| rest.to_string());
                                    div().flex_1().min_w_0().flex().items_center().gap_2()
                                        .child(self.link_row(("card-passport-link", row_id), &path, None, cx).flex_none().max_w(px(420.)))
                                        .when_some(rest, |row, rest| row.child(div().min_w_0().truncate().child(rest)))
                                        .into_any_element()
                                }
                                // An empty field shows a dash to click and fill.
                                None => div().id(("card-pvalue", row_id)).flex_1().min_w_0().cursor_text()
                                    .when(value.is_empty(), |cell| cell.text_color(rgb(TEXT_DIM)))
                                    .child(if value.is_empty() { "—".to_string() } else { display_text(value) }).on_click(edit).into_any_element(),
                            };
                            part = part.child(div().flex().items_start().gap_3().text_size(px(12.))
                                .child(div().w(px(150.)).flex_none().text_color(rgb(TEXT_MUTED)).child(label.clone()))
                                .child(value_view));
                        }
                        if section.rows.len() > 60 { part = part.child(dim(format!("Ещё {} строк — откройте редактор блока", section.rows.len() - 60))); }
                        row_id += 1;
                        body = body.child(part);
                    }
                    if target == Some(Target::NewPassportBlock) {
                        body = body.child(div().mt_1().child(self.edit_fields(None, true, true, cx)));
                    }
                    // The journal: what was done and when, newest last.
                    body = body.child(div().h(px(24.)).mt_1().flex().items_center().gap_1()
                        .child(div().flex_1().text_size(px(11.)).text_color(rgb(TEXT_DIM)).child("ЖУРНАЛ")));
                    if target == Some(Target::PassportJournal) {
                        body = body.child(self.edit_fields(Some("Журнал — запись в строке".into()), false, true, cx));
                    } else {
                        let shown = passport.journal.len().saturating_sub(12);
                        if shown > 0 { body = body.child(dim(format!("…ещё {shown} раньше"))); }
                        let all = Arc::new(passport.journal.join("\n"));
                        for (index, line) in passport.journal.iter().enumerate().skip(shown) {
                            let all = all.clone();
                            body = body.child(div().id(("card-journal", index)).text_size(px(12.)).cursor_text().child(display_text(line))
                                .on_click(cx.listener(move |this, _, window, cx| this.begin_edit(Target::PassportJournal, false, "Журнал".into(), all.as_ref().clone(), window, cx))));
                        }
                        body = body.child(div().text_size(px(12.)).child(Input::new(&self.cards.journal_input)));
                    }
                    body = body.child(div().flex().items_center().gap_2().mt_1()
                        .child(self.link_row("card-passport-file", file, None, cx).flex_1().min_w_0())
                        .when_some(html.clone(), |row, html| row.child(text_button("card-passport-page", "Страница паспорта", ButtonKind::Ghost)
                            .tooltip(|window, cx| Tooltip::new("Открыть object.html в браузере").build(window, cx))
                            .on_click(cx.listener(move |this, _, _, cx| this.open_outside(&html, cx))))));
                }
            }
            page = page.child(body);
        }
        if !open.card.is_hidden("changes") {
            let mut body = block("card-changes".into()).child(built_in("changes", "Последние изменения", None, cx));
            match &open.changes {
                None => body = body.child(dim("Читаются…".into())),
                Some(changes) if changes.is_empty() => body = body.child(dim(format!(
                    "За {DAYS} дней изменений не замечено. Papka видит изменения в папках, пока они открыты в ней."))),
                Some(changes) => for (index, event) in changes.iter().enumerate() {
                    let path = event.path.clone();
                    let full = event.path.display().to_string();
                    let gone = event.kind == "missing";
                    body = body.child(div().id(("card-change", index)).h(px(24.)).flex_none().px_1().flex().items_center().gap_2().rounded(px(4.))
                        .cursor_pointer().hover(|style| style.bg(rgb(HOVER))).text_size(px(12.))
                        .child(div().w(px(110.)).flex_none().text_color(rgb(TEXT_DIM)).child(format!("{} {}", event.day_label(), event.clock_label())))
                        .child(div().flex_1().min_w_0().truncate().child(browser::display_name(&event.path)))
                        .child(div().flex_none().text_color(rgb(if gone { DANGER } else { ACCENT })).child(event.feed_label().to_string()))
                        .tooltip(move |window, cx| Tooltip::new(format!("Открыть в новой вкладке: {full}")).build(window, cx))
                        .on_click(cx.listener(move |this, _, _, cx| this.open_card_link(path.clone(), cx))));
                },
            }
            page = page.child(body);
        }
        let hidden: Vec<(&'static str, &'static str)> = model::BUILT_IN.into_iter()
            .filter(|(key, _)| open.card.is_hidden(key)).collect();
        if !hidden.is_empty() && !busy {
            let mut row = div().flex().flex_wrap().items_center().gap_2().text_size(px(12.)).text_color(rgb(TEXT_DIM)).child("Скрыто:");
            for (key, title) in hidden {
                row = row.child(div().id(gpui::SharedString::from(format!("card-show-{key}"))).px_2().rounded_sm().cursor_pointer().text_color(rgb(ACCENT))
                    .hover(|style| style.bg(rgb(HOVER))).child(format!("вернуть «{title}»"))
                    .on_click(cx.listener(move |this, _, _, cx| this.change_card(|card| card.set_hidden(key, false), cx))));
            }
            page = page.child(row);
        }

        div().size_full().min_h_0().flex().flex_col().bg(rgb(SURFACE))
            .child(div().h(px(36.)).flex_none().px_3().flex().items_center().gap_2().border_b_1().border_color(rgb(BORDER))
                .child(small_icon("fm/layers.svg", ACCENT))
                .child(div().flex_1().min_w_0().truncate().font_weight(gpui::FontWeight::SEMIBOLD).child(format!("Проект «{}»", open.name)))
                .child(text_button("card-add", "Добавить раздел", ButtonKind::Ghost)
                    .on_click(cx.listener(|this, _, window, cx| this.add_card_section(window, cx))))
                .child(icon_button("card-close", "fm/x.svg", "Вернуться к папкам (Esc)", false, true)
                    .on_click(cx.listener(|this, _, _, cx| this.close_project_card(cx)))))
            .child(div().id(("project-card-page", group.0 as usize)).flex_1().min_h_0().overflow_y_scroll().p_4().child(page))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: i64, observed_ms: i64) -> Event {
        Event { id, path: format!("f{id}").into(), kind: "modified".into(), observed_ms, recorded_by: String::new(), author: None, comment: String::new() }
    }

    #[test]
    fn changes_of_all_folders_come_newest_first_without_repeats() {
        // A tab and its parent folder both report event 2.
        let merged = merge_changes(vec![vec![event(2, 200), event(1, 100)], vec![event(3, 300), event(2, 200)], Vec::new()], 10);
        assert_eq!(merged.iter().map(|event| event.id).collect::<Vec<_>>(), [3, 2, 1]);
        assert_eq!(merge_changes(vec![vec![event(1, 100), event(2, 200), event(3, 300)]], 2).iter().map(|event| event.id).collect::<Vec<_>>(), [3, 2]);
        assert!(merge_changes(Vec::new(), 5).is_empty());
    }

    #[test]
    fn files_named_in_the_passport_become_links() {
        let passport = Path::new("/srv/044/object.json");
        assert_eq!(passport_link(passport, "lira/ostrov.txt · м, кН · Изменён: 03.11"), Some(PathBuf::from("/srv/044/lira/ostrov.txt")));
        assert_eq!(passport_link(passport, "/srv/base/ostrov.sqlite"), Some(PathBuf::from("/srv/base/ostrov.sqlite")));
        for text in ["ostrov.sqlite", "м, кН", "14.10", "C1 ±6 % к №1", "https://example.com/a.html", "ЛИРА 2024/2", "a b/c.txt"] {
            assert_eq!(passport_link(passport, text), None, "{text}");
        }
    }
}
