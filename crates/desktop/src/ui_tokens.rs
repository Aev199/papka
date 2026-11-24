//! Semantic dimensions for the shared desktop controls, in logical pixels.
//! Stage 1 preserves Beta 105 geometry. Keep row density, list column widths,
//! and thumbnail sizing in their own layout policies.
pub(crate) const SIDEBAR_WIDTH: f32 = 220.;
pub(crate) const CONTROL_RADIUS: f32 = 4.;
pub(crate) const PANEL_RADIUS: f32 = 6.;
pub(crate) const ICON_SIZE: f32 = 16.;
pub(crate) const SMALL_ICON_SIZE: f32 = 14.;
pub(crate) const TOOLBAR_BUTTON_SIZE: f32 = 30.;
pub(crate) const STATUS_BUTTON_SIZE: f32 = 24.;
pub(crate) const DIALOG_BUTTON_HEIGHT: f32 = 28.;
pub(crate) const FOCUS_MARK_WIDTH: f32 = 3.;
pub(crate) const FOCUS_MARK_HEIGHT: f32 = 16.;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_controls_keep_beta_105_geometry() {
        assert_eq!([SIDEBAR_WIDTH, CONTROL_RADIUS, PANEL_RADIUS], [220., 4., 6.]);
        assert_eq!([ICON_SIZE, SMALL_ICON_SIZE, TOOLBAR_BUTTON_SIZE, STATUS_BUTTON_SIZE,
            DIALOG_BUTTON_HEIGHT, FOCUS_MARK_WIDTH, FOCUS_MARK_HEIGHT],
            [16., 14., 30., 24., 28., 3., 16.]);
    }

    #[test]
    fn icon_controls_have_room_for_their_content() {
        assert!(TOOLBAR_BUTTON_SIZE - ICON_SIZE >= 12.);
        assert!(STATUS_BUTTON_SIZE - SMALL_ICON_SIZE >= 8.);
        assert!(DIALOG_BUTTON_HEIGHT >= ICON_SIZE + 8.);
        assert!(PANEL_RADIUS >= CONTROL_RADIUS);
    }
}
