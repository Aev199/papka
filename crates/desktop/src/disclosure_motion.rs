//! Retained height and chevron motion for collapsible sections.
use std::time::Instant;

use gpui::{
    prelude::*, App, AnyElement, ElementId, IntoElement, RenderOnce, Styled, Svg, Window, div, px,
};

const DURATION_MS: u64 = crate::motion::DISCLOSURE_MS;

/// Clip content while its accordion height moves. Content stays rendered through closing.
pub(crate) fn reveal(
    id: ElementId,
    open: bool,
    height: f32,
    content: AnyElement,
) -> impl IntoElement {
    Reveal {
        id,
        open,
        height: height.max(0.),
        content,
    }
}

/// Rotate a chevron through a quarter turn as the disclosure opens.
pub(crate) fn chevron(id: ElementId, open: bool, content: Svg) -> impl IntoElement {
    Chevron { id, open, content }
}

#[derive(IntoElement)]
struct Reveal {
    id: ElementId,
    open: bool,
    height: f32,
    content: AnyElement,
}

#[derive(IntoElement)]
struct Chevron {
    id: ElementId,
    open: bool,
    content: Svg,
}

#[derive(Default)]
struct Motion {
    initialized: bool,
    spring: crate::motion_policy::Spring,
}

impl Motion {
    fn sample(&mut self, open: bool, now: Instant, animate: bool) -> (f32, bool) {
        let target = if open { 1. } else { 0. };
        if !self.initialized {
            self.initialized = true;
            return self.spring.sample(target, true, now, DURATION_MS);
        }
        let (amount,active) = self.spring.sample(target, !animate, now, DURATION_MS);
        (amount.clamp(0.,1.),active)
    }
}

impl RenderOnce for Reveal {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| Motion::default());
        let now = Instant::now();
        let (amount, active) = state.update(cx, |motion, _| {
            motion.sample(self.open, now, crate::theme::animations())
        });
        if active {
            window.request_animation_frame();
        }
        let visible_height = self.height * amount;
        let fully_closed = amount <= 0. && !self.open;
        div()
            .id(self.id)
            .relative()
            .h(px(visible_height))
            .w_full()
            .flex_none()
            .min_h_0()
            .overflow_hidden()
            .when(!fully_closed, |element| element.child(div().h(px(self.height)).w_full().flex_none().child(self.content)))
            // Closing rows are visible only for continuity. No invisible
            // actionable entries may remain after the group has settled.
            .when(!self.open && !fully_closed, |element| element.child(crate::popup_motion::shield()))
    }
}

impl RenderOnce for Chevron {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| Motion::default());
        let now = Instant::now();
        let (amount, active) = state.update(cx, |motion, _| {
            motion.sample(self.open, now, crate::theme::animations())
        });
        if active {
            window.request_animation_frame();
        }
        self.content.with_transformation(gpui::Transformation::rotate(gpui::radians(
            amount * std::f32::consts::FRAC_PI_2,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn first_sample_settles_to_requested_state_without_startup_motion() {
        let now = Instant::now();
        let mut motion = Motion::default();
        assert_eq!(motion.sample(true, now, true), (1., false));
        assert_eq!(motion.sample(true, now + Duration::from_millis(40), true), (1., false));
    }

    #[test]
    fn later_toggle_animates_and_reversal_starts_at_current_value() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.sample(false, now, true);
        assert_eq!(motion.sample(true, now, true), (0., true));
        let midway = now + Duration::from_millis(80);
        let (before, active) = motion.sample(true, midway, true);
        assert!(active && before > 0. && before < 1.);
        let (reversed, active) = motion.sample(false, midway, true);
        assert_eq!(reversed, before);
        assert!(active);
    }

    #[test]
    fn disabling_animation_finishes_immediately() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.sample(false, now, true);
        motion.sample(true, now, true);
        assert_eq!(motion.sample(false, now + Duration::from_millis(25), false), (0., false));
    }
}
