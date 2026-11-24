//! Shared timing and interpolation, independent of GPUI and file state.
use std::time::{Duration, Instant};

// Shared motion policy. Preserve the current visual timing while consolidating curves.
pub(crate) const ITEM_REFLOW: Duration = Duration::from_millis(180);
pub(crate) const MODE_FADE_OUT: Duration = Duration::from_millis(70);
pub(crate) const MODE_FADE_IN: Duration = Duration::from_millis(110);
pub(crate) const TRANSIENT_MS: u64 = 120;
pub(crate) const DISCLOSURE_MS: u64 = 160;
pub(crate) const HOVER: Duration = Duration::from_millis(100);

pub(crate) fn ease_out_fraction(t: f32) -> f32 { 1. - (1. - t).powi(3) }
pub(crate) fn smoothstep(t: f32) -> f32 { t * t * (3. - 2. * t) }

/// Eased progress of an opening (0 → 1) `elapsed` into `span`.
pub(crate) fn ease_out(elapsed: Duration, span: Duration) -> f32 {
    let t = (elapsed.as_secs_f32() / span.as_secs_f32()).clamp(0., 1.);
    ease_out_fraction(t)
}

pub(crate) const PANE_OPEN_MS: u64 = 220;
pub(crate) const PANE_CLOSE_MS: u64 = 140;
pub(crate) const PREVIEW_FADE: Duration = Duration::from_millis(150);

#[derive(Default)]
pub(crate) struct Tween { from:f32, target:f32, started:Option<Instant> }
impl Tween {
    pub fn sample(&mut self,target:f32,instant:bool,now:Instant,ms:u64)->(f32,bool) {
        let value=match self.started {
            Some(at)=>{let t=(now.saturating_duration_since(at).as_secs_f32()/(ms as f32/1000.)).clamp(0.,1.);self.from+(self.target-self.from)*ease_out_fraction(t)},
            None=>self.target,
        };
        if instant {self.from=target;self.target=target;self.started=None;return(target,false);}
        if (target-self.target).abs()>0.01 {self.from=value;self.target=target;self.started=Some(now);}
        let active=self.started.is_some_and(|at|now.saturating_duration_since(at)<Duration::from_millis(ms));
        if !active {self.started=None;return(self.target,false);}
        (value,true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_have_exact_endpoints_and_are_monotone() {
        for curve in [ease_out_fraction as fn(f32) -> f32, smoothstep] {
            assert_eq!(curve(0.), 0.);
            assert_eq!(curve(1.), 1.);
            let mut previous = 0.;
            for i in 1..=100 {
                let value = curve(i as f32 / 100.);
                assert!(value >= previous && value <= 1.);
                previous = value;
            }
        }
    }

    #[test]
    fn a_retarget_continues_from_the_current_sample() {
        let start = Instant::now();
        let mut tween = Tween::default();
        assert_eq!(tween.sample(1., false, start, TRANSIENT_MS), (0., true));
        let mid = start + Duration::from_millis(60);
        let before = tween.sample(1., false, mid, TRANSIENT_MS).0;
        assert_eq!(tween.sample(0., false, mid, TRANSIENT_MS), (before, true));
        let end = mid + Duration::from_millis(TRANSIENT_MS);
        assert_eq!(tween.sample(0., false, end, TRANSIENT_MS), (0., false));
    }

    #[test]
    fn unchanged_target_does_not_restart_and_reduced_motion_settles() {
        let start = Instant::now();
        let mut tween = Tween::default();
        tween.sample(1., false, start, TRANSIENT_MS);
        let mid = start + Duration::from_millis(60);
        assert!(tween.sample(1., false, mid, TRANSIENT_MS).1);
        let end = start + Duration::from_millis(TRANSIENT_MS);
        assert_eq!(tween.sample(1., false, end, TRANSIENT_MS), (1., false));
        assert_eq!(tween.sample(0., true, end, TRANSIENT_MS), (0., false));
        assert_eq!(tween.sample(0., false, end, TRANSIENT_MS), (0., false));
    }
}
