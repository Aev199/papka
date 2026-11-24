//! Manual file ordering: a presentation-only preview, followed by an explicit
//! same-folder commit. No file operation is dispatched by this module.
use std::{collections::{HashMap, HashSet}, path::{Path, PathBuf}, sync::Arc, time::{Duration,Instant}};
use gpui::{AppContext, Context, Pixels, Point, ScrollHandle, px};
use filemanager_core::{browser::Listing, manual_order::{ManualOrders, reorder}};
use crate::{Explorer, FileDragInfo, GroupId, list_motion::ListMotion, row_motion::Row};

const KEY: &str = "manual_file_orders";
const LIMIT: usize = 5000;
const SETTLE_LIMIT: usize = 64;
const SETTLE_SPAN: Duration = Duration::from_millis(150);

#[derive(Clone, PartialEq)]
pub(crate) struct Pointer {
    pub source: (GroupId, PathBuf),
    pub position: Point<Pixels>,
}

struct Preview {
    folder: PathBuf,
    listing: Arc<Listing>,
    base: Arc<Vec<usize>>,
    logical_positions: Arc<HashMap<usize, usize>>,
    columns: usize,
    cell: (f32, f32),
    tiles: bool,
    order: Arc<Vec<usize>>,
    boundary: Option<usize>,
    paths: Vec<PathBuf>,
    hidden: Arc<HashSet<PathBuf>>,
    selected: Vec<usize>,
    motion: Option<ListMotion>,
}

struct Settle {
    folder:PathBuf,
    paths:HashSet<PathBuf>,
    started:Instant,
    committed:bool,
}
impl Settle {
    fn new(folder:&Path,paths:&[PathBuf],now:Instant,committed:bool)->Self {
        Self{folder:folder.to_path_buf(),paths:paths.iter().take(SETTLE_LIMIT).cloned().collect(),started:now,committed}
    }
    fn strength(&self,folder:&Path,path:&Path,now:Instant)->Option<(bool,f32)> {
        let elapsed=now.saturating_duration_since(self.started);
        if folder!=self.folder || !self.paths.contains(path) || elapsed>=SETTLE_SPAN {return None;}
        Some((self.committed,1.-crate::motion_policy::ease_out(elapsed,SETTLE_SPAN)))
    }
}

#[derive(Default)]
pub(crate) struct ManualDrag {
    pub pointer: Option<Pointer>,
    pub transfer_modifier: bool,
    previews: HashMap<GroupId, Preview>,
    settles: HashMap<GroupId,Settle>,
}

pub(crate) struct Display {
    pub order: Arc<Vec<usize>>,
    pub hidden: Arc<HashSet<PathBuf>>,
    pub logical_positions: Arc<HashMap<usize, usize>>,
    pub geometry: Option<(Vec<Row>, f32)>,
}

impl ManualDrag {
    pub(crate) fn prune(&mut self, ids: &[GroupId]) {
        self.previews.retain(|id, _| ids.contains(id));
        self.settles.retain(|id,_|ids.contains(id));
        if self.pointer.as_ref().is_some_and(|p| !ids.contains(&p.source.0)) { self.pointer = None; }
    }

    fn settle(&mut self,side:GroupId,folder:&Path,paths:&[PathBuf],now:Instant,committed:bool) {
        if crate::theme::animations() && !paths.is_empty() {
            self.settles.insert(side,Settle::new(folder,paths,now,committed));
        }
    }

    pub(crate) fn advance(&mut self,now:Instant)->bool {
        self.settles.retain(|_,s|crate::theme::animations() && now.saturating_duration_since(s.started)<SETTLE_SPAN);
        !self.settles.is_empty()
    }

    pub(crate) fn strength(&self,side:GroupId,folder:&Path,path:&Path)->Option<(bool,f32)> {
        if !crate::theme::animations() {return None;}
        self.settles.get(&side)?.strength(folder,path,Instant::now())
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

fn selected_positions(listing: &Listing, base: &[usize], paths: &[PathBuf]) -> Option<Vec<usize>> {
    if paths.is_empty() { return None; }
    let wanted: HashSet<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let selected: Vec<_> = base.iter().enumerate().filter_map(|(position, &index)| {
        let entry = &listing.entries[index];
        (!entry.is_directory && wanted.contains(entry.path.as_path())).then_some(position)
    }).collect();
    (selected.len() == paths.len()).then_some(selected)
}

fn pointer_boundary(scroll: &ScrollHandle, position: Point<Pixels>, count: usize, first_file: usize,
    columns: usize, cell: (f32, f32), tiles: bool) -> Option<usize> {
    let bounds = scroll.bounds();
    if !bounds.contains(&position) { return None; }
    let padding = if tiles { 8. } else { 0. };
    let x = ((position.x - bounds.origin.x) / px(1.) - padding).max(0.);
    let y = ((position.y - bounds.origin.y - scroll.offset().y) / px(1.) - padding).max(0.);
    boundary(count, first_file, columns, cell, x, y)
}

fn logical_positions(base: &[usize]) -> Arc<HashMap<usize, usize>> {
    Arc::new(base.iter().enumerate().map(|(position, &index)| (index, position)).collect())
}

impl Explorer {
    pub(crate) fn load_manual_orders(store: Option<&filemanager_core::workspace::WorkspaceStore>) -> ManualOrders {
        store.and_then(|s| s.value(KEY).ok().flatten()).map(|text| ManualOrders::decode(&text)).unwrap_or_default()
    }

    fn persist_manual_orders(&mut self) {
        let result = self.manual_orders.encode().map_err(|e| e.to_string()).and_then(|text| {
            let store = self.workspaces.as_ref().ok_or_else(|| "локальная база недоступна".to_string())?;
            store.set_value(KEY, &text).map_err(|e| e.to_string())?;
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
        if !self.manual_orders.enabled(folder) || listing.truncated || base.len() > LIMIT {
            self.manual_drag.previews.remove(&side);
            return None;
        }

        if self.manual_drag.pointer.is_none() && !self.manual_drag.previews.contains_key(&side) { return None; }
        let pointer = self.manual_drag.pointer.as_ref().filter(|p| p.source.0 == side && p.source.1 == folder);
        let first_file = base.iter().take_while(|&&i| listing.entries[i].is_directory).count();
        let dragging = self.dragging_files(cx);
        let candidate = if dragging && !self.manual_drag.transfer_modifier && self.right_drag.is_none() && self.filter.is_empty()
            && self.label_filter(side).is_none() && self.kinds.get(side).is_none() && !self.renaming {
            pointer.and_then(|p| {
                let at = pointer_boundary(scroll, p.position, base.len(), first_file, columns, cell, tiles)?;
                let selected = self.manual_drag.previews.get(&side)
                    .filter(|view|Arc::ptr_eq(&view.listing,listing) && Arc::ptr_eq(&view.base,base)
                        && view.paths==self.drag_paths && !view.selected.is_empty())
                    .map(|view|view.selected.clone())
                    .or_else(||selected_positions(listing,base,&self.drag_paths))?;
                Some((at, selected, self.drag_paths.clone()))
            })
        } else { None };

        if candidate.is_none() && !dragging {
            let returning=self.manual_drag.previews.get(&side).filter(|view|view.folder==folder && view.boundary.is_some())
                .map(|view|view.paths.clone());
            if let Some(paths)=returning {self.manual_drag.settle(side,folder,&paths,now,false);}
        }
        // A watcher or navigation must invalidate any preview derived from
        // different indices, even if the path and file count are unchanged.
        if candidate.is_none() && !self.manual_drag.previews.contains_key(&side) { return None; }
        let view = self.manual_drag.previews.entry(side).or_insert_with(|| Preview {
            folder: folder.to_path_buf(), listing: listing.clone(), base: base.clone(), order: base.clone(),
            logical_positions: logical_positions(base), columns, cell, tiles,
            boundary: None, paths: Vec::new(), hidden: Arc::new(HashSet::new()), selected:Vec::new(), motion: None,
        });
        if view.folder != folder || !Arc::ptr_eq(&view.listing, listing) || !Arc::ptr_eq(&view.base, base) {
            *view = Preview { folder: folder.to_path_buf(), listing: listing.clone(), base: base.clone(), order: base.clone(),
                logical_positions: logical_positions(base), columns, cell, tiles,
                boundary: None, paths: Vec::new(), hidden: Arc::new(HashSet::new()), selected:Vec::new(), motion: None };
        }
        view.columns = columns; view.cell = cell; view.tiles = tiles;
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
            view.selected = candidate.as_ref().map_or_else(Vec::new,|c|c.1.clone());
            view.hidden = Arc::new(paths.iter().cloned().collect());
        }
        if tiles || !crate::theme::animations() || view.motion.as_ref().is_some_and(|m| !m.active(now)) { view.motion = None; }
        if candidate.is_some() {
            self.drag_target.borrow_mut().manual_folder = Some(folder.to_path_buf());
        }
        let geometry = view.motion.as_ref().map(|m| m.geometry(now));
        if geometry.is_some() { self.motion.late_frame.set(true); }
        if view.boundary.is_none() && geometry.is_none() {
            self.manual_drag.previews.remove(&side);
            return None;
        }
        Some(Display { order: view.order.clone(), hidden: view.hidden.clone(),
            logical_positions: view.logical_positions.clone(), geometry })
    }

    pub(crate) fn commit_manual_drop(&mut self, side: GroupId, folder: &Path, data: &FileDragInfo,
        position: Point<Pixels>, modifiers: gpui::Modifiers, cx: &mut Context<Self>) -> bool {
        if modifiers.control || modifiers.shift || !self.manual_orders.enabled(folder)
            || folder != self.pane_path(side) || !self.filter.is_empty() || self.label_filter(side).is_some()
            || self.kinds.get(side).is_some() || self.renaming
            || data.source.as_ref().is_none_or(|(group, path)| *group != side || path != folder) { return false; }
        let Some((listing, base)) = self.folder_view(side, folder) else { return false; };
        if listing.truncated || base.len() > LIMIT { return false; }
        let Some(view) = self.manual_drag.previews.get(&side).filter(|view|
            view.folder == folder && view.boundary.is_some() && view.paths == data.paths
                && Arc::ptr_eq(&view.listing, &listing) && Arc::ptr_eq(&view.base, &base)) else { return false; };

        // Merge the visible order into the complete one: hidden files retain
        // their slots and are not forgotten when hidden items are toggled.
        let Some(handle) = self.scroll_handles.get(&(side, folder.to_path_buf())) else { return false; };
        let first_file = base.iter().take_while(|&&i| listing.entries[i].is_directory).count();
        let at = pointer_boundary(&handle.0.borrow().base_handle, position, base.len(), first_file,
            view.columns, view.cell, view.tiles);
        let Some(at) = at else { return false; };
        let Some(selected) = selected_positions(&listing, &base, &data.paths) else { return false; };
        let Some(positions) = reorder(base.len(), &selected, at) else { return false; };
        let order: Vec<_> = positions.into_iter().map(|p| base[p]).collect();
        let visible: HashSet<_> = base.iter().copied().filter(|&i| !listing.entries[i].is_directory).collect();
        let mut changed = order.iter().copied().filter(|&i| !listing.entries[i].is_directory);
        let mut full = filemanager_core::sort::sorted_indices(&listing.entries, self.sort_for(side));
        self.manual_orders.apply(folder, &listing.entries, &mut full);
        let ordered: Vec<_> = full.into_iter().filter(|&i| !listing.entries[i].is_directory).map(|i| {
            let index = if visible.contains(&i) { changed.next().unwrap_or(i) } else { i };
            listing.entries[index].path.clone()
        }).collect();
        let now = Instant::now();
        let follow_through = if !view.tiles && crate::theme::animations() {
            if view.order.as_slice() == order.as_slice() { None }
            else { ListMotion::new(listing.clone(), &view.order, &order, view.motion.as_ref(), now) }
        } else { None };
        self.manual_drag.pointer = None;
        let previous = self.manual_drag.previews.remove(&side);
        if let Some(motion) = follow_through.or_else(|| previous.and_then(|view| view.motion).filter(|m| m.matches(&order))) {
            self.motion.lists.insert((side, folder.to_path_buf()), motion);
        }
        if self.manual_orders.save(folder, &ordered) {
            self.status = "Ручной порядок сохранён".into();
            self.persist_manual_orders();
            self.manual_drag.settle(side,folder,&data.paths,now,true);
        }
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settlement_is_bounded_folder_scoped_and_expires() {
        let now=Instant::now();let folder=Path::new("a");
        let paths:Vec<_>=(0..100).map(|n|folder.join(n.to_string())).collect();
        let s=Settle::new(folder,&paths,now,true);
        assert_eq!(s.paths.len(),SETTLE_LIMIT);
        assert_eq!(s.strength(folder,&paths[0],now),Some((true,1.)));
        assert!(s.strength(Path::new("b"),&paths[0],now).is_none());
        assert!(s.strength(folder,&paths[80],now).is_none());
        assert!(s.strength(folder,&paths[0],now+SETTLE_SPAN).is_none());
        let s=Settle::new(folder,&paths,now,false);
        assert_eq!(s.strength(folder,&paths[0],now),Some((false,1.)));
    }

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
