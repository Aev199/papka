//! Manual file ordering: a presentation-only preview, followed by an explicit
//! same-folder commit. No file operation is dispatched by this module.
use std::{collections::{HashMap, HashSet}, path::{Path, PathBuf}, sync::Arc, time::Instant};
use gpui::{AppContext, Context, Pixels, Point, ScrollHandle, px};
use filemanager_core::{browser::Listing, manual_order::{ManualOrders, reorder}};
use crate::{Explorer, FileDragInfo, GroupId, list_motion::ListMotion, row_motion::Row};

const KEY: &str = "manual_file_orders";
const LIMIT: usize = 5000;

#[derive(Clone, PartialEq)]
pub(crate) struct Pointer {
    pub source: (GroupId, PathBuf),
    pub position: Point<Pixels>,
}

struct Preview {
    folder: PathBuf,
    listing: Arc<Listing>,
    base: Arc<Vec<usize>>,
    order: Arc<Vec<usize>>,
    boundary: Option<usize>,
    paths: Vec<PathBuf>,
    hidden: Arc<HashSet<PathBuf>>,
    motion: Option<ListMotion>,
}

#[derive(Default)]
pub(crate) struct ManualDrag {
    pub pointer: Option<Pointer>,
    previews: HashMap<GroupId, Preview>,
}

pub(crate) struct Display {
    pub order: Arc<Vec<usize>>,
    pub hidden: Arc<HashSet<PathBuf>>,
    pub geometry: Option<(Vec<Row>, f32)>,
}

impl ManualDrag {
    pub(crate) fn prune(&mut self, ids: &[GroupId]) {
        self.previews.retain(|id, _| ids.contains(id));
        if self.pointer.as_ref().is_some_and(|p| !ids.contains(&p.source.0)) { self.pointer = None; }
    }
}

/// The drop boundary is always measured against the unchanged logical
/// slots, not the animated files. Otherwise a moving row chases the pointer.
fn boundary(count: usize, first_file: usize, columns: usize, cell: (f32, f32), x: f32, y: f32) -> Option<usize> {
    if count == 0 || columns == 0 || cell.0 <= 0. || cell.1 <= 0. || x < 0. || y < 0. { return None; }
    let row = (y / cell.1).floor() as usize;
    if row >= count.div_ceil(columns) { return Some(count); }
    let column = if columns == 1 { 0 } else { ((x / cell.0).floor() as usize).min(columns - 1) };
    let position = row * columns + column;
    if position < first_file { return None; }
    if position >= count { return Some(count); }
    let after = if columns == 1 { y / cell.1 - row as f32 >= 0.5 }
        else { x / cell.0 - column as f32 >= 0.5 };
    Some((position + usize::from(after)).min(count))
}

impl Explorer {
    pub(crate) fn load_manual_orders(store: Option<&filemanager_core::workspace::WorkspaceStore>) -> ManualOrders {
        store.and_then(|s| s.value(KEY).ok().flatten()).map(|text| ManualOrders::decode(&text)).unwrap_or_default()
    }

    fn persist_manual_orders(&mut self) {
        let result = self.manual_orders.encode().map_err(|e| e.to_string()).and_then(|text| {
            if let Some(store) = &self.workspaces { store.set_value(KEY, &text).map_err(|e| e.to_string())?; }
            Ok(())
        });
        if let Err(error) = result { self.status = format!("Ручной порядок не сохранён: {error}"); }
    }

    pub(crate) fn set_manual_order(&mut self, side: GroupId, enabled: bool, cx: &mut Context<Self>) {
        let folder = self.pane_path(side);
        if filemanager_core::archive::is_inside(&folder) { return; }
        if self.manual_orders.enabled(&folder) == enabled { return; }
        self.manual_orders.set_enabled(&folder, enabled);
        self.manual_drag.previews.remove(&side);
        self.manual_drag.pointer = None;
        self.persist_manual_orders();
        cx.notify();
    }

    pub(crate) fn manual_display(&mut self, side: GroupId, folder: &Path, listing: &Arc<Listing>, base: &Arc<Vec<usize>>,
        scroll: &ScrollHandle, columns: usize, cell: (f32, f32), tiles: bool, now: Instant, cx: &Context<Self>) -> Option<Display> {
        if folder != self.pane_path(side) { return None; }
        if !self.manual_orders.enabled(folder) || base.len() > LIMIT {
            self.manual_drag.previews.remove(&side);
            return None;
        }

        let pointer = self.manual_drag.pointer.as_ref().filter(|p| p.source.0 == side && p.source.1 == folder);
        let first_file = base.iter().take_while(|&&i| listing.entries[i].is_directory).count();
        let candidate = if self.dragging_files(cx) && self.right_drag.is_none() && self.filter.is_empty()
            && self.label_filter(side).is_none() && self.kinds.get(side).is_none() && !self.renaming {
            pointer.and_then(|p| {
                let bounds = scroll.bounds();
                if !bounds.contains(&p.position) || self.drag_paths.is_empty() { return None; }
                let selected = self.drag_paths.iter().map(|path| {
                    base.iter().position(|&i| !listing.entries[i].is_directory && listing.entries[i].path == *path)
                }).collect::<Option<Vec<_>>>()?;
                let padding = if tiles { 8. } else { 0. };
                let x = ((p.position.x - bounds.origin.x) / px(1.) - padding).max(0.);
                let y = ((p.position.y - bounds.origin.y - scroll.offset().y) / px(1.) - padding).max(0.);
                let at = boundary(base.len(), first_file, columns, cell, x, y)?;
                Some((at, selected, self.drag_paths.clone()))
            })
        } else { None };

        // A watcher or navigation must invalidate any preview derived from
        // different indices, even if the path and file count are unchanged.
        let view = self.manual_drag.previews.entry(side).or_insert_with(|| Preview {
            folder: folder.to_path_buf(), listing: listing.clone(), base: base.clone(), order: base.clone(),
            boundary: None, paths: Vec::new(), hidden: Arc::new(HashSet::new()), motion: None,
        });
        if view.folder != folder || !Arc::ptr_eq(&view.listing, listing) || !Arc::ptr_eq(&view.base, base) {
            *view = Preview { folder: folder.to_path_buf(), listing: listing.clone(), base: base.clone(), order: base.clone(),
                boundary: None, paths: Vec::new(), hidden: Arc::new(HashSet::new()), motion: None };
        }
        let at = candidate.as_ref().map(|c| c.0);
        let paths = candidate.as_ref().map_or(&[][..], |c| c.2.as_slice());
        if at != view.boundary || paths != view.paths.as_slice() {
            let next = candidate.as_ref().and_then(|(at, selected, _)| reorder(base.len(), selected, *at))
                .map(|positions| Arc::new(positions.into_iter().map(|p| base[p]).collect()))
                .unwrap_or_else(|| base.clone());
            view.motion = if !tiles && crate::theme::animations() {
                ListMotion::new(listing.clone(), &view.order, &next, view.motion.as_ref(), now)
            } else { None };
            view.order = next;
            view.boundary = at;
            view.paths = paths.to_vec();
            view.hidden = Arc::new(paths.iter().cloned().collect());
        }
        if tiles || !crate::theme::animations() || view.motion.as_ref().is_some_and(|m| !m.active(now)) { view.motion = None; }
        let geometry = view.motion.as_ref().map(|m| m.geometry(now));
        if geometry.is_some() { self.motion.late_frame.set(true); }
        Some(Display { order: view.order.clone(), hidden: view.hidden.clone(), geometry })
    }

    pub(crate) fn commit_manual_drop(&mut self, side: GroupId, folder: &Path, data: &FileDragInfo,
        modifiers: gpui::Modifiers, cx: &mut Context<Self>) -> bool {
        if modifiers.control || modifiers.shift || !self.manual_orders.enabled(folder)
            || data.source.as_ref().is_none_or(|(group, path)| *group != side || path != folder) { return false; }
        let Some((listing, base)) = self.folder_view(side, folder) else { return false; };
        let Some(view) = self.manual_drag.previews.get(&side).filter(|view|
            view.folder == folder && view.boundary.is_some() && view.paths == data.paths
                && Arc::ptr_eq(&view.listing, &listing) && Arc::ptr_eq(&view.base, &base)) else { return false; };

        // Merge the visible order into the complete one: hidden files retain
        // their slots and are not forgotten when hidden items are toggled.
        let visible: HashSet<_> = base.iter().copied().filter(|&i| !listing.entries[i].is_directory).collect();
        let mut changed = view.order.iter().copied().filter(|&i| !listing.entries[i].is_directory);
        let mut full = filemanager_core::sort::sorted_indices(&listing.entries, self.sort_for(side));
        self.manual_orders.apply(folder, &listing.entries, &mut full);
        let ordered: Vec<_> = full.into_iter().filter(|&i| !listing.entries[i].is_directory).map(|i| {
            let index = if visible.contains(&i) { changed.next().unwrap_or(i) } else { i };
            listing.entries[index].path.clone()
        }).collect();
        self.manual_drag.pointer = None;
        self.manual_drag.previews.remove(&side);
        if self.manual_orders.save(folder, &ordered) {
            self.status = "Ручной порядок сохранён".into();
            self.persist_manual_orders();
        }
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logical_boundary_respects_folders_grid_and_empty_tail() {
        assert_eq!(boundary(8, 2, 1, (200., 24.), 30., 10.), None);
        assert_eq!(boundary(8, 2, 1, (200., 24.), 30., 49.), Some(2));
        assert_eq!(boundary(8, 2, 1, (200., 24.), 30., 71.), Some(3));
        assert_eq!(boundary(8, 2, 3, (100., 120.), 250., 10.), Some(3));
        assert_eq!(boundary(8, 2, 3, (100., 120.), 250., 250.), Some(8));
        assert_eq!(boundary(8, 2, 3, (100., 120.), 20., 500.), Some(8));
    }
}
