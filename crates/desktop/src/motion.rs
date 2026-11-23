//! Short animations that say what just happened. Each runs only for its
//! fraction of a second: while one does, the window asks for the next frame
//! (`Motion::wants_frame`); otherwise nothing is redrawn.
//!
//! - new items (copied, moved, created, extracted) light up and fade;
//! - a folder held under dragged files fills up until it opens;
//! - the copy progress bar glides to each new value;
//! - a new tab widens into place, a closed one narrows away;
//! - a folder's contents come in from the side the user went.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use filemanager_core::panes::GroupId;

/// How long a new item stays lit.
pub(crate) const FLASH: Duration = Duration::from_millis(900);
/// A new item that has not come on screen by then is not waited for.
const FLASH_WAIT: Duration = Duration::from_secs(10);
pub(crate) const TAB_OPEN: Duration = Duration::from_millis(160);
pub(crate) const TAB_CLOSE: Duration = Duration::from_millis(140);
/// Time constant of the progress bar's glide.
const PROGRESS_TAU: f32 = 0.1;
/// How far a folder's contents travel when they come in.
pub(crate) const SLIDE: f32 = 10.;

/// Strength of a new item's light `elapsed` after it came on screen: from
/// full to nothing, fading faster at first.
pub(crate) fn flash_strength(elapsed: Duration) -> Option<f32> {
    if !crate::theme::animations() { return None; }
    let t = elapsed.as_secs_f32() / FLASH.as_secs_f32();
    (t < 1.).then(|| (1. - t) * (1. - t))
}

/// Eased progress of an opening (0 → 1) `elapsed` into `span`.
pub(crate) fn ease_out(elapsed: Duration, span: Duration) -> f32 {
    let t = (elapsed.as_secs_f32() / span.as_secs_f32()).clamp(0., 1.);
    1. - (1. - t).powi(3)
}

/// The bar's next value on the way from `shown` to `real` after `dt` s.
pub(crate) fn glide(shown: f32, real: f32, dt: f32) -> f32 {
    if (real - shown).abs() < 0.002 || real < shown { return real; }
    shown + (real - shown) * (1. - (-dt / PROGRESS_TAU).exp())
}

/// Which way a pane went from `old` to `new`: +1 into a subfolder, -1 out
/// to a parent, 0 elsewhere.
pub(crate) fn direction(old: &Path, new: &Path) -> f32 {
    if new != old && new.starts_with(old) { 1. }
    else if new != old && old.starts_with(new) { -1. }
    else { 0. }
}

/// What a light on an item says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flash {
    /// Made here: copied, moved, created, renamed, extracted.
    New,
    /// Changed by another program or person while on screen.
    Changed,
    /// The folder the pane just came out of.
    Left,
    /// Where a search hit, a feed entry or «Показать в папке» led.
    Found,
    /// An item a transfer or rename could not finish.
    Failed,
}

/// How a new item is looked up: Windows names ignore letter case, and the
/// pane's spelling of a folder may differ from the one a copy reports.
fn flash_key(path: &Path) -> String {
    let path = filemanager_core::path_utils::normalize_extended_path(path);
    let text = path.to_string_lossy();
    if cfg!(windows) { text.to_lowercase() } else { text.into_owned() }
}

#[derive(Default)]
pub(crate) struct Motion {
    pub(crate) layout: crate::layout_motion::LayoutMotion,
    pub(crate) grids: HashMap<GroupId, crate::grid_motion::GridMotion>,
    pub(crate) rows: HashMap<(GroupId,PathBuf),crate::row_motion::RowMotion>,
    pub(crate) lists: HashMap<(GroupId,PathBuf),crate::list_motion::ListMotion>,
    pub(crate) modes: HashMap<GroupId,crate::mode_motion::PaneMotion>,
    /// New items and when each came on screen (`None`: not drawn yet).
    flashes: RefCell<HashMap<String, (Flash, Instant, Option<Instant>)>>,
    opened_tab: Option<(GroupId, usize, Instant)>,
    /// A closed tab's place: its pane, position and width.
    closed_tab: Option<(GroupId, usize, f32, Instant)>,
    /// Tab widths drawn last, per pane: a closed tab narrows from its own.
    /// How many tabs each pane's strip had when last drawn: the layout the
    /// strip's scroll handle keeps is of those.
    tabs_drawn: RefCell<HashMap<GroupId, usize>>,
    /// The progress bar as drawn, and when.
    progress: Option<(f32, Instant)>,
    /// Entrance clocks belong to the pane, not a temporary list element.
    entrances: HashMap<GroupId, Entrance>,
    /// The active tab's line per pane: drawn x and width, their target, and
    /// when it was last drawn.
    pub(crate) tab_lines: Rc<RefCell<HashMap<GroupId, TabLine>>>,
    /// Pictures (thumbnails, the details panel's) and when each arrived.
    pictures: RefCell<HashMap<String, Instant>>,
    /// Set while drawing when something decided only then (the tab line's
    /// new place) still moves: the window asks for one more frame.
    pub(crate) late_frame: Rc<std::cell::Cell<bool>>,
    /// The selection sliding from row to row: pane, folder, from, to, start.
    plate: Option<(GroupId, PathBuf, f32, usize, Instant)>,
}

pub(crate) const LIST_APPEAR: Duration = Duration::from_millis(140);

struct Entrance {
    folder: PathBuf,
    mode: char,
    way: f32,
    from: f32,
    started: Option<Instant>,
}

/// Coverage of the row's text band by a selection at fractional row `at`.
/// Both the row being left and the arriving row use this same clock.
pub(crate) fn plate_ink(at: f32, row: usize) -> f32 {
    let row = row as f32;
    let covered = ((at + 1.).min(row + 0.8) - at.max(row + 0.2)).max(0.) / 0.6;
    let t = covered.clamp(0., 1.);
    t * t * (3. - 2. * t)
}

/// Where tab `index` lies in its strip (x from the strip's left, width),
/// by the layout of the last frame that the strip's scroll handle keeps.
pub(crate) fn tab_place(scroll: &gpui::ScrollHandle, index: usize) -> Option<(f32, f32)> {
    let first = scroll.bounds_for_item(0)?;
    let tab = scroll.bounds_for_item(index)?;
    // The strip's padding is 4 px.
    Some((f32::from(tab.origin.x - first.origin.x) + 4., f32::from(tab.size.width)))
}

/// Whether `group`'s line has to move toward (x, width): after layout, a
/// renamed or added tab can shift it with nothing else being redrawn.
pub(crate) fn line_behind(lines: &RefCell<HashMap<GroupId, TabLine>>, group: GroupId, x: f32, width: f32) -> bool {
    lines.borrow().get(&group).is_some_and(|line| (line.target_x - x).abs() >= 0.5 || (line.target_width - width).abs() >= 0.5)
}

#[derive(Clone, Copy)]
pub(crate) struct TabLine {
    x: f32,
    width: f32,
    target_x: f32,
    target_width: f32,
    drawn: Instant,
}

/// How long a picture takes to come in, and the selection to slide a row.
pub(crate) const PICTURE_IN: Duration = Duration::from_millis(160);
pub(crate) const PLATE: Duration = Duration::from_millis(100);
/// Time constant of the tab line's glide.
const LINE_TAU: f32 = 0.045;

impl Motion {
    /// Lights up `paths` as they appear in their folders.
    pub(crate) fn flash(&self, paths: impl IntoIterator<Item = PathBuf>) {
        self.flash_as(Flash::New, paths);
    }

    /// Lights up `paths` with `kind`. A change another program makes to an
    /// item lit here already (our own copy being written) keeps that light.
    pub(crate) fn flash_as(&self, kind: Flash, paths: impl IntoIterator<Item = PathBuf>) {
        // Animations off: nothing is lit, so nothing is kept to draw frames for.
        if !crate::theme::animations() { return; }
        let now = Instant::now();
        let mut flashes = self.flashes.borrow_mut();
        for path in paths {
            let key = flash_key(&path);
            if kind == Flash::Changed && flashes.get(&key).is_some_and(|(old, _, _)| *old != Flash::Changed) { continue; }
            flashes.insert(key, (kind, now, None));
        }
    }

    /// The light on a row or tile being drawn now; starts its clock.
    pub(crate) fn flash_of(&self, path: &Path) -> Option<(Flash, f32)> {
        let mut flashes = self.flashes.borrow_mut();
        if flashes.is_empty() { return None; }
        let (kind, _, shown) = flashes.get_mut(&flash_key(path))?;
        let since = *shown.get_or_insert_with(Instant::now);
        flash_strength(since.elapsed()).map(|strength| (*kind, strength))
    }

    pub(crate) fn tab_opened(&mut self, group: GroupId, index: usize) {
        if !crate::theme::animations() { return; }
        self.opened_tab = Some((group, index, Instant::now()));
        self.closed_tab = None;
    }

    /// The tab at `index` of `group`, `width` wide, was closed.
    pub(crate) fn tab_closed(&mut self, group: GroupId, index: usize, width: f32) {
        if !crate::theme::animations() { return; }
        self.closed_tab = Some((group, index, width, Instant::now()));
        self.opened_tab = None;
    }

    /// How far open the tab at `index` is (1: fully), while it opens.
    pub(crate) fn tab_opening(&self, group: GroupId, index: usize) -> Option<f32> {
        if !crate::theme::animations() { return None; }
        let (g, i, start) = self.opened_tab?;
        (g == group && i == index && start.elapsed() < TAB_OPEN).then(|| ease_out(start.elapsed(), TAB_OPEN))
    }

    /// The gap a closed tab leaves before `index`: its width now.
    pub(crate) fn tab_gap(&self, group: GroupId) -> Option<(usize, f32)> {
        if !crate::theme::animations() { return None; }
        let (g, index, width, start) = self.closed_tab?;
        (g == group && start.elapsed() < TAB_CLOSE).then(|| (index, width * (1. - ease_out(start.elapsed(), TAB_CLOSE))))
    }

    /// The progress bar's value as drawn this frame.
    pub(crate) fn bar(&self) -> Option<f32> { self.progress.map(|(shown, _)| shown) }

    /// Moves the progress bar one frame toward the real `fraction`.
    pub(crate) fn progress(&mut self, fraction: Option<f64>) -> Option<f32> {
        let Some(real) = fraction.map(|f| f.clamp(0., 1.) as f32) else {
            self.progress = None;
            return None;
        };
        let now = Instant::now();
        let shown = match self.progress {
            Some((shown, at)) if crate::theme::animations() => glide(shown, real, (now - at).as_secs_f32().min(0.1)),
            Some(_) => real,
            None => real,
        };
        self.progress = Some((shown, now));
        Some(shown)
    }

    /// Observe navigation even while the listing is loading or empty. A
    /// watcher/F5 may temporarily remove the list, but cannot reset this clock.
    pub(crate) fn list_location(&mut self, group: GroupId, path: &Path, mode: char) {
        if self.entrances.get(&group).is_some_and(|e| e.folder == path && e.mode == mode) { return; }
        let old = self.entrances.get(&group);
        let changed = old.is_some_and(|e| e.mode != mode);
        let switch = changed && old.is_some_and(|e|e.folder==path);
        let way = old.map_or(0., |e| direction(&e.folder, path));
        self.entrances.insert(group, Entrance { folder: path.to_path_buf(), mode,
            way: if mode == 'c' { 0. } else { way },
            from: if switch {1.} else if changed { 0.1 } else { 0.35 }, started: switch.then(||Instant::now()-LIST_APPEAR) });
    }

    /// (opacity, horizontal offset), starting when the contents first arrive.
    pub(crate) fn list_appearance(&mut self, group: GroupId) -> (f32, f32) {
        let Some(e) = self.entrances.get_mut(&group) else { return (1., 0.); };
        let started = *e.started.get_or_insert_with(Instant::now);
        if !crate::theme::animations() || e.mode == 'c' { return (1., 0.); }
        let t = ease_out(started.elapsed(), LIST_APPEAR);
        if t < 1. { self.late_frame.set(true); }
        (e.from + (1. - e.from) * t, e.way * SLIDE * (1. - t))
    }

    /// Notes that `group`'s strip is drawn with `count` tabs; true when it
    /// had as many last time (the kept layout is of these tabs).
    pub(crate) fn tabs_drawn(&self, group: GroupId, count: usize) -> bool {
        self.tabs_drawn.borrow_mut().insert(group, count) == Some(count)
    }

    /// Where to draw the active tab's line in `group` this frame, on its
    /// way to `target` (x, width): it slides when another tab becomes active
    /// or the tabs move. A pane seen for the first time gets it in place.
    pub(crate) fn tab_line(&self, group: GroupId, target_x: f32, target_width: f32) -> (f32, f32) {
        let now = Instant::now();
        let mut lines = self.tab_lines.borrow_mut();
        let line = lines.entry(group).or_insert(TabLine { x: target_x, width: target_width, target_x, target_width, drawn: now });
        let dt = (now - line.drawn).as_secs_f32().min(0.05);
        // Not drawn for a while (the pane was hidden): no slide from the past.
        let fresh = now - line.drawn < Duration::from_millis(500);
        let step = |from: f32, to: f32| {
            if !crate::theme::animations() || !fresh || (to - from).abs() < 0.5 { to } else { from + (to - from) * (1. - (-dt / LINE_TAU).exp()) }
        };
        line.x = step(line.x, target_x);
        line.width = step(line.width, target_width);
        line.target_x = target_x;
        line.target_width = target_width;
        line.drawn = now;
        // The target was learnt while drawing: ask for the frame the window
        // did not know it needed.
        if (line.x - target_x).abs() >= 0.5 || (line.width - target_width).abs() >= 0.5 { self.late_frame.set(true); }
        (line.x, line.width)
    }

    /// A picture for `key` has just arrived: it comes in softly.
    pub(crate) fn picture_arrived(&self, key: String) {
        if !crate::theme::animations() { return; }
        self.pictures.borrow_mut().insert(key, Instant::now());
    }

    /// How far in the picture for `key` is (1: fully, or never tracked).
    pub(crate) fn picture_opacity(&self, key: &str) -> f32 {
        self.pictures.borrow().get(key).map_or(1., |arrived| ease_out(arrived.elapsed(), PICTURE_IN))
    }

    /// The selection moved by keys from row `from` to row `to` of `folder`.
    pub(crate) fn plate_moved(&mut self, group: GroupId, folder: &Path, from: usize, to: usize) {
        if !crate::theme::animations() || from == to || from.abs_diff(to) > 3 { self.plate = None; return; }
        // Repeated/reversed keys continue from the position actually drawn.
        let shown = self.plate_at(group, folder).map_or(from as f32, |(at, _)| at);
        self.plate = Some((group, folder.to_path_buf(), shown, to, Instant::now()));
    }

    pub(crate) fn cancel_plate(&mut self) { self.plate = None; }

    /// Where the sliding selection is in `folder` of `group` (in rows), and
    /// the row it goes to; None once it is there.
    pub(crate) fn plate_at(&self, group: GroupId, folder: &Path) -> Option<(f32, usize)> {
        let (g, f, from, to, start) = self.plate.as_ref()?;
        if *g != group || f != folder || start.elapsed() >= PLATE { return None; }
        let t = ease_out(start.elapsed(), PLATE);
        Some((*from + (*to as f32 - *from) * t, *to))
    }

    /// Whether something is moving and the next frame must be drawn. Also
    /// drops what has finished.
    pub(crate) fn wants_frame(&mut self, fraction: Option<f64>) -> bool {
        let mut flashes = self.flashes.borrow_mut();
        flashes.retain(|_, (_, added, shown)| match shown {
            Some(shown) => shown.elapsed() < FLASH,
            None => added.elapsed() < FLASH_WAIT,
        });
        let flashing = flashes.values().any(|(_, _, shown)| shown.is_some());
        drop(flashes);
        if self.opened_tab.is_some_and(|(_, _, start)| start.elapsed() >= TAB_OPEN) { self.opened_tab = None; }
        if self.closed_tab.is_some_and(|(_, _, _, start)| start.elapsed() >= TAB_CLOSE) { self.closed_tab = None; }
        let gliding = self.progress_moving(fraction);
        // A line of a pane no longer drawn is let go.
        let mut lines = self.tab_lines.borrow_mut();
        lines.retain(|_, line| line.drawn.elapsed() < Duration::from_secs(2));
        let line_moving = lines.values().any(|line| (line.x - line.target_x).abs() >= 0.5 || (line.width - line.target_width).abs() >= 0.5);
        drop(lines);
        let mut pictures = self.pictures.borrow_mut();
        pictures.retain(|_, arrived| arrived.elapsed() < PICTURE_IN * 3);
        let pictures_coming = pictures.values().any(|arrived| arrived.elapsed() < PICTURE_IN);
        drop(pictures);
        if self.plate.as_ref().is_some_and(|plate| plate.4.elapsed() >= PLATE) { self.plate = None; }
        let entering = crate::theme::animations() && self.entrances.values().any(|e|
            e.mode != 'c' && e.started.is_some_and(|at| at.elapsed() < LIST_APPEAR));
        entering || flashing || self.opened_tab.is_some() || self.closed_tab.is_some() || gliding || line_moving || pictures_coming || self.plate.is_some()
    }

    /// Whether the progress bar still has a way to go to `fraction`.
    pub(crate) fn progress_moving(&self, fraction: Option<f64>) -> bool {
        match (self.progress, fraction) {
            (Some((shown, _)), Some(real)) => (real as f32 - shown).abs() >= 0.002,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flash_fades_out_and_ends() {
        assert_eq!(flash_strength(Duration::ZERO), Some(1.));
        let half = flash_strength(FLASH / 2).unwrap();
        assert!(half > 0.2 && half < 0.3, "{half}");
        assert_eq!(flash_strength(FLASH), None);
    }

    #[test]
    fn a_flash_starts_when_drawn() {
        let motion = Motion::default();
        motion.flash([PathBuf::from("/w/new.txt")]);
        assert_eq!(motion.flash_of(Path::new("/w/other.txt")), None);
        let (kind, first) = motion.flash_of(Path::new("/w/new.txt")).unwrap();
        assert!(kind == Flash::New && first > 0.9);
        // The watcher then reports the same file: it stays lit as new.
        motion.flash_as(Flash::Changed, [PathBuf::from("/w/new.txt"), PathBuf::from("/w/theirs.txt")]);
        assert_eq!(motion.flash_of(Path::new("/w/new.txt")).unwrap().0, Flash::New);
        assert_eq!(motion.flash_of(Path::new("/w/theirs.txt")).unwrap().0, Flash::Changed);
    }

    #[test]
    fn the_bar_glides_forward_and_jumps_back() {
        let mut shown = 0.;
        for _ in 0..3 { shown = glide(shown, 0.5, 0.016); }
        assert!(shown > 0.1 && shown < 0.5, "{shown}");
        for _ in 0..60 { shown = glide(shown, 0.5, 0.016); }
        assert_eq!(shown, 0.5);
        // A new transfer starts over at once.
        assert_eq!(glide(0.8, 0.1, 0.016), 0.1);
    }

    #[test]
    fn the_way_a_pane_went() {
        assert_eq!(direction(Path::new("/a"), Path::new("/a/b")), 1.);
        assert_eq!(direction(Path::new("/a/b"), Path::new("/a")), -1.);
        assert_eq!(direction(Path::new("/a/b"), Path::new("/c")), 0.);

    }

    #[test]
    fn the_tab_line_slides_to_its_tab() {
        let motion = Motion::default();
        let group = GroupId(1);
        assert_eq!(motion.tab_line(group, 10., 80.), (10., 80.));
        std::thread::sleep(Duration::from_millis(16));
        let (x, width) = motion.tab_line(group, 110., 60.);
        assert!(x > 10. && x < 110. && width < 80. && width > 60., "{x} {width}");
        for _ in 0..80 { std::thread::sleep(Duration::from_millis(5)); motion.tab_line(group, 110., 60.); }
        assert_eq!(motion.tab_line(group, 110., 60.), (110., 60.));
    }

    #[test]
    fn the_plate_slides_a_few_rows_only() {
        let mut motion = Motion::default();
        let group = GroupId(1);
        motion.plate_moved(group, Path::new("/w"), 3, 4);
        let (at, to) = motion.plate_at(group, Path::new("/w")).unwrap();
        assert!((3. ..=4.).contains(&at) && to == 4);
        assert!(motion.plate_at(group, Path::new("/other")).is_none());
        // A jump (Home, End, PageDown) does not slide.
        motion.plate_moved(group, Path::new("/w"), 0, 40);
        assert!(motion.plate_at(group, Path::new("/w")).is_none());
    }

    #[test]
    fn eases_out() {
        assert_eq!(ease_out(Duration::ZERO, TAB_OPEN), 0.);
        assert_eq!(ease_out(TAB_OPEN, TAB_OPEN), 1.);
        assert!(ease_out(TAB_OPEN / 2, TAB_OPEN) > 0.5);
    }

    #[test]
    fn rereading_a_listing_cannot_replay_its_entrance() {
        let mut motion = Motion::default(); let g = GroupId(1);
        motion.list_location(g, Path::new("/a"), 'l');
        // Loading does not use up the clock; first appearance starts it.
        assert!(motion.entrances[&g].started.is_none());
        motion.list_appearance(g);
        let finished = Instant::now() - LIST_APPEAR * 2;
        motion.entrances.get_mut(&g).unwrap().started = Some(finished);
        // No list element while refreshing; return with the same location.
        motion.list_location(g, Path::new("/a"), 'l');
        assert_eq!(motion.list_appearance(g), (1., 0.));
        assert_eq!(motion.entrances[&g].started, Some(finished));
        motion.list_location(g, Path::new("/a/empty"), 'l');
        motion.list_location(g, Path::new("/a"), 'l');
        assert!(motion.list_appearance(g).1 < 0., "return from an empty folder is a new opening");
        motion.list_location(g, Path::new("/a"), 'c');
        assert_eq!(motion.list_appearance(g), (1., 0.));
    }

    #[test]
    fn selection_text_and_repeated_keys_follow_the_drawn_plate() {
        assert_eq!(plate_ink(2., 2), 1.);
        assert_eq!(plate_ink(2., 3), 0.);
        assert!((plate_ink(2.5, 2) - 0.5).abs() < 0.001);
        assert!((plate_ink(2.5, 3) - 0.5).abs() < 0.001);
        let mut previous = 0.;
        for step in 0..=100 {
            let now = plate_ink(2. + step as f32 / 100., 3);
            assert!(now >= previous && now - previous < 0.04);
            previous = now;
        }
        let mut motion = Motion::default(); let g = GroupId(1); let f = Path::new("/a");
        motion.plate_moved(g, f, 2, 3);
        motion.plate.as_mut().unwrap().4 = Instant::now() - PLATE / 3;
        let shown = motion.plate_at(g, f).unwrap().0;
        motion.plate_moved(g, f, 3, 2);
        let reversed = motion.plate_at(g, f).unwrap().0;
        assert!((shown - reversed).abs() < 0.05);
        motion.cancel_plate(); assert!(motion.plate_at(g, f).is_none());
    }
}

#[cfg(test)]
mod mode_entrance_tests {
    use super::*;
    #[test] fn switching_modes_in_place_does_not_start_another_folder_entrance() {
        let mut motion=Motion::default();let id=GroupId(1);let path=Path::new("/same");
        motion.list_location(id,path,'l');motion.list_appearance(id);
        motion.list_location(id,path,'t');assert_eq!(motion.list_appearance(id),(1.,0.));
        motion.list_location(id,path,'c');assert_eq!(motion.list_appearance(id),(1.,0.));
    }
}
