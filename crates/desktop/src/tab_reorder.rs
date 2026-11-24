//! Retained tab-strip geometry while a tab is dragged toward a new slot.
use std::{collections::HashMap, time::Instant};

use filemanager_core::panes::GroupId;

use crate::motion_policy::{Spring, Tween};

const TAB_GAP: f32 = 4.;
const DURATION_MS: u64 = crate::motion_policy::TRANSIENT_MS;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Target {
    pub from: GroupId,
    pub index: usize,
    pub to: GroupId,
    /// Insertion point before the source tab is removed.
    pub boundary: usize,
    pub width: f32,
}

#[derive(Default)]
pub(crate) struct TabReorder {
    pub target: Option<Target>,
    offsets: HashMap<GroupId, Vec<Spring>>,
    trailing: HashMap<GroupId, Tween>,
    counts: HashMap<GroupId, usize>,
}

impl TabReorder {
    pub fn set_target(&mut self, target: Target) -> bool {
        if self.target == Some(target) {
            return false;
        }
        self.target = Some(target);
        true
    }

    /// Remove the preview target. The next sample eases all retained geometry
    /// back to zero.
    pub fn clear(&mut self) -> bool {
        self.target.take().is_some()
    }

    /// Forget all retained geometry after the model has committed the move.
    pub fn reset(&mut self) {
        self.target = None;
        self.offsets.clear();
        self.trailing.clear();
        self.counts.clear();
    }

    pub fn prune(&mut self, groups: &[GroupId]) {
        self.offsets.retain(|group, _| groups.contains(group));
        self.trailing.retain(|group, _| groups.contains(group));
        self.counts.retain(|group, _| groups.contains(group));
        if self.target.is_some_and(|target| !groups.contains(&target.from) || !groups.contains(&target.to)) {
            self.clear();
        }
    }

    /// Invalidate a pending preview when either participating pane changed
    /// size since its last sampled layout. The caller can then stop the drag.
    pub fn invalidate_changed_counts(&mut self, current: &[(GroupId, usize)]) -> bool {
        let Some(target) = self.target else { return false; };
        let count_for = |group: GroupId| current.iter().find(|(id, _)| *id == group).map(|(_, count)| *count);
        let changed = [target.from, target.to].into_iter().any(|group| {
            count_for(group).is_none_or(|count| self.counts.get(&group) != Some(&count))
        }) || count_for(target.from).is_some_and(|count| target.index >= count);
        if changed { self.reset(); }
        changed
    }

    /// Sample horizontal offsets for each tab and an optional trailing spacer.
    /// The strip owns the 4 px flex gap, so trailing width excludes that gap.
    pub fn sample(
        &mut self,
        group: GroupId,
        widths: &[f32],
        now: Instant,
        animate: bool,
    ) -> (Vec<f32>, f32, bool) {
        let desired = self.target_offsets(group, widths);
        let extra = self.target_trailing(group, widths);
        if self.counts.insert(group, widths.len()).is_some_and(|old| old != widths.len()) {
            self.offsets.remove(&group);
            self.trailing.remove(&group);
        }
        let tweens = self.offsets.entry(group).or_default();
        tweens.resize_with(widths.len(), Spring::default);
        tweens.truncate(widths.len());

        let mut active = false;
        let offsets = tweens
            .iter_mut()
            .zip(desired)
            .map(|(tween, target)| {
                let (value, moving) = tween.sample(target, !animate, now, DURATION_MS);
                active |= moving;
                value
            })
            .collect();
        let (extra, moving) = self
            .trailing
            .entry(group)
            .or_default()
            .sample(extra, !animate, now, DURATION_MS);
        active |= moving;

        (offsets, extra, active)
    }

    /// Convert a pre-removal insertion boundary to the post-removal index.
    pub fn destination(
        &self,
        group: GroupId,
        source: GroupId,
        index: usize,
        count: usize,
    ) -> Option<usize> {
        let target = self.target?;
        if target.to != group || target.from != source || target.index != index || target.boundary > count {
            return None;
        }
        if group == source {
            if index >= count {
                return None;
            }
            Some(target.boundary - usize::from(target.boundary > index))
        } else {
            Some(target.boundary)
        }
    }

    fn target_offsets(&self, group: GroupId, widths: &[f32]) -> Vec<f32> {
        let mut offsets = vec![0.; widths.len()];
        let Some(target) = self.target else {
            return offsets;
        };

        if target.from == target.to && group == target.from {
            if target.index >= widths.len() {
                return offsets;
            }
            let destination = self.destination(group, target.from, target.index, widths.len())
                .unwrap_or(target.index);
            let mut order: Vec<usize> = (0..widths.len()).filter(|&i| i != target.index).collect();
            order.insert(destination.min(order.len()), target.index);
            let mut x = vec![0.; widths.len()];
            let mut left = 0.;
            for i in order {
                x[i] = left;
                left += widths[i] + TAB_GAP;
            }
            let mut old_x = 0.;
            for (i, width) in widths.iter().enumerate() {
                offsets[i] = x[i] - old_x;
                old_x += width + TAB_GAP;
            }
            return offsets;
        }

        if group == target.to && group != target.from {
            let boundary = target.boundary.min(widths.len());
            for offset in offsets.iter_mut().skip(boundary) {
                *offset = target.width + TAB_GAP;
            }
        }
        offsets
    }

    fn target_trailing(&self, group: GroupId, widths: &[f32]) -> f32 {
        let Some(target) = self.target else {
            return 0.;
        };
        if target.from != target.to && group == target.to && target.boundary <= widths.len() {
            target.width.max(0.)
        } else {
            0.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn id(value: u64) -> GroupId {
        GroupId(value)
    }

    fn target(from: GroupId, index: usize, to: GroupId, boundary: usize, width: f32) -> Target {
        Target { from, index, to, boundary, width }
    }

    #[test]
    fn same_pane_variable_width_reorder_and_end_boundary() {
        let now = Instant::now();
        let mut reorder = TabReorder::default();
        let widths = [30., 50., 20.];
        // Boundary 3 is before-removal; moving tab 0 lands at final index 2.
        reorder.set_target(target(id(1), 0, id(1), 3, 30.));
        let (offsets, extra, _) = reorder.sample(id(1), &widths, now, false);
        assert_eq!(offsets, vec![78., -34., -34.]);
        assert_eq!(extra, 0.);
        assert_eq!(reorder.destination(id(1), id(1), 0, widths.len()), Some(2));
        assert_eq!(reorder.destination(id(2), id(1), 0, widths.len()), None);

        reorder.set_target(target(id(1), 0, id(1), 2, 30.));
        assert_eq!(reorder.destination(id(1), id(1), 0, widths.len()), Some(1));
    }

    #[test]
    fn retargeting_is_continuous_and_motion_can_be_cancelled() {
        let now = Instant::now();
        let mut reorder = TabReorder::default();
        let widths = [30., 40., 50.];
        reorder.set_target(target(id(1), 0, id(1), 3, 30.));
        reorder.sample(id(1), &widths, now, true);
        let mid = now + Duration::from_millis(60);
        let before = reorder.sample(id(1), &widths, mid, true).0;
        reorder.set_target(target(id(1), 0, id(1), 1, 30.));
        let after = reorder.sample(id(1), &widths, mid, true).0;
        assert_eq!(before, after);
        assert!(reorder.clear());
        let (settling, _, active) = reorder.sample(id(1), &widths, mid, true);
        assert_eq!(settling, after);
        assert!(active);
        assert!(!reorder.clear());
        let (zero, extra, active) = reorder.sample(id(1), &widths, mid + Duration::from_secs(1), true);
        assert_eq!(zero, vec![0.; 3]);
        assert_eq!(extra, 0.);
        assert!(!active);
    }

    #[test]
    fn cross_pane_insertion_accounts_for_gap_and_end_spacer() {
        let now = Instant::now();
        let mut reorder = TabReorder::default();
        reorder.set_target(target(id(1), 1, id(2), 1, 45.));
        let (offsets, extra, _) = reorder.sample(id(2), &[20., 30., 40.], now, false);
        assert_eq!(offsets, vec![0., 49., 49.]);
        assert_eq!(extra, 45.);
        reorder.set_target(target(id(1), 1, id(2), 3, 45.));
        let (offsets, extra, _) = reorder.sample(id(2), &[20., 30., 40.], now, false);
        assert_eq!(offsets, vec![0., 0., 0.]);
        assert_eq!(extra, 45.);
        reorder.set_target(target(id(1), 7, id(2), 0, 45.));
        assert_eq!(reorder.destination(id(2), id(1), 7, 3), Some(0));
    }

    #[test]
    fn changed_source_or_destination_counts_cancel_the_drag() {
        let now=Instant::now();let mut reorder=TabReorder::default();
        reorder.sample(id(1),&[30.,40.],now,false);
        reorder.sample(id(2),&[50.],now,false);
        reorder.set_target(target(id(1),1,id(2),0,40.));
        assert!(!reorder.invalidate_changed_counts(&[(id(1),2),(id(2),1),(id(3),9)]));
        assert!(reorder.invalidate_changed_counts(&[(id(1),3),(id(2),1)]));
        assert!(reorder.target.is_none());
        reorder.sample(id(1),&[30.,40.],now,false);
        reorder.sample(id(2),&[50.],now,false);
        reorder.set_target(target(id(1),1,id(2),0,40.));
        assert!(reorder.invalidate_changed_counts(&[(id(1),2),(id(2),2)]));
    }

    #[test]
    fn missing_panes_and_invalid_sources_cancel_but_idle_changes_do_not() {
        let now=Instant::now();let mut reorder=TabReorder::default();
        assert!(!reorder.invalidate_changed_counts(&[(id(1),1)]));
        reorder.sample(id(1),&[30.],now,false);
        reorder.sample(id(2),&[40.],now,false);
        reorder.set_target(target(id(1),0,id(2),0,30.));
        assert!(reorder.invalidate_changed_counts(&[(id(1),1)]));
        reorder.sample(id(1),&[30.],now,false);
        reorder.set_target(target(id(1),2,id(1),0,30.));
        assert!(reorder.invalidate_changed_counts(&[(id(1),1)]));
    }

    #[test]
    fn disabled_animations_and_reset_are_immediate() {
        let now = Instant::now();
        let mut reorder = TabReorder::default();
        reorder.set_target(target(id(1), 0, id(1), 2, 25.));
        let (offsets, _, active) = reorder.sample(id(1), &[25., 35.], now, false);
        assert_eq!(offsets, vec![39., -29.]);
        assert!(!active);
        reorder.reset();
        let (offsets, extra, active) = reorder.sample(id(1), &[25., 35.], now, true);
        assert_eq!(offsets, vec![0., 0.]);
        assert_eq!(extra, 0.);
        assert!(!active);
    }
}
