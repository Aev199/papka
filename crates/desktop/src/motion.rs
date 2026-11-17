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
    /// New items and when each came on screen (`None`: not drawn yet).
    flashes: RefCell<HashMap<String, (Flash, Instant, Option<Instant>)>>,
    opened_tab: Option<(GroupId, usize, Instant)>,
    /// A closed tab's place: its pane, position and width.
    closed_tab: Option<(GroupId, usize, f32, Instant)>,
    /// Tab widths drawn last, per pane: a closed tab narrows from its own.
    pub(crate) tab_widths: Rc<RefCell<HashMap<GroupId, Vec<f32>>>>,
    /// The progress bar as drawn, and when.
    progress: Option<(f32, Instant)>,
    /// Where each pane was, and which way it went last.
    places: HashMap<GroupId, (PathBuf, f32)>,
}

impl Motion {
    /// Lights up `paths` as they appear in their folders.
    pub(crate) fn flash(&self, paths: impl IntoIterator<Item = PathBuf>) {
        self.flash_as(Flash::New, paths);
    }

    /// Lights up `paths` with `kind`. A change another program makes to an
    /// item lit here already (our own copy being written) keeps that light.
    pub(crate) fn flash_as(&self, kind: Flash, paths: impl IntoIterator<Item = PathBuf>) {
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
        self.opened_tab = Some((group, index, Instant::now()));
        self.closed_tab = None;
    }

    pub(crate) fn tab_closed(&mut self, group: GroupId, index: usize) {
        let width = self.tab_widths.borrow().get(&group).and_then(|widths| widths.get(index).copied()).unwrap_or(120.);
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

    /// Notes where `group` is; returns the way it went when that changed
    /// (kept until the next change, for the fade-in of the new contents).
    pub(crate) fn went(&mut self, group: GroupId, path: &Path) -> f32 {
        match self.places.get_mut(&group) {
            Some((old, way)) if old.as_path() != path => {
                *way = direction(old, path);
                *old = path.to_path_buf();
                *way
            }
            Some((_, way)) => *way,
            None => {
                self.places.insert(group, (path.to_path_buf(), 0.));
                0.
            }
        }
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
        flashing || self.opened_tab.is_some() || self.closed_tab.is_some() || gliding
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
        let mut motion = Motion::default();
        let group = GroupId(1);
        assert_eq!(motion.went(group, Path::new("/a")), 0.);
        assert_eq!(motion.went(group, Path::new("/a/b")), 1.);
        // Redrawing the same folder keeps the way it came in.
        assert_eq!(motion.went(group, Path::new("/a/b")), 1.);
        assert_eq!(motion.went(group, Path::new("/a")), -1.);
    }

    #[test]
    fn eases_out() {
        assert_eq!(ease_out(Duration::ZERO, TAB_OPEN), 0.);
        assert_eq!(ease_out(TAB_OPEN, TAB_OPEN), 1.);
        assert!(ease_out(TAB_OPEN / 2, TAB_OPEN) > 0.5);
    }
}
