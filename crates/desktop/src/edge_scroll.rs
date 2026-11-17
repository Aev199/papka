//! Scrolling a list while files are dragged near its top or bottom edge,
//! so a folder further down can be reached without letting go. The closer
//! to the edge, the faster; it stops when the pointer leaves the band, the
//! list or the drag ends.

use std::time::Duration;

use gpui::{px, Context, Pixels, Point, ScrollHandle};

use crate::Explorer;

/// Height of the band at each edge where scrolling starts.
pub(crate) const BAND: f32 = 40.;
/// Pixels per tick right at the edge; about 1100 px/s at 60 ticks.
const FASTEST: f32 = 18.;
const SLOWEST: f32 = 2.;
const TICK: Duration = Duration::from_millis(16);

/// Pixels to move per tick for a pointer at `y` over a list spanning
/// `top..bottom`: negative scrolls up, positive down, 0 away from edges.
/// Lists too short for two bands do not scroll this way.
pub(crate) fn step(y: f32, top: f32, bottom: f32) -> f32 {
    if bottom - top < BAND * 3. || y < top || y > bottom { return 0.; }
    let depth = |into: f32| {
        let closeness = (into / BAND).clamp(0., 1.);
        SLOWEST + (FASTEST - SLOWEST) * closeness * closeness
    };
    if y < top + BAND { -depth(top + BAND - y) }
    else if y > bottom - BAND { depth(y - (bottom - BAND)) }
    else { 0. }
}

/// The list under a drag and how fast it moves.
#[derive(Default)]
pub(crate) struct EdgeScroll {
    target: Option<(ScrollHandle, f32)>,
    /// Set by the hovered list on every move; the root drops an unclaimed one.
    claimed: bool,
    running: bool,
}

impl Explorer {
    /// A list's mouse move during a drag: starts, retunes or stops its scrolling.
    pub(crate) fn edge_scroll_hover(&mut self, handle: &ScrollHandle, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.dragging_files(cx) { return; }
        let bounds = handle.bounds();
        let speed = step(f32::from(position.y), f32::from(bounds.top()), f32::from(bounds.bottom()));
        if speed == 0. { return; }
        self.edge_scroll.target = Some((handle.clone(), speed));
        self.edge_scroll.claimed = true;
        if self.edge_scroll.running { return; }
        self.edge_scroll.running = true;
        let mut last = std::time::Instant::now();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(TICK).await;
            // Speed is per 16 ms: a slow frame moves further, not slower.
            let ticks = (last.elapsed().as_secs_f32() / TICK.as_secs_f32()).clamp(0.5, 4.);
            last = std::time::Instant::now();
            let more = this.update(cx, |this, cx| {
                if !this.dragging_files(cx) { this.edge_scroll.target = None; }
                let Some((handle, speed)) = this.edge_scroll.target.as_ref() else {
                    this.edge_scroll.running = false;
                    return false;
                };
                let mut offset = handle.offset();
                let lowest = -handle.max_offset().y;
                let moved = (offset.y - px(*speed * ticks)).clamp(lowest.min(px(0.)), px(0.));
                if moved != offset.y {
                    offset.y = moved;
                    handle.set_offset(offset);
                    cx.notify();
                }
                true
            }).unwrap_or(false);
            if !more { break; }
        }).detach();
    }

    /// Root mouse move, after the lists had their turn.
    pub(crate) fn edge_scroll_settle(&mut self) {
        if self.edge_scroll.claimed { self.edge_scroll.claimed = false; } else { self.edge_scroll.target = None; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_grows_toward_the_edges_and_is_zero_between() {
        assert_eq!(step(300., 0., 600.), 0.);
        assert!(step(30., 0., 600.) < 0.);
        assert!(step(570., 0., 600.) > 0.);
        // Closer to the edge is faster.
        assert!(step(2., 0., 600.).abs() > step(30., 0., 600.).abs());
        assert!((step(600., 0., 600.) - FASTEST).abs() < 0.01);
        assert!(step(599., 0., 600.) <= FASTEST);
        assert!(step(39., 0., 600.).abs() >= SLOWEST);
        // Outside the list, or a list too short for bands: nothing.
        assert_eq!(step(-5., 0., 600.), 0.);
        assert_eq!(step(610., 0., 600.), 0.);
        assert_eq!(step(10., 0., 100.), 0.);
    }
}
