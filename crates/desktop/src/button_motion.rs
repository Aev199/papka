//! Small retained hover-color transitions for button backgrounds.

use std::time::{Duration, Instant};

use gpui::{App, ElementId, IntoElement, RenderOnce, Styled, Window, canvas, px};

const HOVER_TIME: Duration = crate::motion::HOVER;

/// Paint a button's normal/hover background without taking over its input
/// handling. `None` keeps the normal state transparent (for icon buttons).
pub(crate) fn background(id: ElementId, normal: Option<u32>, hover: u32) -> impl IntoElement {
    ButtonBackground { id, normal, hover }
}

#[derive(IntoElement)]
struct ButtonBackground {
    id: ElementId,
    normal: Option<u32>,
    hover: u32,
}

#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    to: f32,
    current: f32,
    target_hovered: bool,
    started: Instant,
}

impl Tween {
    fn new() -> Self {
        Self { from: 0., to: 0., current: 0., target_hovered: false, started: Instant::now() }
    }

    fn sample(&mut self, hovered: bool, now: Instant, animate: bool) -> (f32, bool) {
        let requested = if hovered { 1. } else { 0. };
        if !animate {
            self.target_hovered = hovered;
            self.from = requested;
            self.to = requested;
            self.current = requested;
            return (self.current, false);
        }

        if self.from == self.to && self.target_hovered == hovered && self.to == requested {
            return (self.current, false);
        }

        if self.target_hovered != hovered || self.to != requested {
            self.from = self.current;
            self.to = requested;
            self.target_hovered = hovered;
            self.started = now;
        }

        let t = (now.duration_since(self.started).as_secs_f32() / HOVER_TIME.as_secs_f32()).clamp(0., 1.);
        let eased = crate::motion::smoothstep(t);
        self.current = self.from + (self.to - self.from) * eased;
        (self.current, t < 1.)
    }
}

impl RenderOnce for ButtonBackground {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id, cx, |_, _| Tween::new());
        let (normal, hover) = (self.normal, self.hover);
        canvas(
            move |bounds, window, cx| {
                let hovered = bounds.contains(&window.mouse_position());
                let now = Instant::now();
                let (amount, moving) = state.update(cx, |tween, _| {
                    tween.sample(hovered, now, crate::theme::animations())
                });
                if moving { window.request_animation_frame(); }
                (normal, hover, amount)
            },
            move |bounds, (normal, hover, amount), window, _| {
                let color = normal.map(|normal| crate::palette::mix(normal, hover, amount)).unwrap_or(hover);
                if normal.is_some() {
                    window.paint_quad(gpui::fill(bounds, gpui::rgb(color)).corner_radii(px(4.)));
                } else {
                    let alpha = (amount * 255.).round() as u32;
                    if alpha > 0 {
                        window.paint_quad(gpui::fill(bounds, gpui::rgba((color << 8) | alpha)).corner_radii(px(4.)));
                    }
                }
            },
        ).size_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tween_starts_from_current_color_and_lands_exactly() {
        let start = Instant::now();
        let mut tween = Tween::new();
        assert_eq!(tween.sample(true, start, true), (0., true));
        let half_time = start + HOVER_TIME / 2;
        let (middle, moving) = tween.sample(true, half_time, true);
        assert!(moving);
        assert!((middle - 0.5).abs() < f32::EPSILON);

        let reverse = half_time + Duration::from_millis(10);
        let (before_reverse, _) = tween.sample(true, reverse, true);
        let (reversed, moving) = tween.sample(false, reverse, true);
        assert_eq!(reversed, before_reverse);
        assert!(moving);
        let (landed, moving) = tween.sample(false, reverse + HOVER_TIME, true);
        assert_eq!(landed, 0.);
        assert!(!moving);
    }

    #[test]
    fn disabled_animation_uses_the_target_color_immediately() {
        let mut tween = Tween::new();
        let (amount, moving) = tween.sample(true, Instant::now(), false);
        assert_eq!(amount, 1.);
        assert!(!moving);
    }
}
