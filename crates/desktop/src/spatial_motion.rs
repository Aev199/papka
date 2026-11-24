//! Bounded, pane-local FLIP geometry for changing the presentation mode.
//! Only slots actually painted by the virtual list are recorded. Paths, not
//! indices, identify a file across sorting, filtering and presentation changes.
use std::{cell::RefCell, collections::HashMap, path::{Path, PathBuf}, rc::Rc, time::Instant};

use crate::motion_policy::{ease_out, ITEM_REFLOW};

const LIMIT: usize = 256;
const MAX_TRAVEL: f32 = 1200.;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Position { pub x: f32, pub y: f32 }

impl Position {
    pub(crate) fn plus(self, other: Self) -> Self { Self { x: self.x + other.x, y: self.y + other.y } }
    fn minus(self, other: Self) -> Self { Self { x: self.x - other.x, y: self.y - other.y } }
    fn scale(self, t: f32) -> Self { Self { x: self.x * t, y: self.y * t } }
}

#[derive(Clone, Copy)]
struct Slot { natural: Position, shown: Position }

#[derive(Default)]
struct State {
    folder: PathBuf,
    frame_id: u64,
    current: HashMap<PathBuf, Slot>,
    previous: HashMap<PathBuf, Slot>,
    source: HashMap<PathBuf, Position>,
    started: Option<Instant>,
}

#[derive(Clone, Default)]
pub(crate) struct SpatialMotion(Rc<RefCell<State>>);

/// A render-frame token prevents delayed callbacks from a replaced view from
/// polluting the current snapshot.
#[derive(Clone)]
pub(crate) struct Frame { state: SpatialMotion, id: u64 }

impl SpatialMotion {
    pub(crate) fn begin(&self, folder: &Path, now: Instant) {
        let mut state = self.0.borrow_mut();
        state.source = if state.folder == folder {
            state.current.iter().map(|(path, slot)| (path.clone(), slot.shown)).collect()
        } else { HashMap::new() };
        state.started = (!state.source.is_empty()).then_some(now);
        state.current.clear();
        state.previous.clear();
        state.frame_id = state.frame_id.wrapping_add(1);
    }

    pub(crate) fn active(&self) -> bool { self.0.borrow().started.is_some() }

    pub(crate) fn cancel(&self) {
        let mut state = self.0.borrow_mut();
        state.started = None;
        state.source.clear();
    }

    pub(crate) fn frame(&self, folder: &Path, now: Instant, animate: bool) -> (Frame, bool) {
        let mut state = self.0.borrow_mut();
        if state.folder != folder {
            state.folder = folder.to_path_buf();
            state.current.clear();
            state.previous.clear();
            state.source.clear();
            state.started = None;
        }
        if !animate || state.started.is_some_and(|start| now.saturating_duration_since(start) >= ITEM_REFLOW) {
            state.started = None;
            state.source.clear();
        }
        state.previous = std::mem::take(&mut state.current);
        state.frame_id = state.frame_id.wrapping_add(1);
        (Frame { state: self.clone(), id: state.frame_id }, state.started.is_some())
    }
}

impl Frame {
    pub(crate) fn offset(&self, path: &Path, now: Instant) -> Position {
        let state = self.state.0.borrow();
        if state.frame_id != self.id { return Position::default(); }
        let Some(start) = state.started else { return Position::default(); };
        let Some(from) = state.source.get(path) else { return Position::default(); };
        let Some(target) = state.previous.get(path) else { return Position::default(); };
        let delta = from.minus(target.natural);
        if !delta.x.is_finite() || !delta.y.is_finite() || delta.x.abs().max(delta.y.abs()) > MAX_TRAVEL {
            return Position::default();
        }
        delta.scale(1. - ease_out(now.saturating_duration_since(start), ITEM_REFLOW))
    }

    pub(crate) fn record(&self, path: PathBuf, natural: Position, offset: Position) {
        let mut state = self.state.0.borrow_mut();
        if state.frame_id != self.id || (state.current.len() >= LIMIT && !state.current.contains_key(&path)) { return; }
        state.current.insert(path, Slot { natural, shown: natural.plus(offset) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(x: f32, y: f32) -> Position { Position { x, y } }
    fn path(s: &str) -> PathBuf { PathBuf::from(s) }

    #[test]
    fn matched_items_move_and_unmatched_items_do_not() {
        let now = Instant::now();
        let motion = SpatialMotion::default();
        let folder = Path::new("folder");
        let (old, _) = motion.frame(folder, now, true);
        old.record(path("a"), p(100., 80.), Position::default());
        motion.begin(folder, now);
        let (first, active) = motion.frame(folder, now, true);
        assert!(active);
        first.record(path("a"), p(10., 20.), Position::default());
        let (next, _) = motion.frame(folder, now, true);
        assert_eq!(next.offset(Path::new("a"), now), p(90., 60.));
        assert_eq!(next.offset(Path::new("other"), now), Position::default());
        let mid = next.offset(Path::new("a"), now + ITEM_REFLOW / 2);
        assert!(mid.x > 0. && mid.x < 90.);
        assert_eq!(next.offset(Path::new("a"), now + ITEM_REFLOW), Position::default());
    }

    #[test]
    fn reversing_uses_the_displayed_not_the_original_position() {
        let now = Instant::now();
        let motion = SpatialMotion::default();
        let folder = Path::new("folder");
        let (old, _) = motion.frame(folder, now, true);
        old.record(path("a"), p(100., 80.), Position::default());
        motion.begin(folder, now);
        let (first, _) = motion.frame(folder, now, true);
        first.record(path("a"), p(10., 20.), p(40., 30.));
        motion.begin(folder, now);
        let (next, _) = motion.frame(folder, now, true);
        next.record(path("a"), p(100., 80.), Position::default());
        let (next, _) = motion.frame(folder, now, true);
        assert_eq!(next.offset(Path::new("a"), now), p(-50., -30.));
    }

    #[test]
    fn changed_folder_stale_callbacks_and_disabled_motion_are_safe() {
        let now = Instant::now();
        let motion = SpatialMotion::default();
        let folder = Path::new("folder");
        let (old, _) = motion.frame(folder, now, true);
        old.record(path("a"), p(100., 80.), Position::default());
        motion.begin(folder, now);
        let (next, active) = motion.frame(Path::new("new"), now, true);
        assert!(!active);
        old.record(path("a"), p(100., 80.), Position::default());
        assert_eq!(next.offset(Path::new("a"), now), Position::default());
        next.record(path("a"), p(0., 0.), Position::default());
        motion.begin(Path::new("new"), now);
        let (next, active) = motion.frame(Path::new("new"), now, false);
        assert!(!active);
        assert_eq!(next.offset(Path::new("a"), now), Position::default());
    }
}
