//! Retained opacity for switching a pane's presentation mode.
use std::time::{Duration, Instant};

const FADE_OUT: Duration = crate::motion::MODE_FADE_OUT;
const FADE_IN: Duration = crate::motion::MODE_FADE_IN;
// Keep a faint layout at the handover: no fully empty/white frame.
const LOW:f32=0.12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode { List, Columns, Thumbnails }

#[derive(Clone, Copy, Debug)]
enum Phase {
    Stable(Mode),
    Out { from: Mode, to: Mode, started: Instant, opacity: f32 },
    In { mode: Mode, started: Instant, opacity: f32 },
}

/// Pane-local transition. The caller commits its model mode only when `sample`
/// reports a destination, keeping the outgoing layout alive until hidden.
pub(crate) struct ModeMotion { phase: Phase }
impl Default for ModeMotion { fn default() -> Self { Self { phase: Phase::Stable(Mode::List) } } }

impl ModeMotion {
    pub(crate) fn new(mode: Mode) -> Self { Self { phase: Phase::Stable(mode) } }

    pub(crate) fn active(&self) -> bool { !matches!(self.phase, Phase::Stable(_)) }

    /// Reset retained state to the model's current mode after an external change.

    /// `current` is the rendered model mode, not a queued destination.
    pub(crate) fn begin(&mut self, current: Mode, target: Mode, now: Instant, animate: bool) {
        let opacity = self.opacity_at(now);
        if target == current {
            self.phase = if opacity >= 1. || !animate { Phase::Stable(current) }
                else { Phase::In { mode: current, started: now, opacity } };
            return;
        }
        if !animate {
            self.phase = Phase::Out { from: current, to: target, started: now, opacity: 0. };
            return;
        }
        match self.phase {
            Phase::Out { from, started, opacity: initial, .. } if from == current => {
                self.phase = Phase::Out { from, to: target, started, opacity: initial };
            }
            Phase::In { mode, .. } if mode == current => {
                self.phase = Phase::Out { from: current, to: target, started: now, opacity };
            }
            _ => self.phase = Phase::Out { from: current, to: target, started: now, opacity },
        }
    }

    /// Returns `(opacity, rendered mode, mode to commit)`. Call once per frame.
    pub(crate) fn sample(&mut self, now: Instant, animate: bool) -> (f32, Mode, Option<Mode>) {
        match self.phase {
            Phase::Stable(mode) => (1., mode, None),
            Phase::Out { from, to, started, opacity } => {
                if !animate { self.phase = Phase::Stable(to); return (1., to, Some(to)); }
                let t = progress(now, started, FADE_OUT);
                let value = opacity + (LOW-opacity)*ease(t);
                if t >= 1. {
                    self.phase = Phase::In { mode: to, started: now, opacity: LOW };
                    (LOW, from, Some(to))
                } else { (value, from, None) }
            }
            Phase::In { mode, started, opacity } => {
                if !animate { self.phase = Phase::Stable(mode); return (1., mode, None); }
                let t = progress(now, started, FADE_IN);
                let value = opacity + (1. - opacity) * ease(t);
                if t >= 1. { self.phase = Phase::Stable(mode); (1., mode, None) }
                else { (value, mode, None) }
            }
        }
    }

    fn opacity_at(&self, now: Instant) -> f32 {
        match self.phase {
            Phase::Stable(_) => 1.,
            Phase::Out { started, opacity, .. } => opacity + (LOW-opacity)*ease(progress(now, started, FADE_OUT)),
            Phase::In { started, opacity, .. } => opacity + (1. - opacity) * ease(progress(now, started, FADE_IN)),
        }
    }
}

fn progress(now: Instant, started: Instant, duration: Duration) -> f32 {
    (now.saturating_duration_since(started).as_secs_f32() / duration.as_secs_f32()).clamp(0., 1.)
}
use crate::motion::ease_out_fraction as ease;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_switches_replace_the_fadeout_destination() {
        let now = Instant::now(); let mut motion = ModeMotion::default();
        motion.begin(Mode::List, Mode::Columns, now, true);
        motion.begin(Mode::List, Mode::Thumbnails, now + Duration::from_millis(25), true);
        assert_eq!(motion.sample(now + FADE_OUT, true), (LOW, Mode::List, Some(Mode::Thumbnails)));
    }

    #[test]
    fn reversing_during_fadeout_has_no_opacity_jump() {
        let now = Instant::now(); let mut motion = ModeMotion::default();
        motion.begin(Mode::List, Mode::Columns, now, true);
        let at = now + Duration::from_millis(30); let before = motion.sample(at, true).0;
        motion.begin(Mode::List, Mode::List, at, true);
        assert_eq!(motion.sample(at, true).0, before);
        assert!(motion.sample(at + Duration::from_millis(40), true).0 > before);
    }

    #[test]
    fn disabling_animation_commits_immediately() {
        let now = Instant::now(); let mut motion = ModeMotion::default();
        motion.begin(Mode::List, Mode::Columns, now, false);
        assert_eq!(motion.sample(now, false), (1., Mode::Columns, Some(Mode::Columns)));
        assert_eq!(motion.sample(now, false), (1., Mode::Columns, None));
    }

    #[test]
    fn fadein_retargeting_starts_at_current_opacity_and_endpoints_do_not_replay() {
        let now = Instant::now(); let mut motion = ModeMotion::default();
        motion.begin(Mode::List, Mode::Columns, now, true);
        assert_eq!(motion.sample(now + FADE_OUT, true).2, Some(Mode::Columns));
        let at = now + FADE_OUT + Duration::from_millis(40); let before = motion.sample(at, true).0;
        motion.begin(Mode::Columns, Mode::Thumbnails, at, true);
        assert_eq!(motion.sample(at, true).0, before);
        assert_eq!(motion.sample(at + FADE_OUT, true).2, Some(Mode::Thumbnails));
        let end = at + FADE_OUT + FADE_IN + Duration::from_secs(1);
        assert_eq!(motion.sample(end, true), (1., Mode::Thumbnails, None));
        assert_eq!(motion.sample(end, true), (1., Mode::Thumbnails, None));
    }
}

pub(crate) struct PaneMotion {
    pub transition:ModeMotion,
    pub anchor:Option<std::path::PathBuf>,
    pub folder:std::path::PathBuf,
}
