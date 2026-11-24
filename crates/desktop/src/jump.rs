//! Ctrl+P: jump to a folder by a few letters of its name. The command
//! palette's window lists folders instead of commands: favourites, recent
//! places, open tabs, places and the subfolders of the current folder and
//! of the favourites, read in the background.

use std::collections::HashSet;
use std::path::PathBuf;

use filemanager_core::{jump, labels, path_utils};
use gpui::{prelude::*, px, AnyElement, AppContext, Context, Window};
use gpui_component::input::Input;

use crate::interface::Palette;
use crate::theme::*;
use crate::Explorer;

/// Rows shown at most; more letters narrow the list.
const SHOWN: usize = 60;
/// Subfolders read for the list: levels below each root and the total.
const DEPTH: usize = 3;
const LIMIT: usize = 6000;

/// Stays in folders, for Ctrl+P: a folder counts once per stay, and only
/// when it was the active pane's on two autosave ticks running (2 s or
/// more), not when a tick catches it on the way through.
#[derive(Default)]
pub(crate) struct Stays {
    pub(crate) visits: jump::Visits,
    noted: Option<PathBuf>,
    seen: Option<PathBuf>,
}

impl Stays {
    pub(crate) fn new(visits: jump::Visits) -> Self { Self { visits, ..Self::default() } }

    /// The autosave tick with the active pane's folder; true when a stay
    /// was counted (the visits are to be saved).
    pub(crate) fn tick(&mut self, active: &std::path::Path, now_ms: i64) -> bool {
        let stayed = self.seen.as_deref() == Some(active);
        self.seen = Some(active.to_path_buf());
        if !stayed {
            // Left the folder of the last stay: coming back is a new stay.
            if self.noted.as_deref() != Some(active) { self.noted = None; }
            return false;
        }
        if self.noted.as_deref() == Some(active) { return false; }
        self.noted = Some(active.to_path_buf());
        self.visits.visit(active, now_ms);
        true
    }
}

#[derive(Default)]
pub(crate) struct Folders {
    /// Candidates in the order they are offered without a query.
    items: Vec<(PathBuf, &'static str)>,
    seen: HashSet<String>,
    pub(crate) reading: bool,
    checked_path: Option<(PathBuf,bool)>,
    checking_path: bool,
}

impl Folders {
    fn add(&mut self, path: PathBuf, source: &'static str) {
        if self.seen.insert(labels::key(&path)) {
            self.items.push((path, source));
        }
    }
}

fn typed_local_path(query:&str)->Option<PathBuf>{
    let query=query.trim();
    let path=PathBuf::from(query);
    ((query.contains('\\') || query.contains('/')) && !query.starts_with(r"\\") && path.is_absolute()
        && !path_utils::is_network_path(&path)).then_some(path)
}

impl Explorer {
    pub(crate) fn open_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            // Ctrl+P again closes it, as a toggle.
            self.motion.surfaces.retire_interface(self.palette.take(),None);
            window.focus(&self.focus_handle, cx);
            cx.notify();
            return;
        }
        let mut folders = Folders::default();
        let current = self.pane_path(self.active_side());
        for path in &self.favorites { folders.add(path.clone(), "Избранное"); }
        for path in &self.recent { folders.add(path.clone(), "Недавние"); }
        // Folders of daily work, however deep or far (a share): offered even
        // when the walk below does not reach them.
        for path in self.stays.visits.top(80, crate::now_ms()) { folders.add(path, "Частые"); }
        for group in &self.browser.groups {
            for tab in &group.tabs { folders.add(tab.active().path.clone(), "Вкладка"); }
        }
        if let Some(listing) = self.dirs.cache.get(&current) {
            for entry in listing.entries.iter().filter(|entry| entry.is_directory && !entry.hidden) {
                folders.add(entry.path.clone(), "Здесь");
            }
        }
        for place in &self.places { folders.add(place.path.clone(), "Места"); }
        // Deeper folders of the current folder and the favourites; network
        // favourites are left out, their reading could take minutes.
        let near = |path: &PathBuf| !path_utils::is_network_path(path) && filemanager_core::archive::split(path).is_none();
        // A network folder is read one level only (its listing is cached).
        let mut roots: Vec<PathBuf> = Some(current.clone()).filter(|path| near(path)).into_iter().collect();
        roots.extend(self.favorites.iter().filter(|path| near(path)).cloned());
        // One level of the places, recent folders and the current folder's
        // neighbours.
        let mut shallow: Vec<PathBuf> = self.places.iter().map(|place| place.path.clone())
            .chain(self.recent.iter().cloned()).chain(current.parent().map(PathBuf::from)).filter(|path| near(path)).collect();
        shallow.dedup();
        folders.reading = true;
        self.open_palette_mode("Перейти к папке: несколько букв имени или путь…", Some(folders), window, cx);
        let Some(palette)=self.palette.as_ref().filter(|p|p.folders.is_some()) else {return;};
        let session=palette.input.entity_id();
        let task = cx.background_spawn(async move {
            let mut found = jump::subfolders(&roots, DEPTH, LIMIT);
            found.extend(jump::subfolders(&shallow, 1, LIMIT));
            found
        });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let _ = this.update(cx, |this, cx| {
                if !this.palette.as_ref().is_some_and(|p|p.input.entity_id()==session) {return;}
                let selected=this.jump_selected_path();
                if let Some(folders) = this.palette.as_mut().and_then(|palette| palette.folders.as_mut()) {
                    for path in found { folders.add(path, "Вложенная"); }
                    folders.reading = false;
                    this.restore_jump_selection(selected);
                    cx.notify();
                }
            });
        }).detach();
        cx.notify();
    }

    /// At most one typed-path probe per open palette; a changed query
    /// replaces the pending target instead of spawning work on every key.
    pub(crate) fn probe_jump_path(&mut self,cx:&mut Context<Self>) {
        let Some(palette)=self.palette.as_mut() else{return;};
        let Some(folders)=palette.folders.as_mut() else{return;};
        let Some(path)=typed_local_path(&palette.query) else{return;};
        if folders.checking_path || folders.checked_path.as_ref().is_some_and(|(p,_)|p==&path) {return;}
        folders.checking_path=true;
        let session=palette.input.entity_id();
        let probe=path.clone();
        let task=cx.background_spawn(async move {probe.is_dir()});
        cx.spawn(async move |weak,cx| {
            let valid=task.await;
            let _=weak.update(cx,|this,cx|{
                if !this.palette.as_ref().is_some_and(|p|p.input.entity_id()==session){return;}
                let selected=this.jump_selected_path();
                if let Some(folders)=this.palette.as_mut().and_then(|p|p.folders.as_mut()){
                    folders.checking_path=false;
                    folders.checked_path=Some((path,valid));
                }
                this.restore_jump_selection(selected);
                this.probe_jump_path(cx);
                cx.notify();
            });
        }).detach();
    }

    fn jump_selected_path(&self)->Option<PathBuf>{
        let selected=self.palette.as_ref()?.selected;
        self.jump_results().into_iter().nth(selected).map(|(path,_)|path)
    }

    fn restore_jump_selection(&mut self,path:Option<PathBuf>){
        let results=self.jump_results();
        if let Some(palette)=self.palette.as_mut(){
            palette.selected=path.and_then(|path|results.iter().position(|(p,_)|p==&path))
                .unwrap_or_else(||palette.selected.min(results.len().saturating_sub(1)));
            if !results.is_empty(){palette.scroll.scroll_to_item(palette.selected);}
        }
    }

    /// The folders matching what is typed, best first.
    pub(crate) fn jump_results(&self) -> Vec<(PathBuf, &'static str)> {
        self.palette.as_ref().map_or_else(Vec::new,|p|self.jump_results_for(p))
    }

    fn jump_results_for(&self,palette:&Palette)->Vec<(PathBuf,&'static str)>{
        let Some(folders) = palette.folders.as_ref() else { return Vec::new() };
        let query = palette.query.trim();
        let mut results = Vec::new();
        // Only use the completed probe for the exact current query.
        if let Some((path,true))=&folders.checked_path {
            if typed_local_path(query).as_ref()==Some(path) {results.push((path.clone(),"Путь"));}
        }
        if query.is_empty() {
            results.extend(folders.items.iter().filter(|(_, source)| *source != "Вложенная").take(SHOWN).cloned());
            return results;
        }
        // Folders opened often and lately rank above equal matches.
        let now = crate::now_ms();
        let mut scored: Vec<(i64, usize)> = folders.items.iter().enumerate()
            .filter_map(|(index, (path, _))| jump::score(query, path).map(|score| (score + self.stays.visits.bonus(path, now), index)))
            .collect();
        // Better match first; among equals, the order of the sources.
        scored.sort_by_key(|(score, index)| (-score, *index));
        let direct=results.first().map(|(path,_)|path.clone());
        results.extend(scored.into_iter().filter(|(_,index)|direct.as_ref()!=Some(&folders.items[*index].0))
            .take(SHOWN.saturating_sub(results.len())).map(|(_,index)|folders.items[index].clone()));
        results
    }

    pub(crate) fn jump_to(&mut self, path: PathBuf, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.motion.surfaces.retire_interface(self.palette.take(),None);
        window.focus(&self.focus_handle, cx);
        if !path.is_dir() {
            self.status = format!("Папка недоступна: {}", path.display());
            cx.notify();
            return;
        }
        if new_tab {
            self.open_in_new_tab(&path, cx);
        } else {
            let side = self.active_side();
            self.navigate_side(side, &path, None, cx);
        }
        cx.notify();
    }

    /// The window's list in jump mode.
    pub(crate) fn jump_list(&self, palette: &Palette, list_height:f32, cx: &mut Context<Self>) -> (AnyElement, String) {
        let results = self.jump_results_for(palette);
        let reading = palette.folders.as_ref().is_some_and(|folders| folders.reading || folders.checking_path);
        let mut list = gpui::div().id("jump-list").min_h_0().max_h(px(list_height)).overflow_y_scroll().py_1().track_scroll(&palette.scroll);
        if results.is_empty() {
            list = list.child(gpui::div().px_4().py_3().text_color(rgb(TEXT_MUTED))
                .child(if reading { "Читаем вложенные папки…" } else { "Папки не найдены" }));
        }
        let footer = format!("Папок: {}{}", results.len(), if reading { " · ищем глубже…" } else { "" });
        for (index, (path, source)) in results.into_iter().enumerate() {
            let chosen = index == palette.selected;
            let name = filemanager_core::browser::display_name(&path);
            let parent = path.parent().map(|parent| parent.display().to_string()).unwrap_or_default();
            let target = path.clone();
            list = list.child(gpui::div().id(("jump-folder", index)).mx_1().h(px(crate::ui_tokens::JUMP_ROW_HEIGHT)).px_3().flex().items_center().gap_3()
                .rounded(px(crate::ui_tokens::CONTROL_RADIUS)).cursor_pointer()
                .when(chosen, |row| row.bg(rgb(SELECTED)))
                .when(!chosen, |row| row.hover(|style| style.bg(rgb(HOVER))))
                .child(gpui::svg().path("fm/folder-fill.svg").size(px(16.)).flex_none()
                    .text_color(rgb(if chosen { SELECTION_TEXT } else { FOLDER })))
                .child(gpui::div().flex_1().min_w_0().flex().flex_col()
                    .child(gpui::div().truncate().text_color(rgb(if chosen { SELECTION_TEXT } else { TEXT })).child(name))
                    .child(gpui::div().truncate().text_size(px(11.)).text_color(rgb(if chosen { SELECTION_TEXT } else { TEXT_DIM })).child(parent)))
                .child(gpui::div().flex_none().text_size(px(11.)).text_color(rgb(if chosen { SELECTION_TEXT } else { TEXT_DIM })).child(source))
                .on_click(cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    this.jump_to(target.clone(), event.modifiers().control, window, cx);
                })));
        }
        (list.into_any_element(), footer)
    }
}

/// The search field of the jump window (the palette's own field).
pub(crate) fn field(palette: &Palette) -> Input {
    Input::new(&palette.input).appearance(false)
        .prefix(gpui::svg().path("fm/folder-fill.svg").size(px(14.)).text_color(rgb(TEXT_MUTED)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_stay_counts_once_and_a_pass_not_at_all() {
        let mut stays = Stays::default();
        let work = Path::new("/p/work");
        // First tick in a folder: maybe just passing through.
        assert!(!stays.tick(work, 1));
        // Still there on the next tick: one stay.
        assert!(stays.tick(work, 2));
        assert!(!stays.tick(work, 3));
        // Through another folder and back: a new stay after two ticks.
        assert!(!stays.tick(Path::new("/p/other"), 4));
        assert!(!stays.tick(work, 5));
        assert!(stays.tick(work, 6));
        assert!(stays.visits.bonus(work, 6) > 0);
        assert_eq!(stays.visits.bonus(Path::new("/p/other"), 6), 0);
    }
}
