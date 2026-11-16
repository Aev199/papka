//! The project card: the project's own page, shown in its pane in place of
//! the file list. It opens from the project's name at the head of the tab
//! strip. The user's sections (free text: notes, fields, links to folders)
//! are edited right there and saved with the project; three more sections
//! are filled in by Papka — the project's folders, the object passport
//! (`object.json`, read only) and the latest changes — and can be hidden.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use filemanager_core::browser;
use filemanager_core::history::{self, Event};
use filemanager_core::panes::GroupId;
use filemanager_core::passport::{self, Passport};
use filemanager_core::project_card::{self as model, Card, Line};
use gpui::{AnyElement, AppContext, Context, Div, ElementId, Entity, Focusable, Stateful, Window, div, prelude::*, px};
use gpui_component::input::{Input, InputState, Textarea, TextareaState};
use gpui_component::tooltip::Tooltip;

use crate::Explorer;
use crate::theme::*;
use crate::view::{ButtonKind, icon_button, small_icon, text_button};

/// Changes are read this far back and this many are shown.
const DAYS: i64 = 30;
const CHANGES: usize = 12;

#[derive(Default)]
pub(crate) enum PassportState {
    #[default]
    Loading,
    /// No `object.json` in the project's folders or above them.
    Absent,
    Found { file: PathBuf, page: Option<PathBuf>, passport: Passport },
    Unreadable { file: PathBuf, error: String },
}

/// The section being edited.
pub(crate) struct Editing {
    index: usize,
    /// Just added: cancelling removes it again.
    fresh: bool,
}

pub(crate) struct OpenCard {
    pub(crate) group: GroupId,
    pub(crate) name: String,
    card: Card,
    editing: Option<Editing>,
    /// The section whose bin was clicked once: the second click deletes.
    doomed: Option<usize>,
    passport: PassportState,
    /// `None` while they are being read.
    changes: Option<Vec<Event>>,
    /// What each link of the card is on disk: a folder, a file, or (None)
    /// not found. Read off the window's thread; a link not looked at yet
    /// is drawn by its name.
    kinds: HashMap<PathBuf, Option<bool>>,
    request: u64,
}

pub(crate) struct ProjectCards {
    pub(crate) open: Option<OpenCard>,
    title_input: Entity<InputState>,
    text_input: Entity<TextareaState>,
    requests: u64,
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
        ProjectCards {
            open: None,
            title_input: cx.new(|cx| InputState::new(window, cx).placeholder("Название раздела")),
            text_input: cx.new(|cx| TextareaState::new(window, cx).auto_grow(4, 18)
                .placeholder("Свободный текст. «Метка: значение» станет полем, путь к папке — ссылкой.")),
            requests: 0,
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
        self.cards.open = Some(OpenCard { group, name, card: Card::load(saved.as_deref()), editing: None, doomed: None,
            passport: PassportState::Loading, changes: None, kinds: HashMap::new(), request: self.cards.requests });
        self.context_menu = None;
        self.focus_group(group, cx);
        // Nothing stays chosen in the list hidden under the card.
        self.selected = None; self.marked.clear(); self.anchor = None; self.marquee = None;
        self.confirm_recycle = None; self.address_editing = None; self.filter_open = false;
        self.close_search();
        self.reload_project_card(cx);
        cx.notify();
    }

    /// Reads the passport and the changes again. Everything on disk (a
    /// passport on a network drive, the journal) is read off the window's
    /// thread; the card is on screen meanwhile.
    fn reload_project_card(&mut self, cx: &mut Context<Self>) {
        let Some(group) = self.cards.open.as_ref().map(|open| open.group) else { return };
        let paths = self.project_folders(group).into_iter().map(|(_, path)| path).collect::<Vec<_>>();
        self.cards.requests = self.cards.requests.wrapping_add(1);
        let request = self.cards.requests;
        let Some(open) = self.cards.open.as_mut() else { return };
        open.request = request;
        open.passport = PassportState::Loading;
        open.changes = None;
        self.look_at_card_links(cx);
        let journal = self.journal.clone();
        let task = cx.background_spawn(async move {
            let passport = match passport::find(&paths) {
                None => PassportState::Absent,
                Some(file) => match passport::read(&file) {
                    Ok(passport) => PassportState::Found { page: passport::page_of(&file), file, passport },
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
                open.passport = passport;
                open.changes = Some(changes);
                cx.notify();
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
                open.kinds.extend(kinds);
                cx.notify();
            });
        }).detach();
    }

    /// The project's tabs: title and folder.
    fn project_folders(&self, group: GroupId) -> Vec<(String, PathBuf)> {
        self.browser.group(group).map(|pane| pane.tabs.iter().map(|tab| (tab.title.clone(), tab.pane.path.clone())).collect())
            .unwrap_or_default()
    }

    /// Gives the pane back to its folders. Text being edited is kept: a
    /// click on a tab must not lose a note.
    pub(crate) fn close_project_card(&mut self, cx: &mut Context<Self>) {
        if self.cards.open.as_ref().is_some_and(|open| open.editing.is_some()) { self.save_card_section(cx); }
        if self.cards.open.take().is_some() { cx.notify(); }
    }

    /// The pane moves on (another tab, another folder): its card closes.
    pub(crate) fn leave_project_card(&mut self, group: GroupId, cx: &mut Context<Self>) {
        if self.cards.shown_in(group) { self.close_project_card(cx); }
    }

    /// Esc: out of the edit first, then out of the card. Whether it did anything.
    pub(crate) fn dismiss_project_card(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(open) = self.cards.open.as_mut() else { return false };
        if open.doomed.take().is_some() { cx.notify(); return true; }
        match open.editing.take() {
            Some(editing) => {
                if editing.fresh { open.card.remove(editing.index); }
                cx.notify();
            }
            None => self.close_project_card(cx),
        }
        true
    }

    fn store_card(&mut self) {
        let Some(open) = &self.cards.open else { return };
        let result = self.workspaces.as_ref().ok_or_else(|| std::io::Error::other("база настроек недоступна"))
            .and_then(|store| store.set_value(&model::storage_key(&open.name), &open.card.to_json()));
        if let Err(error) = result { self.status = format!("Карточка проекта не сохранена: {error}"); }
    }

    fn edit_card_section(&mut self, index: usize, fresh: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.cards.open.as_ref().is_some_and(|open| open.editing.is_some()) { self.save_card_section(cx); }
        let Some(open) = self.cards.open.as_mut() else { return };
        let Some(section) = open.card.sections.get(index) else { return };
        // A new section starts with an empty name to type over.
        let (title, text) = (if fresh { String::new() } else { section.title.clone() }, section.text.clone());
        open.editing = Some(Editing { index, fresh });
        open.doomed = None;
        self.cards.title_input.update(cx, |input, cx| input.set_value(title, window, cx));
        self.cards.text_input.update(cx, |input, cx| input.set_value(text, window, cx));
        let focus = if fresh { self.cards.title_input.focus_handle(cx) } else { self.cards.text_input.focus_handle(cx) };
        window.focus(&focus, cx);
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
        open.card.set(editing.index, &title, &text);
        self.store_card();
        self.look_at_card_links(cx);
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
                if let Some(file) = select { this.navigate_side(group, &folder, Some(file), cx); }
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

    /// The card in place of the pane's address row and file list.
    pub(crate) fn project_card_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(open) = self.cards.open.as_ref() else { return div().into_any_element() };
        let group = open.group;
        let editing = open.editing.as_ref().map(|editing| editing.index);
        let busy = editing.is_some();
        let dim = |text: String| div().text_size(px(12.)).text_color(rgb(TEXT_MUTED)).child(text);
        let heading = |title: String| div().flex_1().min_w_0().truncate().text_size(px(14.)).font_weight(gpui::FontWeight::SEMIBOLD).child(title);
        let block = || div().flex().flex_col().gap_1().pb_3().border_b_1().border_color(rgb(BORDER));
        let mut page = div().w_full().max_w(px(860.)).flex().flex_col().gap_3();

        // The user's own sections, in their order.
        for (index, section) in open.card.sections.iter().enumerate() {
            if editing == Some(index) {
                page = page.child(block()
                    .child(Input::new(&self.cards.title_input))
                    .child(Textarea::new(&self.cards.text_input))
                    .child(div().flex().items_center().gap_2()
                        .child(div().flex_1().min_w_0().text_size(px(11.)).text_color(rgb(TEXT_DIM))
                            .child("Строка «Метка: значение» станет полем, путь к папке или файлу — ссылкой."))
                        .child(text_button("card-cancel", "Отмена", ButtonKind::Ghost)
                            .on_click(cx.listener(|this, _, _, cx| { this.dismiss_project_card(cx); })))
                        .child(text_button("card-save", "Сохранить", ButtonKind::Primary)
                            .on_click(cx.listener(|this, _, _, cx| this.save_card_section(cx))))));
                continue;
            }
            let doomed = open.doomed == Some(index);
            let mut body = block().child(div().flex().items_center().gap_1()
                .child(heading(section.title.clone()))
                .when(doomed, |row| row.child(div().flex_none().text_size(px(12.)).text_color(rgb(DANGER)).child("Удалить раздел? Нажмите ещё раз")))
                .child(icon_button(("card-edit", index), "fm/pencil.svg", "Изменить раздел", false, true)
                    .on_click(cx.listener(move |this, _, window, cx| this.edit_card_section(index, false, window, cx))))
                .child(icon_button(("card-up", index), "fm/arrow-up.svg", "Поднять выше", false, index > 0 && !busy)
                    .when(index > 0 && !busy, |button| button.on_click(cx.listener(move |this, _, _, cx| this.change_card(|card| card.move_up(index), cx)))))
                .child(icon_button(("card-delete", index), "fm/trash.svg", "Удалить раздел", doomed, !busy)
                    .when(!busy, |button| button.on_click(cx.listener(move |this, _, _, cx| this.delete_card_section(index, cx))))));
            if section.text.is_empty() {
                body = body.child(div().id(("card-empty", index)).text_size(px(12.)).text_color(rgb(TEXT_DIM)).cursor_pointer()
                    .child("Пусто — щёлкните, чтобы написать")
                    .on_click(cx.listener(move |this, _, window, cx| this.edit_card_section(index, false, window, cx))));
            }
            for (row, line) in model::read_lines(&section.text).into_iter().enumerate() {
                let id = index * 10_000 + row;
                body = match line {
                    Line::Blank => body.child(div().h(px(6.))),
                    Line::Text(text) => body.child(div().text_size(px(13.)).child(text)),
                    Line::Link(path) => body.child(self.link_row(("card-link", id), &path, None, cx)),
                    Line::Field { label, value, link } => body.child(div().flex().items_start().gap_3().text_size(px(13.))
                        .child(div().w(px(150.)).flex_none().text_color(rgb(TEXT_MUTED)).child(label))
                        .child(match link {
                            Some(path) => self.link_row(("card-field", id), &path, None, cx).flex_1().min_w_0().into_any_element(),
                            None => div().flex_1().min_w_0().child(value).into_any_element(),
                        })),
                };
            }
            page = page.child(body);
        }

        // Papka's own sections: hidden with the eye, brought back below.
        let built_in = |key: &'static str, title: &'static str, cx: &mut Context<Self>| div().flex().items_center().gap_1()
            .child(heading(title.into()))
            .child(icon_button(gpui::SharedString::from(format!("card-hide-{key}")), "fm/eye-off.svg", "Скрыть раздел", false, !busy)
                .when(!busy, |button| button.on_click(cx.listener(move |this, _, _, cx| this.change_card(|card| card.set_hidden(key, true), cx)))));
        if !open.card.is_hidden("folders") {
            let mut body = block().child(built_in("folders", "Папки проекта", cx));
            for (index, (title, path)) in self.project_folders(group).into_iter().enumerate() {
                body = body.child(self.link_row(("card-folder", index), &path, Some(title), cx));
            }
            page = page.child(body);
        }
        if !open.card.is_hidden("passport") {
            let mut body = block().child(built_in("passport", "Паспорт объекта", cx));
            match &open.passport {
                PassportState::Loading => body = body.child(dim("Читается…".into())),
                PassportState::Absent => body = body.child(dim(format!(
                    "В папках проекта и до пяти уровней выше нет файла {}. Его ведут расчётные программы; Papka только показывает.",
                    passport::FILE_NAME))),
                PassportState::Unreadable { file, error } => {
                    body = body.child(div().text_size(px(12.)).text_color(rgb(DANGER)).child(format!("Не прочитан: {error}")))
                        .child(self.link_row("card-passport-file", file, None, cx));
                }
                PassportState::Found { passport, file, page: html } => {
                    if passport.title.is_some() || passport.updated.is_some() {
                        body = body.child(div().flex().items_baseline().gap_2().text_size(px(13.))
                            .child(div().flex_1().min_w_0().font_weight(gpui::FontWeight::SEMIBOLD).child(passport.title.clone().unwrap_or_default()))
                            .when_some(passport.updated.clone(), |row, updated| row.child(div().flex_none().text_size(px(11.)).text_color(rgb(TEXT_DIM))
                                .child(format!("обновлено {updated}")))));
                    }
                    for warning in &passport.warnings {
                        body = body.child(div().flex().items_start().gap_2().px_2().py_1().rounded(px(4.)).bg(rgb(ACCENT_SOFT))
                            .child(div().mt(px(3.)).child(small_icon("fm/alert.svg", DANGER)))
                            .child(div().flex_1().min_w_0().text_size(px(12.)).child(warning.clone())));
                    }
                    for section in &passport.sections {
                        body = body.child(div().mt_1().text_size(px(11.)).text_color(rgb(TEXT_DIM)).child(section.title.to_uppercase()));
                        for (label, value) in &section.rows {
                            body = body.child(div().flex().items_start().gap_3().text_size(px(12.))
                                .child(div().w(px(150.)).flex_none().text_color(rgb(TEXT_MUTED)).child(label.clone()))
                                .child(div().flex_1().min_w_0().child(value.clone())));
                        }
                    }
                    if !passport.journal.is_empty() {
                        body = body.child(div().mt_1().text_size(px(11.)).text_color(rgb(TEXT_DIM)).child("ЖУРНАЛ РАСЧЁТОВ"));
                        for line in passport.journal.iter().take(12) { body = body.child(div().text_size(px(12.)).child(line.clone())); }
                        if passport.journal.len() > 12 { body = body.child(dim(format!("…и ещё {}", passport.journal.len() - 12))); }
                    }
                    body = body.child(self.link_row("card-passport-file", file, None, cx));
                    if let Some(html) = html.clone() {
                        body = body.child(div().flex().child(text_button("card-passport-page", "Открыть страницу паспорта в браузере", ButtonKind::Ghost)
                            .on_click(cx.listener(move |this, _, _, cx| this.open_outside(&html, cx)))));
                    }
                }
            }
            page = page.child(body);
        }
        if !open.card.is_hidden("changes") {
            let mut body = block().child(built_in("changes", "Последние изменения", cx));
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
        let hidden: Vec<(&'static str, &'static str)> = model::BUILT_IN.into_iter().filter(|(key, _)| open.card.is_hidden(key)).collect();
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
                .child(icon_button("card-refresh", "fm/refresh.svg", "Прочитать паспорт и изменения заново", false, true)
                    .on_click(cx.listener(|this, _, _, cx| this.reload_project_card(cx))))
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
}
