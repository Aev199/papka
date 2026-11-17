//! Spring-loaded navigation: holding dragged files over a folder, place,
//! path crumb or tab for a moment opens it, so the drop target can be
//! reached without letting go.

use std::path::PathBuf;
use std::time::Duration;

use filemanager_core::panes::GroupId;
use gpui::{App, Context};

use crate::Explorer;

const DELAY: Duration = Duration::from_millis(700);

#[derive(Clone, PartialEq)]
pub(crate) enum SpringTarget {
    /// Open the folder in this pane.
    Folder(GroupId, PathBuf),
    /// Switch this pane to the tab.
    Tab(GroupId, usize),
}

pub(crate) struct Spring {
    target: SpringTarget,
    generation: u64,
    /// When the pointer settled on the target: the fill runs from here.
    started: std::time::Instant,
    /// Re-claimed by the hovered element on every move; the root drops an
    /// unclaimed spring, so leaving the target cancels it.
    claimed: bool,
}

impl Explorer {
    /// Files are being dragged: ours (left or right button) or Explorer's.
    pub(crate) fn dragging_files(&self, cx: &App) -> bool {
        (cx.has_active_drag() && self.file_drag) || self.right_drag.as_ref().is_some_and(|drag| drag.active)
    }

    /// Called by a hovered element while the pointer moves over it.
    pub(crate) fn spring_hover(&mut self, target: SpringTarget, cx: &mut Context<Self>) {
        if !self.dragging_files(cx) { return; }
        if let SpringTarget::Folder(_, folder) = &target {
            let dragged = match self.right_drag.as_ref().filter(|drag| drag.active) {
                Some(drag) => &drag.paths,
                None => &self.drag_paths,
            };
            if dragged.iter().any(|path| folder.starts_with(path)) { return; }
        }
        if let Some(spring) = self.spring.as_mut().filter(|spring| spring.target == target) {
            spring.claimed = true;
            return;
        }
        self.spring_generation += 1;
        let generation = self.spring_generation;
        self.spring = Some(Spring { target, generation, claimed: true, started: std::time::Instant::now() });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DELAY).await;
            let _ = this.update(cx, |this, cx| {
                let Some(spring) = this.spring.as_ref().filter(|spring| spring.generation == generation) else { return };
                if !this.dragging_files(cx) { this.spring = None; return; }
                let target = spring.target.clone();
                // Fired once; the same target does not re-arm until left.
                this.spring.as_mut().unwrap().generation = 0;
                match target {
                    SpringTarget::Folder(side, folder) => {
                        if this.pane_path(side) != folder { this.navigate_side(side, &folder, None, cx); }
                    }
                    SpringTarget::Tab(group, index) => this.switch_group_tab(group, index, cx),
                }
                cx.notify();
            });
        }).detach();
    }

    /// How far `target` has filled on its way to opening (0..1), while it does.
    pub(crate) fn spring_fill(&self, target: &SpringTarget) -> Option<f32> {
        if !crate::theme::animations() { return None; }
        let spring = self.spring.as_ref().filter(|spring| spring.generation != 0 && &spring.target == target)?;
        Some((spring.started.elapsed().as_secs_f32() / DELAY.as_secs_f32()).min(1.))
    }

    /// Whether a target is filling now (the window draws its next frame).
    pub(crate) fn spring_filling(&self) -> bool {
        self.spring.as_ref().is_some_and(|spring| spring.generation != 0 && spring.started.elapsed() < DELAY)
    }

    /// What a release over the hovered element does (a folder row or tile,
    /// a place, a tab), for the word on the dragged files. A folder the
    /// files come from or lie inside takes nothing.
    pub(crate) fn drag_over(&mut self, at: crate::DropAt, cx: &gpui::App) {
        if !self.dragging_files(cx) { return; }
        let at = match at {
            crate::DropAt::Folder(folder) if self.drag_paths.iter().any(|path| folder.starts_with(path)) => crate::DropAt::Nothing,
            other => other,
        };
        let mut drop = self.drag_target.borrow_mut();
        drop.at = at;
        drop.claimed = true;
        drop.touched = true;
    }

    /// A list's own folder under dragged files, unless a folder in it (a
    /// row or tile, met first) took the move.
    pub(crate) fn drag_over_list(&mut self, folder: &std::path::Path) {
        let mut drop = self.drag_target.borrow_mut();
        if !drop.claimed { drop.at = crate::DropAt::Folder(folder.to_path_buf()); }
        drop.touched = true;
    }

    /// Root mouse move: nothing under the pointer takes files, no word.
    pub(crate) fn drag_target_settle(&mut self) {
        let mut drop = self.drag_target.borrow_mut();
        if !drop.touched { drop.at = crate::DropAt::Nothing; }
        drop.touched = false;
        drop.claimed = false;
    }

    /// Root mouse move, after the hovered elements had their turn.
    pub(crate) fn spring_settle(&mut self) {
        match self.spring.as_mut() {
            Some(spring) if spring.claimed => spring.claimed = false,
            Some(_) => self.spring = None,
            None => {}
        }
    }
}
