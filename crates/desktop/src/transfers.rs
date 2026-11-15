//! Copies and moves of the window: the one running, the one waiting for an
//! answer about name conflicts, and those asked for meanwhile.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use filemanager_core::operations::{CopyControl, DropZone};

/// A copy or move waiting to run (or for the conflict dialog).
pub(crate) struct Transfer {
    pub(crate) zone: DropZone,
    pub(crate) target: PathBuf,
    pub(crate) copy: bool,
    /// Occupied names get "name (2).ext" instead of being refused.
    pub(crate) keep_both: bool,
    pub(crate) replace: bool,
    /// Items came from the Drop Zone and go back there if not done.
    pub(crate) restore_failed: bool,
}

#[derive(Default)]
pub(crate) struct Transfers {
    /// A copy or move is running (its progress is shown).
    pub(crate) copying: bool,
    /// Share of the running copy or move done, 0..=1, for the progress bar.
    pub(crate) fraction: Option<f64>,
    /// Any operation that changes files is running: one at a time.
    pub(crate) busy: bool,
    pub(crate) control: Option<Arc<CopyControl>>,
    /// Items a transfer could not finish (e.g. the network dropped): their
    /// target and whether it was a copy, for «Повторить сбойные».
    pub(crate) retry: Option<(Vec<PathBuf>, PathBuf, bool)>,
    /// Transfer paused on name conflicts, with the conflicting items.
    pub(crate) pending: Option<(Transfer, Vec<PathBuf>)>,
    /// Asked for while another ran (or folders of a sync): started in turn.
    pub(crate) queue: VecDeque<Transfer>,
}

impl Transfers {
    /// A new transfer has to wait: files are being changed, or the conflict
    /// dialog of an earlier one is still open.
    pub(crate) fn occupied(&self) -> bool {
        self.copying || self.busy || self.pending.is_some()
    }

    /// The next queued transfer, once nothing else runs.
    pub(crate) fn next(&mut self) -> Option<Transfer> {
        if self.occupied() { None } else { self.queue.pop_front() }
    }

    /// Empties the queue («Отмена»), handing back what was in it.
    pub(crate) fn drain(&mut self) -> Vec<Transfer> {
        self.queue.drain(..).collect()
    }

    /// " · в очереди: 2" for the status line, or nothing.
    pub(crate) fn queue_note(&self) -> String {
        if self.queue.is_empty() { String::new() } else { format!(" · в очереди: {}", self.queue.len()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transfer(target: &str) -> Transfer {
        Transfer { zone: DropZone::default(), target: target.into(), copy: true, keep_both: false, replace: false, restore_failed: false }
    }

    #[test]
    fn queued_transfers_start_in_order_and_only_when_idle() {
        let mut transfers = Transfers::default();
        assert!(!transfers.occupied());
        assert!(transfers.next().is_none());
        transfers.queue.push_back(transfer("a"));
        transfers.queue.push_back(transfer("b"));
        assert_eq!(transfers.queue_note(), " · в очереди: 2");
        transfers.copying = true;
        assert!(transfers.next().is_none(), "a copy is running");
        transfers.copying = false;
        transfers.busy = true;
        assert!(transfers.next().is_none(), "a rename or delete is running");
        transfers.busy = false;
        assert_eq!(transfers.next().unwrap().target, PathBuf::from("a"));
        assert_eq!(transfers.next().unwrap().target, PathBuf::from("b"));
        assert!(transfers.next().is_none());
        assert_eq!(transfers.queue_note(), "");
    }

    #[test]
    fn an_open_conflict_dialog_holds_the_queue() {
        let mut transfers = Transfers::default();
        transfers.queue.push_back(transfer("b"));
        transfers.pending = Some((transfer("a"), vec![PathBuf::from("a/x")]));
        assert!(transfers.occupied());
        assert!(transfers.next().is_none());
        transfers.pending = None;
        assert!(transfers.next().is_some());
    }

    #[test]
    fn cancelling_hands_back_everything_queued() {
        let mut transfers = Transfers::default();
        transfers.queue.push_back(transfer("a"));
        transfers.queue.push_back(transfer("b"));
        assert_eq!(transfers.drain().len(), 2);
        assert!(transfers.queue.is_empty());
    }
}
