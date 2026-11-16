//! The project card: what a project is about, on one page. Opens from the
//! project's name at the head of its tab strip. It shows the project's
//! folders, the object passport (`object.json`) found in or above them —
//! read only, the calculation programs own it — and the latest changes
//! Papka saw in those folders.

use std::path::PathBuf;

use filemanager_core::browser;
use filemanager_core::history::{self, Event};
use filemanager_core::panes::GroupId;
use filemanager_core::passport::{self, Passport};
use gpui::{AnimationExt, AnyElement, Context, MouseButton, Window, div, prelude::*, px};
use gpui_component::tooltip::Tooltip;

use crate::Explorer;
use crate::theme::*;
use crate::view::{ButtonKind, icon_button, small_icon, text_button};

/// Changes are read this far back and this many are shown.
const DAYS: i64 = 30;
const CHANGES: usize = 12;

pub(crate) struct Folder {
    pub(crate) title: String,
    pub(crate) path: PathBuf,
    pub(crate) pinned: bool,
}

#[derive(Default)]
pub(crate) enum PassportState {
    #[default]
    Loading,
    /// No `object.json` in the project's folders or above them.
    Absent,
    Found { file: PathBuf, page: Option<PathBuf>, passport: Passport },
    Unreadable { file: PathBuf, error: String },
}

pub(crate) struct ProjectCard {
    pub(crate) group: GroupId,
    pub(crate) name: String,
    pub(crate) folders: Vec<Folder>,
    pub(crate) passport: PassportState,
    /// `None` while they are being read.
    pub(crate) changes: Option<Vec<Event>>,
    request: u64,
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

impl Explorer {
    /// Opens the card of the project shown in pane `group`. Everything on
    /// disk (a passport on a network drive, the journal) is read off the
    /// window's thread; the card opens at once and fills in.
    pub(crate) fn open_project_card(&mut self, group: GroupId, cx: &mut Context<Self>) {
        let Some(pane) = self.browser.group(group) else { return };
        let Some(project) = pane.project.as_ref() else { return };
        let folders: Vec<Folder> = pane.tabs.iter()
            .map(|tab| Folder { title: tab.title.clone(), path: tab.pane.path.clone(), pinned: tab.pinned }).collect();
        let request = self.project_card.as_ref().map_or(1, |card| card.request.wrapping_add(1));
        let paths: Vec<PathBuf> = folders.iter().map(|folder| folder.path.clone()).collect();
        self.project_card = Some(ProjectCard { group, name: project.name.clone(), folders, passport: PassportState::Loading, changes: None, request });
        self.context_menu = None;
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
                let Some(card) = this.project_card.as_mut().filter(|card| card.request == request) else { return };
                card.passport = passport;
                card.changes = Some(changes);
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(crate) fn close_project_card(&mut self, cx: &mut Context<Self>) {
        self.project_card = None;
        cx.notify();
    }

    fn open_outside(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        if let Err(error) = open::that(path) { self.status = format!("Не удалось открыть «{}»: {error}", browser::display_name(path)); }
        cx.notify();
    }

    pub(crate) fn project_card_view(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = self.project_card.as_ref()?;
        let group = card.group;
        let caption = |text: &str| div().mt_1().text_size(px(11.)).text_color(rgb(TEXT_DIM)).child(text.to_uppercase());
        let dim = |text: String| div().text_size(px(12.)).text_color(rgb(TEXT_MUTED)).child(text);
        let max_height = window.viewport_size().height - px(96.);
        let mut body = div().id("project-card-body").flex_1().min_h_0().overflow_y_scroll().px_4().pb_3().flex().flex_col().gap_1();

        // The passport: its own name and date first, then what needs attention.
        match &card.passport {
            PassportState::Loading => body = body.child(dim("Паспорт объекта читается…".into())),
            PassportState::Absent => body = body.child(dim(format!(
                "Паспорта объекта нет: в папках проекта и до пяти уровней выше не найден файл {}.", passport::FILE_NAME))),
            PassportState::Unreadable { file, error } => {
                body = body.child(div().text_size(px(12.)).text_color(rgb(DANGER)).child(format!("Паспорт объекта не прочитан: {error}")))
                    .child(dim(file.display().to_string()));
            }
            PassportState::Found { passport, .. } => {
                if let Some(title) = &passport.title {
                    body = body.child(div().flex().items_baseline().gap_2()
                        .child(div().flex_1().min_w_0().font_weight(gpui::FontWeight::SEMIBOLD).child(title.clone()))
                        .when_some(passport.updated.clone(), |row, updated| row.child(div().flex_none().text_size(px(11.)).text_color(rgb(TEXT_DIM))
                            .child(format!("обновлено {updated}")))));
                } else if let Some(updated) = &passport.updated {
                    body = body.child(dim(format!("Паспорт обновлён {updated}")));
                }
                for warning in &passport.warnings {
                    body = body.child(div().flex().items_start().gap_2().px_2().py_1().rounded(px(4.)).bg(rgb(ACCENT_SOFT))
                        .child(div().mt(px(3.)).child(small_icon("fm/alert.svg", DANGER)))
                        .child(div().flex_1().min_w_0().text_size(px(12.)).child(warning.clone())));
                }
                for section in &passport.sections {
                    body = body.child(caption(&section.title));
                    for (label, value) in &section.rows {
                        body = body.child(div().flex().items_start().gap_3().text_size(px(12.))
                            .child(div().w(px(120.)).flex_none().text_color(rgb(TEXT_MUTED)).child(label.clone()))
                            .child(div().flex_1().min_w_0().child(value.clone())));
                    }
                }
                if !passport.journal.is_empty() {
                    body = body.child(caption("Журнал расчётов"));
                    for line in passport.journal.iter().take(12) { body = body.child(div().text_size(px(12.)).child(line.clone())); }
                    if passport.journal.len() > 12 { body = body.child(dim(format!("…и ещё {}", passport.journal.len() - 12))); }
                }
            }
        }

        body = body.child(caption("Папки проекта"));
        for (index, folder) in card.folders.iter().enumerate() {
            let full = folder.path.display().to_string();
            body = body.child(div().id(("project-folder", index)).h(px(26.)).flex_none().px_1().flex().items_center().gap_2().rounded(px(4.))
                .cursor_pointer().hover(|style| style.bg(rgb(HOVER))).text_size(px(12.))
                .child(small_icon("fm/folder-fill.svg", FOLDER))
                .child(div().flex_none().max_w(px(200.)).truncate().child(folder.title.clone()))
                .child(div().flex_1().min_w_0().truncate().text_color(rgb(TEXT_DIM)).child(full.clone()))
                .when(folder.pinned, |row| row.child(div().flex_none().text_size(px(11.)).text_color(rgb(TEXT_DIM)).child("закреплена")))
                .tooltip(move |window, cx| Tooltip::new(format!("Перейти: {full}")).build(window, cx))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.project_card = None;
                    this.switch_group_tab(group, index, cx);
                })));
        }

        body = body.child(caption("Последние изменения"));
        match &card.changes {
            None => body = body.child(dim("Читаются…".into())),
            Some(changes) if changes.is_empty() => body = body.child(dim(format!(
                "За {DAYS} дней изменений не замечено. Papka видит изменения в папках, пока они открыты в ней."))),
            Some(changes) => for (index, event) in changes.iter().enumerate() {
                let path = event.path.clone();
                let full = event.path.display().to_string();
                let gone = event.kind == "missing";
                body = body.child(div().id(("project-change", index)).h(px(26.)).flex_none().px_1().flex().items_center().gap_2().rounded(px(4.))
                    .cursor_pointer().hover(|style| style.bg(rgb(HOVER))).text_size(px(12.))
                    .child(div().w(px(96.)).flex_none().text_color(rgb(TEXT_DIM)).child(format!("{} {}", event.day_label(), event.clock_label())))
                    .child(div().flex_1().min_w_0().truncate().child(browser::display_name(&event.path)))
                    .child(div().flex_none().text_color(rgb(if gone { DANGER } else { ACCENT })).child(event.feed_label().to_string()))
                    .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project_card = None;
                        this.focus_group(group, cx);
                        this.open_feed_event(path.clone(), cx);
                    })));
            },
        }

        let (file, page) = match &card.passport {
            PassportState::Found { file, page, .. } => (Some(file.clone()), page.clone()),
            PassportState::Unreadable { file, .. } => (Some(file.clone()), None),
            _ => (None, None),
        };
        let footer = div().flex_none().px_4().py_3().flex().items_center().gap_2().border_t_1().border_color(rgb(BORDER))
            .child(div().flex_1().min_w_0().text_size(px(11.)).text_color(rgb(TEXT_DIM))
                .child("Карточку только читают: паспорт объекта ведут расчётные программы."))
            .when_some(file, |row, file| row.child(text_button("project-card-file", "Показать паспорт", ButtonKind::Ghost)
                .tooltip({ let tip = file.display().to_string(); move |window, cx| Tooltip::new(tip.clone()).build(window, cx) })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.project_card = None;
                    this.focus_group(group, cx);
                    this.open_feed_event(file.clone(), cx);
                }))))
            .when_some(page, |row, page| row.child(text_button("project-card-page", "Открыть в браузере", ButtonKind::Ghost)
                .on_click(cx.listener(move |this, _, _, cx| this.open_outside(&page, cx)))));

        let sheet = div().id("project-card").occlude().w(px(620.)).max_w_full().max_h(max_height).flex().flex_col().rounded(px(6.))
            .bg(rgb(RAISED)).border_1().border_color(rgb(BORDER_STRONG)).text_size(px(13.)).text_color(rgb(TEXT))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().flex_none().h(px(44.)).pl_4().pr_2().flex().items_center().gap_2()
                .child(small_icon("fm/layers.svg", ACCENT))
                .child(div().flex_1().min_w_0().truncate().text_size(px(15.)).font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(format!("Проект «{}»", card.name)))
                .child(icon_button("project-card-refresh", "fm/refresh.svg", "Прочитать заново", false, true)
                    .on_click(cx.listener(move |this, _, _, cx| this.open_project_card(group, cx))))
                .child(icon_button("project-card-close", "fm/x.svg", "Закрыть (Esc)", false, true)
                    .on_click(cx.listener(|this, _, _, cx| this.close_project_card(cx)))))
            .child(body)
            .child(footer);
        Some(div().id("project-card-backdrop").absolute().inset_0().flex().items_start().justify_center().pt(px(64.)).px_4()
            .bg(gpui::rgba(0x00000066))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_project_card(cx)))
            .child(sheet)
            .with_animation("project-card-appear", crate::theme::motion(130), |this, delta| this.opacity(delta))
            .into_any_element())
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
