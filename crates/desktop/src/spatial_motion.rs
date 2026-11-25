//! Bounded, pane-local FLIP geometry for changing the presentation mode.
//! Only slots actually painted by the virtual list are recorded. Paths, not
//! indices, identify a file across sorting, filtering and presentation changes.
use std::{cell::RefCell, collections::HashMap, path::{Path, PathBuf}, rc::Rc, time::Instant};

use crate::motion_policy::{diagnostic_span, ease_out, ITEM_REFLOW};

const LIMIT: usize = 256;
const MAX_TRAVEL: f32 = 1200.;
const GHOST_LIMIT: usize = 48;

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
struct Slot { natural: Geometry, shown: Geometry, directory: bool, content:Option<Content> }

/// Separate icon and label tracks inside the moving live item. Only bounded
/// geometry is retained; no image assets, live controls or filesystem work.
#[derive(Clone,Copy,Debug,PartialEq)]
pub(crate) struct Content {
    pub icon:Position, pub icon_size:f32,
    pub name:Position, pub name_width:f32, pub font:f32,
}
impl Content {
    pub(crate) fn layout(size:Position,label_chars:usize)->Self {
        let tile=size.y>48.;
        let font=if tile{12.}else{13.};
        let label=(label_chars.min(120) as f32*font*0.55).min((size.x-40.).max(1.));
        if tile {
            let picture=(size.y-52.).max(16.);let icon_size=(picture/2.).round();
            Self{icon:Position{x:(size.x-icon_size)/2.,y:4.+(picture-icon_size)/2.},icon_size,
                name:Position{x:(size.x-label)/2.,y:picture+8.},name_width:label,font}
        } else {
            Self{icon:Position{x:8.,y:(size.y-16.)/2.},icon_size:16.,
                name:Position{x:32.,y:(size.y-16.)/2.},name_width:label,font}
        }
    }
    fn mix(self,target:Self,remaining:f32)->Self {
        let value=|a:f32,b:f32|b+(a-b)*remaining;
        Self{icon:target.icon.plus(self.icon.minus(target.icon).scale(remaining)),
            icon_size:value(self.icon_size,target.icon_size),
            name:target.name.plus(self.name.minus(target.name).scale(remaining)),
            name_width:value(self.name_width,target.name_width),font:value(self.font,target.font)}
    }
}
#[derive(Clone,Copy)]
pub(crate) struct SharedPose {pub size:Position,pub content:Content}

/// Lightweight departure representation: never retains a live row or handler.
pub(crate) struct Ghost {
    pub path: PathBuf,
    pub geometry: Geometry,
    pub directory: bool,
    pub alpha: f32,
}

#[derive(Default)]
struct State {
    folder: PathBuf,
    frame_id: u64,
    current: HashMap<PathBuf, Slot>,
    previous: HashMap<PathBuf, Slot>,
    source: HashMap<PathBuf, Slot>,
    viewport: Option<Geometry>,
    source_viewport: Option<Geometry>,
    scroll:Option<Position>,
    started: Option<Instant>,
}

#[derive(Clone, Default)]
pub(crate) struct SpatialMotion(Rc<RefCell<State>>);

/// A render-frame token prevents delayed callbacks from a replaced view from
/// polluting the current snapshot.
#[derive(Clone)]
pub(crate) struct Frame { state: SpatialMotion, id: u64, at:Instant }

impl SpatialMotion {
    pub(crate) fn begin(&self, folder: &Path, now: Instant) {
        let mut state = self.0.borrow_mut();
        state.source = if state.folder == folder {
            state.current.clone()
        } else { HashMap::new() };
        state.source_viewport = state.viewport;
        state.scroll=None;
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
        state.source_viewport = None;
    }

    pub(crate) fn frame(&self, folder: &Path, now: Instant, animate: bool) -> (Frame, bool) {
        let mut state = self.0.borrow_mut();
        if state.folder != folder {
            state.folder = folder.to_path_buf();
            state.current.clear();
            state.previous.clear();
            state.source.clear();
            state.started = None;
            state.viewport = None;
            state.scroll=None;
            state.source_viewport = None;
        }
        if !animate || state.started.is_some_and(|start| now.saturating_duration_since(start) >= diagnostic_span(ITEM_REFLOW)) {
            state.started = None;
            state.source.clear();
            state.source_viewport = None;
        }
        state.previous = std::mem::take(&mut state.current);
        state.frame_id = state.frame_id.wrapping_add(1);
        (Frame { state: self.clone(), id: state.frame_id, at:now }, state.started.is_some())
    }
}

impl SpatialMotion {
    pub(crate) fn measuring(&self)->bool {
        let state=self.0.borrow();
        state.started.is_some() && state.previous.is_empty() && !state.source.is_empty()
    }

    pub(crate) fn shared_pose(&self,path:&Path,now:Instant)->Option<SharedPose> {
        let state=self.0.borrow();
        let start=state.started?;
        let source=state.source.get(path)?;
        let target=state.previous.get(path)?;
        let chars=path.file_name().map_or(1,|s|s.to_string_lossy().chars().count());
        let remaining=1.-ease_out(now.saturating_duration_since(start),diagnostic_span(ITEM_REFLOW));
        if remaining<=0. || source.shown.origin.minus(target.natural.origin).x.abs()
            .max(source.shown.origin.minus(target.natural.origin).y.abs())>MAX_TRAVEL {return None;}
        let size=source.shown.mix(target.natural,remaining).size;
        let from=source.content.unwrap_or_else(||Content::layout(source.shown.size,chars));
        Some(SharedPose{size,content:from.mix(Content::layout(target.natural.size,chars),remaining)})
    }

    pub(crate) fn observe_scroll(&self,offset:Position) {
        let mut state=self.0.borrow_mut();
        if state.started.is_some() && state.scroll.is_some_and(|old|
            (old.x-offset.x).abs().max((old.y-offset.y).abs())>0.75) {
            state.started=None;state.source.clear();state.source_viewport=None;
        }
        state.scroll=Some(offset);
    }

    pub(crate) fn viewport(&self, geometry: Geometry) {
        if !geometry.valid(){return;}
        let mut state=self.0.borrow_mut();
        if state.source_viewport.is_some_and(|old|
            (old.size.x-geometry.size.x).abs().max((old.size.y-geometry.size.y).abs())>1.) {
            state.started=None;state.source.clear();state.source_viewport=None;
        }
        state.viewport=Some(geometry);
    }

    pub(crate) fn ghosts(&self, folder: &Path, now: Instant) -> Vec<Ghost> {
        let state = self.0.borrow();
        let Some(start) = state.started else { return Vec::new(); };
        let (Some(viewport), Some(source_viewport)) = (state.viewport, state.source_viewport) else { return Vec::new(); };
        if state.folder != folder || (viewport.size.x-source_viewport.size.x).abs()>1.
            || (viewport.size.y-source_viewport.size.y).abs()>1. { return Vec::new(); }
        // Wait until the first destination measurement is available.
        let measuring=state.previous.is_empty();
        let remaining = 1.-ease_out(now.saturating_duration_since(start),diagnostic_span(ITEM_REFLOW));
        if remaining <= 0. {return Vec::new();}
        let mut ghosts: Vec<_> = state.source.iter()
            .filter(|(path,_)| !state.previous.contains_key(*path))
            .filter_map(|(path,slot)| {
                let mut geometry=slot.shown;
                geometry.origin=geometry.origin.minus(source_viewport.origin);
                let p=geometry.origin;let size=geometry.size;
                if p.x>=viewport.size.x || p.y>=viewport.size.y || p.x+size.x<=0. || p.y+size.y<=0. {return None;}
                geometry.origin.y -= 5.*(1.-remaining);
                Some(Ghost { path:path.clone(),geometry,directory:slot.directory,alpha:if measuring{1.}else{0.65*remaining} })
            }).collect();
        ghosts.sort_by(|a,b|a.geometry.origin.y.total_cmp(&b.geometry.origin.y)
            .then_with(||a.geometry.origin.x.total_cmp(&b.geometry.origin.x)).then_with(||a.path.cmp(&b.path)));
        ghosts.truncate(GHOST_LIMIT);
        ghosts
    }
}

impl Frame {
    pub(crate) fn pose(&self, path: &Path, now: Instant) -> Option<(Position, Position)> {
        let state = self.state.0.borrow();
        if state.frame_id != self.id { return None; }
        let start = state.started?;
        let from = state.source.get(path)?.shown;
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

    #[cfg(test)]
    pub(crate) fn record(&self, path: PathBuf, natural: Geometry, pose: Option<(Position, Position)>) {
        self.record_kind(path,natural,pose,false);
    }

    pub(crate) fn record_kind(&self, path: PathBuf, natural: Geometry, pose: Option<(Position, Position)>,directory:bool) {
        let mut state = self.state.0.borrow_mut();
        if state.frame_id != self.id || (state.current.len() >= LIMIT && !state.current.contains_key(&path)) { return; }
        if !natural.valid() { return; }
        let shown = pose.map_or(natural, |(offset, size)| Geometry { origin: natural.origin.plus(offset), size });
        let chars=path.file_name().map_or(1,|s|s.to_string_lossy().chars().count());
        let remaining=state.started.map(|start|1.-ease_out(self.at.saturating_duration_since(start),diagnostic_span(ITEM_REFLOW)));
        let content=remaining.and_then(|remaining| {
            let source=state.source.get(&path)?;
            let from=source.content.unwrap_or_else(||Content::layout(source.shown.size,chars));
            Some(from.mix(Content::layout(natural.size,chars),remaining))
        });
        state.current.insert(path, Slot { natural, shown, directory,content });
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
    fn icon_and_label_tracks_interpolate_independently_and_keep_endpoints() {
        let row=Content::layout(p(400.,24.),12);
        let tile=Content::layout(p(132.,148.),12);
        assert_eq!(row.mix(tile,1.),row);assert_eq!(row.mix(tile,0.),tile);
        let mid=row.mix(tile,0.5);
        assert!(mid.icon_size>row.icon_size && mid.icon_size<tile.icon_size);
        assert!(mid.name.y>row.name.y && mid.name.y<tile.name.y);
        assert_ne!(mid.icon.y-row.icon.y,mid.name.y-row.name.y);
        assert_eq!(mid.mix(row,1.),mid);
    }

    #[test]
    fn scroll_and_viewport_changes_cancel_the_shared_transition() {
        let now=Instant::now();let folder=Path::new("folder");let m=SpatialMotion::default();
        let (old,_)=m.frame(folder,now,true);
        old.record(path("a"),rect(p(10.,20.)),None);
        m.viewport(Geometry{origin:p(0.,0.),size:p(400.,300.)});
        m.begin(folder,now);assert!(m.active());
        m.observe_scroll(p(0.,0.));m.observe_scroll(p(0.,-30.));assert!(!m.active());
        let (frame,_)=m.frame(folder,now,true);frame.record(path("a"),rect(p(10.,20.)),None);
        m.begin(folder,now);assert!(m.active());
        m.viewport(Geometry{origin:p(0.,0.),size:p(500.,300.)});assert!(!m.active());
    }

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
    fn departure_visuals_are_visible_and_never_duplicate_matches() {
        let now=Instant::now();let folder=Path::new("folder");let m=SpatialMotion::default();
        let (old,_)=m.frame(folder,now,true);
        m.viewport(Geometry{origin:p(10.,20.),size:p(300.,300.)});
        old.record_kind(path("leaving"),rect(p(20.,40.)),None,true);
        old.record(path("matching"),rect(p(20.,80.)),None);
        old.record(path("offscreen"),rect(p(20.,800.)),None);
        m.begin(folder,now);
        let (first,_)=m.frame(folder,now,true);
        assert_eq!(m.ghosts(folder,now).len(),2);
        first.record(path("matching"),rect(p(40.,90.)),None);
        let (_next,_)=m.frame(folder,now,true);
        let ghosts=m.ghosts(folder,now);
        assert_eq!(ghosts.len(),1);
        assert_eq!(ghosts[0].path,path("leaving"));
        assert_eq!(ghosts[0].geometry.origin,p(10.,20.));
        assert!(ghosts[0].directory);
        assert!(m.ghosts(Path::new("other"),now).is_empty());
        m.cancel();assert!(m.ghosts(folder,now).is_empty());
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
