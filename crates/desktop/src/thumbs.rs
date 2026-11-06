//! The thumbnails view (Ctrl+3): tiles with a picture of each file, as in
//! Explorer's «Крупные значки». Pictures come from the Windows Shell (and
//! the DWG's own preview) in the background, one batch at a time, and are
//! kept in the thumbnail cache on disk.
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use gpui::{AppContext, Context};

use crate::{Explorer, GroupId};

/// The gap between tiles.
pub(crate) const TILE_GAP: f32 = 6.;
/// «Размер миниатюр»: small, medium, large.
pub(crate) const SIZES: [&str; 3] = ["Мелкие", "Средние", "Крупные"];
const SCALES: [f32; 3] = [0.75, 1., 1.5];
static SIZE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
pub(crate) fn set_size(index: usize) { SIZE.store(index.min(SIZES.len() - 1), std::sync::atomic::Ordering::Relaxed); }
pub(crate) fn size() -> usize { SIZE.load(std::sync::atomic::Ordering::Relaxed) }
fn scale() -> f32 { SCALES[size()] }
/// The picture area inside a tile, and the tile around it with its name.
pub(crate) fn picture() -> f32 { (96. * scale()).round() }
pub(crate) fn tile_width() -> f32 { picture() + 36. }
pub(crate) fn tile_height() -> f32 { picture() + 52. }
/// Pixels a thumbnail is made at: sharp on high-DPI screens too.
const EDGE: u32 = 256;
const GROUPS_KEY: &str = "thumbnail_panes";

#[derive(Default)]
pub(crate) struct Thumbs {
    /// Panes that show tiles.
    pub(crate) groups: HashSet<GroupId>,
    /// None: asked for, or no picture.
    map: HashMap<PathBuf, Option<PathBuf>>,
    queue: VecDeque<PathBuf>,
    running: bool,
}

/// Tiles in a row of a pane `width` wide.
pub(crate) fn columns(width: f32) -> usize {
    (((width - 16.) / (tile_width() + TILE_GAP)).floor() as usize).max(1)
}

pub(crate) fn is_picture(path: &Path) -> bool {
    path.extension().is_some_and(|e| matches!(e.to_string_lossy().to_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp"))
}

impl Explorer {
    pub(crate) fn load_thumb_panes(&mut self) {
        let saved = self.workspaces.as_ref().and_then(|s| s.value(GROUPS_KEY).ok().flatten()).unwrap_or_default();
        let open = self.browser.leaves();
        self.thumbs.groups = saved.split(',').filter_map(|id| id.trim().parse::<u64>().ok())
            .map(GroupId).filter(|id| open.contains(id)).collect();
    }

    /// Whether `side` shows tiles.
    pub(crate) fn thumbs_view(&self, side: GroupId) -> bool {
        self.thumbs.groups.contains(&side) && !self.miller(side)
    }

    /// Ctrl+1 list, Ctrl+2 columns, Ctrl+3 tiles, for the active pane.
    pub(crate) fn set_view(&mut self, miller: bool, tiles: bool, cx: &mut Context<Self>) {
        let side = self.active_side();
        self.browser.active_group_mut().miller = miller;
        if tiles { self.thumbs.groups.insert(side); } else { self.thumbs.groups.remove(&side); }
        if let Some(store) = &self.workspaces {
            let ids: Vec<String> = self.thumbs.groups.iter().map(|id| id.0.to_string()).collect();
            let _ = store.set_value(GROUPS_KEY, &ids.join(","));
        }
        cx.notify();
    }

    /// The picture of a tile, once made; asks for it otherwise. Only kinds
    /// that have a picture are asked for; the rest keep their icon.
    pub(crate) fn thumb(&mut self, path: &Path, cx: &mut Context<Self>) -> Option<PathBuf> {
        if let Some(found) = self.thumbs.map.get(path) { return found.clone(); }
        let wanted = crate::wants_shell_thumbnail(path) || is_picture(path)
            || path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dwg"));
        if !wanted || filemanager_core::archive::is_inside(path) { return None; }
        // Browsing many folders must not keep every picture.
        if self.thumbs.map.len() > 4000 { self.thumbs.map.clear(); }
        self.thumbs.map.insert(path.to_path_buf(), None);
        self.thumbs.queue.push_back(path.to_path_buf());
        self.pump_thumbs(cx);
        None
    }

    fn pump_thumbs(&mut self, cx: &mut Context<Self>) {
        if self.thumbs.running || self.thumbs.queue.is_empty() { return; }
        self.thumbs.running = true;
        let batch: Vec<PathBuf> = (0..12).filter_map(|_| self.thumbs.queue.pop_front()).collect();
        let task = cx.background_spawn(async move {
            let exe = std::env::current_exe().ok();
            batch.into_iter().map(|path| {
                // A drawing's own picture is read here; the Shell's (with
                // third-party handlers) only in a helper process, as for
                // the details panel, so a bad handler cannot take Papka down.
                let dwg = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dwg"));
                let made = if dwg { filemanager_core::thumbnail::thumbnail(&path, EDGE).ok() }
                    else { exe.as_ref().and_then(|exe| filemanager_core::preview_host::isolated_thumbnail(exe, &path, EDGE).ok()) };
                let made = made
                    // Without the Shell (or when it fails) a picture is its own thumbnail.
                    .or_else(|| is_picture(&path).then(|| path.clone()));
                (path, made)
            }).collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let made = task.await;
            let _ = this.update(cx, |this, cx| {
                this.thumbs.running = false;
                for (path, picture) in made {
                    if let Some(slot) = this.thumbs.map.get_mut(&path) { *slot = picture; }
                }
                this.pump_thumbs(cx);
                cx.notify();
            });
        }).detach();
    }

    /// A folder changed: its pictures are made again.
    pub(crate) fn forget_thumbs(&mut self, folder: &Path) {
        self.thumbs.map.retain(|path, _| path.parent() != Some(folder));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tiles_fill_the_pane_and_never_vanish() {
        assert_eq!(super::columns(0.), 1);
        assert_eq!(super::columns(100.), 1);
        assert_eq!(super::columns(16. + 3. * (super::tile_width() + super::TILE_GAP)), 3);
        assert_eq!(super::columns(1100.), 7);
    }
}
