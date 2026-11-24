//! Retained horizontal reveal for Miller columns. External offset changes
//! (wheel, scrollbar, viewport clamp) interrupt instead of fighting the user.
use std::time::{Duration, Instant};

const SPAN: Duration = Duration::from_millis(180);
const TOLERANCE: f32 = 0.75;

#[derive(Default)]
pub(crate) struct Reveal {
    active: Option<Flight>,
}

struct Flight {
    from: f32,
    to: f32,
    last_applied: f32,
    started: Instant,
}

impl Reveal {
    pub(crate) fn begin(&mut self, actual: f32, target: f32, now: Instant, instant: bool) -> bool {
        self.active = if !instant && actual.is_finite() && target.is_finite()
            && (actual-target).abs() > TOLERANCE {
            Some(Flight { from:actual,to:target,last_applied:actual,started:now })
        } else { None };
        self.active.is_some()
    }

    pub(crate) fn sample(&mut self, actual: f32, now: Instant, instant: bool) -> Option<(f32,bool)> {
        let flight = self.active.as_mut()?;
        if !actual.is_finite() || (actual-flight.last_applied).abs() > TOLERANCE {
            self.active = None;
            return None;
        }
        let t = if instant { 1. } else { (now.saturating_duration_since(flight.started).as_secs_f32()
            / SPAN.as_secs_f32()).clamp(0.,1.) };
        let next = flight.from + (flight.to-flight.from) * (1.-(1.-t).powi(3));
        flight.last_applied = next;
        let active = t < 1.;
        if !active { self.active = None; }
        Some((next,active))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_retargets_at_current_offset_and_finishes() {
        let now = Instant::now();
        let mut reveal = Reveal::default();
        assert!(reveal.begin(0.,-300.,now,false));
        let at = now+SPAN/2;
        let (middle, active) = reveal.sample(0.,at,false).unwrap();
        assert!(active && middle < 0. && middle > -300.);
        assert!(reveal.begin(middle,-100.,at,false));
        assert_eq!(reveal.sample(middle,at,false),Some((middle,true)));
        assert_eq!(reveal.sample(middle,at+SPAN,false),Some((-100.,false)));
        assert!(reveal.sample(-100.,at+SPAN,false).is_none());
    }

    #[test]
    fn manual_scroll_cancels_and_reduced_motion_settles() {
        let now = Instant::now();
        let mut reveal = Reveal::default();
        reveal.begin(0.,-200.,now,false);
        assert!(reveal.sample(-20.,now+SPAN/4,false).is_none());
        assert!(reveal.sample(-20.,now+SPAN,false).is_none());
        reveal.begin(-20.,-100.,now,false);
        assert_eq!(reveal.sample(-20.,now,true),Some((-100.,false)));
        assert!(!reveal.begin(0.,-100.,now,true));
    }
}
