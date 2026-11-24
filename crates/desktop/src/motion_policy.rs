//! Shared timing and interpolation, independent of GPUI and file state.
use std::time::{Duration, Instant};

// Shared motion policy. Preserve the current visual timing while consolidating curves.
pub(crate) const ITEM_REFLOW: Duration = Duration::from_millis(180);
pub(crate) const MODE_FADE_OUT: Duration = Duration::from_millis(70);
pub(crate) const MODE_FADE_IN: Duration = Duration::from_millis(110);
pub(crate) const TRANSIENT_MS: u64 = 120;
pub(crate) const DISCLOSURE_MS: u64 = 160;
pub(crate) const HOVER: Duration = Duration::from_millis(100);

/// Optional, process-local diagnostic slowdown for migrated visual clocks.
/// The factor is fixed at launch, so retargets never change time scale midflight.
pub(crate) fn diagnostic_span(span: Duration) -> Duration {
    #[cfg(test)]
    { span }
    #[cfg(not(test))]
    {
        static SCALE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        let factor = *SCALE.get_or_init(|| std::env::var("PAPKA_MOTION_SLOW").ok()
            .and_then(|s| s.parse::<u32>().ok()).filter(|n| [1, 2, 4].contains(n)).unwrap_or(1));
        span * factor
    }
}

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
        if ms == 0 { self.from=target;self.target=target;self.started=None;return(target,false); }
        let span=diagnostic_span(Duration::from_millis(ms));
        let value=match self.started {
            Some(at)=>{let t=(now.saturating_duration_since(at).as_secs_f32()/span.as_secs_f32()).clamp(0.,1.);self.from+(self.target-self.from)*ease_out_fraction(t)},
            None=>self.target,
        };
        if instant {self.from=target;self.target=target;self.started=None;return(target,false);}
        if (target-self.target).abs()>0.01 {self.from=value;self.target=target;self.started=Some(now);}
        let active=self.started.is_some_and(|at|now.saturating_duration_since(at)<span);
        if !active {self.started=None;return(self.target,false);}
        (value,true)
    }
}

/// Critically damped scalar motion. Unlike a restartable easing, a
/// retarget preserves velocity as well as position. The analytic solution
/// is independent of render frequency; a bounded tail stops frame requests.
#[derive(Default)]
pub(crate) struct Spring {
    from: f32,
    velocity: f32,
    target: f32,
    started: Option<Instant>,
}

impl Spring {
    fn pose(&self, now: Instant, span: Duration) -> (f32, f32) {
        let Some(start) = self.started else { return (self.target, 0.); };
        let elapsed = now.saturating_duration_since(start);
        if elapsed.is_zero() { return (self.from, self.velocity); }
        if elapsed >= span * 2 { return (self.target, 0.); }
        let t = elapsed.as_secs_f32();
        let omega = 8. / span.as_secs_f32();
        let distance = self.from - self.target;
        let c = self.velocity + omega * distance;
        let decay = (-omega * t).exp();
        (self.target + (distance + c * t) * decay, (self.velocity - omega * c * t) * decay)
    }

    pub(crate) fn sample(&mut self, target: f32, instant: bool, now: Instant, ms: u64) -> (f32, bool) {
        if instant || ms == 0 {
            self.from = target; self.target = target; self.velocity = 0.; self.started = None;
            return (target, false);
        }
        let span = diagnostic_span(Duration::from_millis(ms));
        let (position, velocity) = self.pose(now, span);
        if (target - self.target).abs() > 0.01 {
            self.from = position; self.velocity = velocity; self.target = target; self.started = Some(now);
        }
        if (position - self.target).abs() <= 0.02 && velocity.abs() <= 0.5 {
            self.from = self.target; self.velocity = 0.; self.started = None;
            (self.target, false)
        } else {
            (position, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_retargets_with_continuous_position_and_velocity() {
        let now = Instant::now();
        let span = Duration::from_millis(TRANSIENT_MS);
        let mut spring = Spring::default();
        assert_eq!(spring.sample(100., false, now, TRANSIENT_MS), (0., true));
        let mid = now + span / 4;
        let before = spring.pose(mid, span);
        assert!(before.0 > 0. && before.0 < 100.);
        assert!(before.1 > 0.);
        assert_eq!(spring.sample(-20., false, mid, TRANSIENT_MS), (before.0, true));
        let after = spring.pose(mid, span);
        assert!((before.0 - after.0).abs() < 0.001);
        assert!((before.1 - after.1).abs() < 0.001);
        assert_eq!(spring.sample(-20., false, mid + span * 2, TRANSIENT_MS), (-20., false));
        assert_eq!(spring.sample(0., true, mid, TRANSIENT_MS), (0., false));
    }

    #[test]
    fn spring_has_the_same_pose_regardless_of_intermediate_frame_count() {
        let now = Instant::now();
        let mut a = Spring::default();
        let mut b = Spring::default();
        a.sample(50., false, now, TRANSIENT_MS);
        b.sample(50., false, now, TRANSIENT_MS);
        for i in 1..5 { a.sample(50., false, now + Duration::from_millis(i * 10), TRANSIENT_MS); }
        let end = now + Duration::from_millis(50);
        assert_eq!(a.sample(50., false, end, TRANSIENT_MS), b.sample(50., false, end, TRANSIENT_MS));
    }


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
