//! Pure geometry used by the native renderer.
/// Miller columns share one width (Atlas/Finder style): navigation adds or
/// removes a column but never re-flows the others.
pub const COLUMN_DEFAULT: f32 = 220.;
pub const COLUMN_MIN: f32 = 150.;
pub const COLUMN_MAX: f32 = 600.;
pub const ROW_HEIGHT: f32 = 24.;
pub fn row_index(local_y: f32, scroll: f32) -> usize {
    ((local_y + scroll).max(0.) / ROW_HEIGHT).floor() as usize
}
pub fn effective_pane_width(estimate: f32, measured: Option<f32>) -> f32 {
    measured
        .filter(|width| width.is_finite() && *width > 0.)
        .unwrap_or(estimate)
}
pub fn column_identity(width: f32, count: usize, current: usize) -> String {
    format!("{count}-{}-{current}", width as u32 / 40)
}
/// How many columns fit at their minimum width; a preview needs two.
pub fn column_capacity(width: f32) -> usize {
    ((width.max(0.) / COLUMN_MIN) as usize).max(1)
}
/// Every column gets `base` until they no longer fit; then all shrink
/// evenly, down to `COLUMN_MIN`, after which the strip scrolls.
pub fn column_widths(width: f32, count: usize, base: f32) -> Vec<f32> {
    let base = base.clamp(COLUMN_MIN, COLUMN_MAX);
    let each = if count as f32 * base <= width { base } else { (width / count as f32).floor().clamp(COLUMN_MIN, base) };
    vec![each; count]
}
/// Keep the complete trail; viewport width must not delete navigation context.
pub fn column_paths(
    path: &std::path::Path,
    preview: Option<&std::path::Path>,
) -> Vec<std::path::PathBuf> {
    let mut columns: Vec<_> = path.ancestors().map(std::path::Path::to_path_buf).collect();
    columns.reverse();
    columns.extend(preview.map(std::path::Path::to_path_buf));
    columns
}
/// Negative GPUI scroll offset, revealing the current column with minimal movement.
/// Positions are content coordinates obtained from actual post-layout bounds.
pub fn reveal_offset(viewport: f32, start: f32, end: f32, offset: f32) -> f32 {
    if end - start > viewport || start + offset < 0. {
        -start
    } else if end + offset > viewport {
        viewport - end
    } else {
        offset
    }
}
pub fn reveal_column_offset(
    widths: &[f32],
    current: usize,
    viewport: f32,
    start: f32,
    actual_width: f32,
    offset: f32,
) -> f32 {
    let preview_width = widths.get(current + 1).copied().filter(|preview|actual_width + preview <= viewport).unwrap_or(0.);
    reveal_offset(
        viewport,
        start,
        start + actual_width + preview_width,
        offset,
    )
}
/// Dragging any column edge changes the shared width.
pub fn resize_columns(start: f32, delta: f32) -> f32 {
    if delta.is_finite() { (start + delta).clamp(COLUMN_MIN, COLUMN_MAX) } else { start }
}
/// Details view, as in Explorer: name, modified, type and size each keep
/// their own width; a wider table scrolls sideways instead of squeezing.
pub const LIST_DEFAULT: [f32; 4] = [260., 140., 110., 90.];
pub const LIST_MIN: f32 = 40.;
pub const LIST_MAX: f32 = 1600.;
/// Half the grab zone on each side of a header divider.
pub const LIST_GRIP: f32 = 4.;
/// Dragging a divider sizes only the column on its left; the ones to the
/// right move along with it.
pub fn resize_list_column(start: f32, delta: f32) -> f32 {
    if delta.is_finite() { (start + delta).clamp(LIST_MIN, LIST_MAX) } else { start }
}
/// Where each divider sits: the right edge of every column.
pub fn list_dividers(widths: &[f32; 4]) -> [f32; 4] {
    let mut edge = 0.;
    widths.map(|width| { edge += width; edge })
}
/// Saved widths; the older three-value form (no name column) gets the
/// default name width.
pub fn parse_list_widths(text: &str) -> Option<[f32; 4]> {
    let values: Vec<f32> = text.split(',').map(|v| v.trim().parse().ok()).collect::<Option<_>>()?;
    let valid = values.iter().all(|v| (LIST_MIN..=LIST_MAX).contains(v));
    match values.as_slice() {
        &[a, b, c, d] if valid => Some([a, b, c, d]),
        &[b, c, d] if valid => Some([LIST_DEFAULT[0], b, c, d]),
        _ => None,
    }
}
/// Double-click on a divider: the widest content plus the cell's padding.
pub fn fit_list_column(content: f32, padding: f32) -> f32 {
    if content.is_finite() { (content + padding).ceil().clamp(LIST_MIN, LIST_MAX) } else { LIST_DEFAULT[0] }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_left_and_right_widths_drive_capacity_independently() {
        assert_eq!(effective_pane_width(750., Some(410.)), 410.);
        assert_eq!(effective_pane_width(750., None), 750.);
        assert_eq!(column_capacity(effective_pane_width(750., Some(410.))), 2);
        assert_eq!(column_capacity(effective_pane_width(750., Some(670.))), 4);
    }
    #[test]
    fn preview_role_changes_reset_cached_column_sizes() {
        assert_ne!(column_identity(750., 3, 2), column_identity(750., 3, 1));
        assert_eq!(column_identity(750., 3, 1), column_identity(750., 3, 1));
    }
    #[test]
    fn narrow_pane_does_not_force_two_columns() {
        assert_eq!(column_capacity(280.), 1);
        assert_eq!(column_capacity(0.), 1);
    }
    #[test]
    fn columns_keep_one_width_while_they_fit() {
        assert_eq!(column_widths(1000., 3, 220.), vec![220.; 3]);
        assert_eq!(column_widths(1000., 4, 220.), vec![220.; 4]);
        assert!(column_widths(430., 0, 220.).is_empty());
    }
    #[test]
    fn full_window_shrinks_columns_evenly() {
        assert_eq!(column_widths(1000., 5, 220.), vec![200.; 5]);
        assert_eq!(column_widths(1000., 5, 220.).iter().sum::<f32>(), 1000.);
    }
    #[test]
    fn deep_chain_overflows_instead_of_crushing_columns() {
        let widths = column_widths(600., 7, 220.);
        assert_eq!(widths, vec![COLUMN_MIN; 7]);
        assert!(widths.iter().sum::<f32>() > 600.);
    }
    #[test]
    fn list_columns_resize_one_at_a_time() {
        assert_eq!(resize_list_column(140., 30.), 170.);
        assert_eq!(resize_list_column(140., -500.), LIST_MIN);
        assert_eq!(resize_list_column(140., 5000.), LIST_MAX);
        assert_eq!(resize_list_column(140., f32::NAN), 140.);
        assert_eq!(list_dividers(&[200., 100., 50., 80.]), [200., 300., 350., 430.]);
    }
    #[test]
    fn saved_list_widths_include_the_name_column() {
        assert_eq!(parse_list_widths("300,140,110,90"), Some([300., 140., 110., 90.]));
        assert_eq!(parse_list_widths("150, 100, 80"), Some([LIST_DEFAULT[0], 150., 100., 80.]));
        assert_eq!(parse_list_widths("300,140,x,90"), None);
        assert_eq!(parse_list_widths("300,140,10,90"), None);
        assert_eq!(parse_list_widths("1,2"), None);
    }
    #[test]
    fn fitting_a_column_keeps_it_within_limits() {
        assert_eq!(fit_list_column(100.2, 16.), 117.);
        assert_eq!(fit_list_column(0., 16.), LIST_MIN);
        assert_eq!(fit_list_column(1e6, 16.), LIST_MAX);
    }
    #[test]
    fn resizing_changes_the_shared_width_within_limits() {
        assert_eq!(resize_columns(220., 40.), 260.);
        assert_eq!(resize_columns(220., -500.), COLUMN_MIN);
        assert_eq!(resize_columns(220., 5000.), COLUMN_MAX);
        assert_eq!(resize_columns(220., f32::NAN), 220.);
    }
    #[test]
    fn every_ancestor_remains_reachable_in_deep_chain() {
        let path = std::path::Path::new("/one/two/three/four/five/six");
        let preview = path.join("seven");
        let columns = column_paths(path, Some(&preview));
        assert_eq!(columns.len(), 8);
        assert_eq!(columns[0], std::path::PathBuf::from("/"));
        assert_eq!(columns[6], path);
        assert_eq!(columns[7], preview);
    }
    #[test]
    fn deep_navigation_reveals_current_column() {
        assert_eq!(reveal_offset(600., 720., 920., 0.), -320.);
    }
    #[test]
    fn folder_preview_is_visible_beside_current_when_it_fits() {
        assert_eq!(
            reveal_column_offset(&[144., 200., 160.], 1, 600., 720., 200., 0.),
            -480.
        );
    }
    #[test]
    fn oversized_preview_keeps_current_visible() {
        assert_eq!(reveal_column_offset(&[144.,200.,400.],1,440.,144.,200.,0.),0.);
    }
    #[test]
    fn navigating_up_reveals_column_to_the_left() {
        assert_eq!(reveal_offset(600., 144., 344., -720.), -144.);
    }
    #[test]
    fn already_visible_keeps_manual_scroll() {
        assert_eq!(reveal_offset(600., 200., 400., -100.), -100.);
    }
    #[test]
    fn oversized_column_reveals_its_start() {
        assert_eq!(reveal_offset(100., 144., 344., 0.), -144.);
    }
    #[test]
    fn compact_row_hit_testing_includes_scroll() {
        assert_eq!(row_index(49., 0.), 2);
        assert_eq!(row_index(1., 48.), 2);
        assert_eq!(row_index(23.9, 0.), 0);
        assert_eq!(row_index(24., 0.), 1);
    }
}

#[cfg(test)]
mod dock_tests {
    use super::*;
    #[test]
    fn dock_regions_follow_center_edges_and_nearest_corner() {
        assert_eq!(dock_zone(50.,50.,100.,100.),DockZone::Center);
        assert_eq!(dock_zone(5.,50.,100.,100.),DockZone::Left);
        assert_eq!(dock_zone(95.,50.,100.,100.),DockZone::Right);
        assert_eq!(dock_zone(50.,5.,100.,100.),DockZone::Top);
        assert_eq!(dock_zone(50.,95.,100.,100.),DockZone::Bottom);
        assert_eq!(dock_zone(20.,5.,100.,100.),DockZone::Top);
        assert_eq!(dock_zone(95.,80.,100.,100.),DockZone::Right);
        assert_eq!(dock_zone(1.,1.,0.,100.),DockZone::Center);
        assert_eq!(dock_zone(1.,1.,100.,0.),DockZone::Center);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockZone { Left, Right, Top, Bottom, Center }
pub fn dock_zone(x: f32, y: f32, w: f32, h: f32) -> DockZone {
    if ![x,y,w,h].iter().all(|n|n.is_finite()) || w<=0. || h<=0. {return DockZone::Center;}
    let x=x.clamp(0.,w); let y=y.clamp(0.,h);
    [(DockZone::Left,x,x<w*0.3),(DockZone::Right,w-x,x>w*0.7),
     (DockZone::Top,y,y<h*0.3),(DockZone::Bottom,h-y,y>h*0.7)]
        .into_iter().filter(|(_,_,hit)|*hit).min_by(|a,b|a.1.total_cmp(&b.1))
        .map(|(zone,_,_)|zone).unwrap_or(DockZone::Center)
}

#[cfg(test)]
mod dock_hit_tests {
    use super::*;
    #[test]
    fn a_drag_source_outside_its_bounds_cannot_override_the_target() {
        assert_eq!(dock_hit(-510.,200.,550.,380.),None);
        assert_eq!(dock_hit(42.,200.,550.,700.),Some(DockZone::Left));
        assert_eq!(dock_hit(551.,20.,550.,380.),None);
    }
}

pub fn dock_hit(x: f32, y: f32, w: f32, h: f32) -> Option<DockZone> {
    if ![x,y,w,h].iter().all(|n|n.is_finite()) || w<=0. || h<=0. || x<0. || y<0. || x>w || y>h {return None;}
    Some(dock_zone(x,y,w,h))
}
