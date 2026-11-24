//! Opt-in render cadence and CPU UI-construction diagnostics. Not GPU timing.
//! No timer, no file reads, no forced refresh; idle gaps are excluded.
use std::{collections::VecDeque, time::{Duration, Instant}};

const LIMIT: usize = 120;
const IDLE_GAP: Duration = Duration::from_millis(250);

pub(crate) struct Lab {
    enabled: bool,
    previous: Option<Instant>,
    intervals: VecDeque<f32>,
    build: VecDeque<f32>,
}

impl Default for Lab {
    fn default() -> Self {
        Self::new(std::env::var("PAPKA_MOTION_LAB").is_ok_and(|s| s == "1"))
    }
}

impl Lab {
    fn new(enabled: bool) -> Self {
        Self { enabled, previous: None, intervals: VecDeque::new(), build: VecDeque::new() }
    }

    pub(crate) fn begin(&mut self) -> Option<Instant> {
        if !self.enabled { return None; }
        let now = Instant::now();
        self.observe(now);
        Some(now)
    }

    fn observe(&mut self, now: Instant) {
        if let Some(previous) = self.previous.replace(now) {
            let interval = now.saturating_duration_since(previous);
            if !interval.is_zero() && interval <= IDLE_GAP {
                Self::push(&mut self.intervals, interval);
            }
        }
    }

    fn push(samples: &mut VecDeque<f32>, duration: Duration) {
        if samples.len() == LIMIT { samples.pop_front(); }
        samples.push_back(duration.as_secs_f32() * 1000.);
    }

    pub(crate) fn end(&mut self, start: Option<Instant>) {
        if let Some(start) = start { Self::push(&mut self.build, start.elapsed()); }
    }

    fn percentile(samples: &VecDeque<f32>, p: f32) -> Option<f32> {
        if samples.is_empty() { return None; }
        let mut sorted: Vec<_> = samples.iter().copied().collect();
        sorted.sort_by(f32::total_cmp);
        let index = ((sorted.len() as f32 * p).ceil() as usize).saturating_sub(1).min(sorted.len()-1);
        Some(sorted[index])
    }

    pub(crate) fn summary(&self) -> Option<String> {
        if !self.enabled { return None; }
        let fmt = |value: Option<f32>| value.map_or("—".into(), |v| format!("{v:.1}"));
        Some(format!("Lab p95 {} / UI {} мс (n={})", fmt(Self::percentile(&self.intervals,0.95)),
            fmt(Self::percentile(&self.build,0.95)), self.intervals.len()))
    }

    pub(crate) fn reset(&mut self) {
        self.previous = None;
        self.intervals.clear();
        self.build.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_diagnostics_do_not_record_or_display() {
        let mut lab = Lab::new(false);
        let start = lab.begin();
        lab.end(start);
        assert!(lab.summary().is_none());
        assert!(lab.intervals.is_empty() && lab.build.is_empty());
    }

    #[test]
    fn idle_gaps_are_excluded_and_window_is_bounded() {
        let mut lab = Lab::new(true);
        let now = Instant::now();
        for i in 0..200 {
            lab.observe(now + Duration::from_millis(i * 16));
        }
        assert_eq!(lab.intervals.len(), LIMIT);
        assert_eq!(Lab::percentile(&lab.intervals,0.95),Some(16.));
        lab.observe(now + Duration::from_secs(5));
        assert_eq!(Lab::percentile(&lab.intervals,0.95),Some(16.));
        lab.reset();
        assert!(lab.intervals.is_empty() && lab.previous.is_none());
    }

    #[test]
    fn percentile_reports_slow_tail() {
        let samples: VecDeque<_> = [1.,2.,3.,4.,50.].into();
        assert_eq!(Lab::percentile(&samples,0.95),Some(50.));
        assert_eq!(Lab::percentile(&samples,0.5),Some(3.));
    }
}
