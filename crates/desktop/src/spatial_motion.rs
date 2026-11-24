//! Bounded, pane-local FLIP geometry for changing the presentation mode.
//! Only slots actually painted by the virtual list are recorded. Paths, not
//! indices, identify a file across sorting, filtering and presentation changes.
use std::{cell::RefCell, collections::HashMap, path::{Path, PathBuf}, rc::Rc, time::Instant};

use crate::motion_policy::{diagnostic_span, ease_out, ITEM_REFLOW};

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
pub(crate) struct Geometry { pub origin: Position, pub size: Position }

impl Geometry {
    fn valid(self) -> bool {
        self.origin.x.is_finite() && self.origin.y.is_finite() && self.size.x.is_finite()
            && self.size.y.is_finite() && self.size.x > 0. && self.size.y > 0.
    }

    fn mix(self, target: Self, remaining: f32) -> Self {
        Self { origin: target.origin.plus(self.origin.minus(target.origin).scale(remaining)),
            size: target.size.plus(self.size.minus(target.size).scale(remaining)) }
    }
}

#[derive(Clone, Copy)]
struct Slot { natural: Geometry, shown: Geometry }

#[derive(Default)]
struct State {
    folder: PathBuf,
    frame_id: u64,
    current: HashMap<PathBuf, Slot>,
    previous: HashMap<PathBuf, Slot>,
    source: HashMap<PathBuf, Geometry>,
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
        if !animate || state.started.is_some_and(|start| now.saturating_duration_since(start) >= diagnostic_span(ITEM_REFLOW)) {
            state.started = None;
            state.source.clear();
        }
        state.previous = std::mem::take(&mut state.current);
        state.frame_id = state.frame_id.wrapping_add(1);
        (Frame { state: self.clone(), id: state.frame_id }, state.started.is_some())
    }
}

impl Frame {
    pub(crate) fn pose(&self, path: &Path, now: Instant) -> Option<(Position, Position)> {
        let state = self.state.0.borrow();
        if state.frame_id != self.id { return None; }
        let start = state.started?;
        let from = *state.source.get(path)?;
        let target = state.previous.get(path)?.natural;
        let delta = from.origin.minus(target.origin);
        if !from.valid() || !target.valid() || delta.x.abs().max(delta.y.abs()) > MAX_TRAVEL {
            return None;
        }
        let remaining = 1. - ease_out(now.saturating_duration_since(start), diagnostic_span(ITEM_REFLOW));
        if remaining <= 0. { return None; }
        let shown = from.mix(target, remaining);
        Some((shown.origin.minus(target.origin), shown.size))
    }

    pub(crate) fn record(&self, path: PathBuf, natural: Geometry, pose: Option<(Position, Position)>) {
        let mut state = self.state.0.borrow_mut();
        if state.frame_id != self.id || (state.current.len() >= LIMIT && !state.current.contains_key(&path)) { return; }
        if !natural.valid() { return; }
        let shown = pose.map_or(natural, |(offset, size)| Geometry { origin: natural.origin.plus(offset), size });
        state.current.insert(path, Slot { natural, shown });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(x: f32, y: f32) -> Position { Position { x, y } }
    fn path(s: &str) -> PathBuf { PathBuf::from(s) }
    fn rect(origin: Position) -> Geometry { Geometry { origin, size: p(200.,24.) } }
    fn offset(frame: &Frame, path: &Path, now: Instant) -> Position { frame.pose(path, now).map_or(Position::default(), |v|v.0) }

    #[test]
    fn matched_items_move_and_unmatched_items_do_not() {
        let now = Instant::now();
        let motion = SpatialMotion::default();
        let folder = Path::new("folder");
        let (old, _) = motion.frame(folder, now, true);
        old.record(path("a"), rect(p(100., 80.)), None);
        motion.begin(folder, now);
        let (first, active) = motion.frame(folder, now, true);
        assert!(active);
        first.record(path("a"), rect(p(10., 20.)), None);
        let (next, _) = motion.frame(folder, now, true);
        assert_eq!(offset(&next, Path::new("a"), now), p(90., 60.));
        assert_eq!(offset(&next, Path::new("other"), now), Position::default());
        let mid = offset(&next, Path::new("a"), now + ITEM_REFLOW / 2);
        assert!(mid.x > 0. && mid.x < 90.);
        assert_eq!(offset(&next, Path::new("a"), now + ITEM_REFLOW), Position::default());
    }

    #[test]
    fn reversing_uses_the_displayed_not_the_original_position() {
        let now = Instant::now();
        let motion = SpatialMotion::default();
        let folder = Path::new("folder");
        let (old, _) = motion.frame(folder, now, true);
        old.record(path("a"), rect(p(100., 80.)), None);
        motion.begin(folder, now);
        let (first, _) = motion.frame(folder, now, true);
        first.record(path("a"), rect(p(10., 20.)), Some((p(40.,30.),p(120.,80.))));
        motion.begin(folder, now);
        let (next, _) = motion.frame(folder, now, true);
        next.record(path("a"), rect(p(100., 80.)), None);
        let (next, _) = motion.frame(folder, now, true);
        assert_eq!(offset(&next, Path::new("a"), now), p(-50., -30.));
    }

    #[test]
    fn dimensions_interpolate_and_retarget_from_shown_geometry() {
        let now = Instant::now();
        let folder = Path::new("folder");
        let m = SpatialMotion::default();
        let (old, _) = m.frame(folder,now,true);
        old.record(path("a"), rect(p(20.,30.)),None);
        m.begin(folder,now);
        let target = Geometry { origin:p(100.,100.),size:p(96.,120.) };
        let (first,_) = m.frame(folder,now,true);
        first.record(path("a"),target,None);
        let (next,_) = m.frame(folder,now,true);
        assert_eq!(next.pose(Path::new("a"),now),Some((p(-80.,-70.),p(200.,24.))));
        let mid = now + ITEM_REFLOW / 2;
        let pose = next.pose(Path::new("a"),mid).unwrap();
        assert!(pose.1.x > 96. && pose.1.x < 200. && pose.1.y > 24. && pose.1.y < 120.);
        next.record(path("a"),target,Some(pose));
        m.begin(folder,mid);
        let (first,_) = m.frame(folder,mid,true);
        first.record(path("a"),rect(p(20.,30.)),None);
        let (next,_) = m.frame(folder,mid,true);
        assert_eq!(next.pose(Path::new("a"),mid).unwrap().1,pose.1);
        assert!(next.pose(Path::new("a"),mid+ITEM_REFLOW).is_none());
    }

    #[test]
    fn changed_folder_stale_callbacks_and_disabled_motion_are_safe() {
        let now = Instant::now();
        let motion = SpatialMotion::default();
        let folder = Path::new("folder");
        let (old, _) = motion.frame(folder, now, true);
        old.record(path("a"), rect(p(100., 80.)), None);
        motion.begin(folder, now);
        let (next, active) = motion.frame(Path::new("new"), now, true);
        assert!(!active);
        old.record(path("a"), rect(p(100., 80.)), None);
        assert_eq!(offset(&next, Path::new("a"), now), Position::default());
        next.record(path("a"), rect(p(0., 0.)), None);
        motion.begin(Path::new("new"), now);
        let (next, active) = motion.frame(Path::new("new"), now, false);
        assert!(!active);
        assert_eq!(offset(&next, Path::new("a"), now), Position::default());
    }
}
