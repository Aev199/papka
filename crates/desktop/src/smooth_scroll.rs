//! Smooth mouse-wheel scrolling. A wheel notch moves a list by three lines
//! at once; here the same distance is covered in about 0.15 s, easing out,
//! and quick notches add up into one glide. Touchpads report pixels and
//! already move smoothly: their events are left to GPUI, as are Shift
//! (sideways) and Ctrl (zoom) with the wheel.
//!
//! GPUI scrolls an element in the bubble phase of the wheel event. An
//! invisible layer over the list listens in the capture phase, stops the
//! event and records where the list should go. The glide moves one step per
//! drawn frame, in the layer's prepaint: the layer comes before the list,
//! so the list lays out its rows at the new offset in the same frame. No
//! timers: steps follow the frames, whatever their pace.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    canvas, px, DispatchPhase, HitboxBehavior, IntoElement, ScrollDelta, ScrollHandle,
    ScrollWheelEvent, SharedString, Styled,
};

/// Time constant of the glide: 95 % of the way after three of them.
const TAU: f32 = 0.05;
/// The step a frame takes when the time since the last one is unknown.
const FIRST_FRAME: f32 = 1. / 60.;

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
    target: f32,
    /// The offset this glide set last: a different one means something
    /// else (keys, a reveal, a drag at the edge) moved the list.
    placed: f32,
    last: Option<Instant>,
}

/// Running glides by scroll area, shared by the window's layers.
#[derive(Clone, Default)]
pub(crate) struct SmoothScroll(Rc<RefCell<HashMap<SharedString, Glide>>>);

impl SmoothScroll {
    /// Moves the area one frame further; false once its glide is over.
    fn advance(&self, key: &SharedString, handle: &ScrollHandle) -> bool {
        let mut glides = self.0.borrow_mut();
        let Some(run) = glides.get_mut(key) else { return false };
        let mut offset = handle.offset();
        let now = f32::from(offset.y);
        if (now - run.placed).abs() > 1. {
            glides.remove(key);
            return false;
        }
        let time = Instant::now();
        let dt = run.last.map_or(FIRST_FRAME, |last| (time - last).as_secs_f32().min(0.05));
        run.last = Some(time);
        let next = glide(now, run.target, dt);
        offset.y = px(next);
        handle.set_offset(offset);
        run.placed = next;
        if next == run.target {
            glides.remove(key);
            return false;
        }
        true
    }

    /// A wheel notch over the area: the glide's target moves by `delta`.
    fn wheel(&self, key: SharedString, handle: &ScrollHandle, delta: f32) -> bool {
        let max = f32::from(handle.max_offset().y);
        let current = f32::from(handle.offset().y);
        let mut glides = self.0.borrow_mut();
        let from = match glides.get(&key) {
            Some(run) if (run.placed - current).abs() < 1. => run.target,
            _ => current,
        };
        let target = aim(from, delta, max);
        if (target - current).abs() < 0.5 { return false; }
        let last = glides.get(&key).and_then(|run| run.last);
        glides.insert(key, Glide { target, placed: current, last });
        true
    }
}

/// An invisible layer for a scroll area: put it first inside a
/// `relative()` container holding the area tracked by `handle`.
pub(crate) fn layer(state: SmoothScroll, key: SharedString, handle: ScrollHandle) -> impl IntoElement {
    let stepper = state.clone();
    let (step_key, step_handle) = (key.clone(), handle.clone());
    canvas(
        move |bounds, window, _| {
            if stepper.advance(&step_key, &step_handle) { window.request_animation_frame(); }
            window.insert_hitbox(bounds, HitboxBehavior::Normal)
        },
        move |_, hitbox, window, _| {
            let view = window.current_view();
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) { return; }
                let ScrollDelta::Lines(lines) = event.delta else { return };
                if lines.y == 0. || event.modifiers.control || event.modifiers.shift { return; }
                let delta = f32::from(event.delta.pixel_delta(window.line_height()).y);
                cx.stop_propagation();
                if state.wheel(key.clone(), &handle, delta) { cx.notify(view); }
            });
        },
    ).absolute().top_0().left_0().size_full()
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
        // Most of the way within 0.15 s, whatever the frame rate.
        for frame in [1. / 144., 1. / 60., 1. / 30.] {
            let mut y = 0.;
            for _ in 0..(0.15 / frame) as usize { y = glide(y, -60., frame); }
            assert!(y < -54., "{frame} {y}");
        }
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

    #[test]
    fn a_list_without_room_does_not_glide() {
        let state = SmoothScroll::default();
        let handle = ScrollHandle::new();
        let key: SharedString = "list".into();
        // No content yet (max offset 0): nothing to do.
        assert!(!state.wheel(key.clone(), &handle, -60.));
        assert!(!state.advance(&key, &handle));
    }
}
