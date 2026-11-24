//! «Тип»: a pane shows only files of one kind (drawings, PDF, …) in one
//! click; folders stay, so the way to more such files stays open.

use std::collections::HashMap;

use filemanager_core::kinds::Kind;
use filemanager_core::panes::GroupId;
use gpui::Context;

use crate::Explorer;

#[derive(Default)]
pub(crate) struct KindFilters {
    by_pane: HashMap<GroupId, Kind>,
}

impl KindFilters {
    pub(crate) fn get(&self, group: GroupId) -> Option<Kind> { self.by_pane.get(&group).copied() }
    pub(crate) fn retain(&mut self, panes: &[GroupId]) { self.by_pane.retain(|id, _| panes.contains(id)); }
}

impl Explorer {
    /// Shows only files of `kind` in the active pane (None: all again).
    pub(crate) fn set_kind_filter(&mut self, kind: Option<Kind>, cx: &mut Context<Self>) {
        let group = self.browser.active;
        match kind {
            Some(kind) => {
                self.kinds.by_pane.insert(group, kind);
                // The chip in the status bar says it; the message would repeat it.
                self.status.clear();
            }
            None => {
                self.kinds.by_pane.remove(&group);
                self.status = "Показаны файлы всех типов".into();
            }
        }
        self.context_menu = None;
        cx.notify();
    }

    /// Kinds of the files in the active pane's folder, with counts.
    pub(crate) fn kinds_here(&self) -> Vec<(Kind, usize)> {
        let folder = self.pane_path(self.active_side());
        self.dirs.cache.get(&folder).map(|listing| filemanager_core::kinds::count(
            listing.entries.iter().filter(|entry| !entry.is_directory && (self.show_hidden || !entry.hidden)).map(|entry| entry.path.as_path())
        )).unwrap_or_default()
    }
}
