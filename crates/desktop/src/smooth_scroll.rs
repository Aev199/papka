//! Smooth mouse-wheel scrolling. A wheel notch moves a list by three lines
//! at once; here the same distance is covered in about 0.15 s, easing out,
//! and quick notches add up into one glide. Touchpads report pixels and
//! already move smoothly: their events are left to GPUI, as are Shift
//! (sideways) and Ctrl (zoom) with the wheel.
//!
//! GPUI scrolls an element in the bubble phase of the wheel event. A
//! listener in the capture phase, over the same area, stops the event
//! there and moves the same scroll handle step by step instead.

use std::time::{Duration, Instant};

use gpui::{
    canvas, px, Context, DispatchPhase, HitboxBehavior, IntoElement, ScrollDelta, ScrollHandle,
    ScrollWheelEvent, SharedString, Styled,
};

use crate::Explorer;

const TICK: Duration = Duration::from_millis(8);
/// Time constant of the glide: 95 % of the way after three of them.
const TAU: f32 = 0.05;

/// The next offset on the way from `current` to `target` after `dt`
/// seconds; lands exactly once less than half a pixel is left.
pub(crate) fn glide(current: f32, target: f32, dt: f32) -> f32 {
    let left = target - current;
    if left.abs() < 0.5 { return target; }
    current + left * (1. - (-dt / TAU).exp())
}

/// Where a wheel step aims: from the running glide's target (so notches
/// add up) or from the current offset, kept within the content.
pub(crate) fn aim(from: f32, delta: f32, max: f32) -> f32 {
    (from + delta).clamp(-max.max(0.), 0.)
}

struct Glide {
    key: SharedString,
    handle: ScrollHandle,
    target: f32,
    /// The offset this glide set last: a different one means something
    /// else (keys, a reveal, a drag at the edge) moved the list.
    placed: f32,
}

#[derive(Default)]
pub(crate) struct SmoothScroll {
    glide: Option<Glide>,
    running: bool,
}

impl Explorer {
    /// An invisible layer for a scroll area: put it last inside a
    /// `relative()` container holding the area tracked by `handle`.
    pub(crate) fn wheel_layer(key: SharedString, handle: ScrollHandle, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity().downgrade();
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) { return; }
                    let ScrollDelta::Lines(lines) = event.delta else { return };
                    if lines.y == 0. || event.modifiers.control || event.modifiers.shift { return; }
                    let delta = f32::from(event.delta.pixel_delta(window.line_height()).y);
                    cx.stop_propagation();
                    let (key, handle) = (key.clone(), handle.clone());
                    let _ = view.update(cx, |this, cx| this.wheel(key, handle, delta, cx));
                });
            },
        ).absolute().top_0().left_0().size_full()
    }

    fn wheel(&mut self, key: SharedString, handle: ScrollHandle, delta: f32, cx: &mut Context<Self>) {
        let max = f32::from(handle.max_offset().y);
        let current = f32::from(handle.offset().y);
        let from = match self.smooth.glide.as_ref() {
            Some(glide) if glide.key == key && (glide.placed - current).abs() < 1. => glide.target,
            _ => current,
        };
        let target = aim(from, delta, max);
        if (target - current).abs() < 0.5 { return; }
        self.smooth.glide = Some(Glide { key, handle, target, placed: current });
        if self.smooth.running { return; }
        self.smooth.running = true;
        let mut last = Instant::now();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(TICK).await;
            let dt = last.elapsed().as_secs_f32().min(0.05);
            last = Instant::now();
            let more = this.update(cx, |this, cx| {
                let Some(run) = this.smooth.glide.as_mut() else {
                    this.smooth.running = false;
                    return false;
                };
                let mut offset = run.handle.offset();
                let now = f32::from(offset.y);
                // Moved by something else, or reached: the glide ends.
                let next = if (now - run.placed).abs() > 1. { None } else { Some(glide(now, run.target, dt)) };
                match next {
                    Some(next) => {
                        offset.y = px(next);
                        run.handle.set_offset(offset);
                        run.placed = next;
                        if next == run.target { this.smooth.glide = None; }
                        cx.notify();
                    }
                    None => this.smooth.glide = None,
                }
                true
            }).unwrap_or(false);
            if !more { break; }
        }).detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glide_eases_out_and_lands_exactly() {
        let mut y = 0.;
        let mut steps = Vec::new();
        for _ in 0..40 {
            let next = glide(y, -60., 0.008);
            steps.push(y - next);
            y = next;
        }
        assert_eq!(y, -60.);
        // Bigger steps first.
        assert!(steps[0] > steps[5] && steps[5] > steps[10]);
        // Most of the way within 0.15 s (19 ticks of 8 ms).
        let mut y = 0.;
        for _ in 0..19 { y = glide(y, -60., 0.008); }
        assert!(y < -55., "{y}");
    }

    #[test]
    fn aims_add_up_and_stay_within_the_content() {
        assert_eq!(aim(0., -60., 1000.), -60.);
        assert_eq!(aim(-60., -60., 1000.), -120.);
        assert_eq!(aim(-980., -60., 1000.), -1000.);
        assert_eq!(aim(-10., 60., 1000.), 0.);
        // Content shorter than the view: nowhere to go.
        assert_eq!(aim(0., -60., 0.), 0.);
    }
}
