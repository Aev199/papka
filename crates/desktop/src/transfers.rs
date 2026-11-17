//! Copies and moves of the window: the one running, the one waiting for an
//! answer about name conflicts, and those asked for meanwhile.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use filemanager_core::operations::{Conflict, CopyControl, DropZone, Pruned};

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

/// The running copy or move as its panel shows it.
pub(crate) struct Live {
    /// «Копирование» or «Перемещение».
    pub(crate) action: &'static str,
    pub(crate) target: PathBuf,
    /// «Проверка копии…» and the like; empty while plainly copying.
    pub(crate) phase: String,
    pub(crate) processed: usize,
    pub(crate) total: usize,
    pub(crate) bytes: u64,
    pub(crate) total_bytes: u64,
    /// «2 мин», once there is something to estimate from.
    pub(crate) left: Option<String>,
    pub(crate) current: Option<PathBuf>,
}

/// One item a transfer could not finish, with the whole error text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) path: PathBuf,
    pub(crate) error: String,
}

/// How a copy or move ended; stays on screen while it lists errors.
pub(crate) struct Report {
    pub(crate) id: u64,
    pub(crate) title: String,
    pub(crate) target: PathBuf,
    pub(crate) failures: Vec<Failure>,
}

/// «Скопировано: 5, ошибок: 2, отменено: 3» — only the parts that happened.
pub(crate) fn report_title(done: &str, ok: usize, failed: usize, cancelled: usize) -> String {
    let mut title = format!("{done}: {ok}");
    if failed > 0 { title.push_str(&format!(", ошибок: {failed}")); }
    if cancelled > 0 { title.push_str(&format!(", отменено: {cancelled}")); }
    title
}

/// An error a cancelled transfer gives for the items it stopped: the
/// queue's `Interrupted`, or Windows' "request aborted" (1235) from a copy
/// stopped half way.
pub(crate) fn is_cancellation(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::Interrupted || error.raw_os_error() == Some(1235)
}

/// What the conflict dialog says under a name: which side is newer and
/// the two sizes, «переносимый новее · 10 Б, в папке 3 Б».
pub(crate) fn conflict_note(conflict: &Conflict) -> String {
    use filemanager_core::format::format_size;
    let mut parts = Vec::new();
    match conflict.incoming_newer() {
        Some(true) => parts.push("переносимый новее".to_string()),
        Some(false) => parts.push("в папке новее".to_string()),
        None if conflict.source_modified.is_some() && conflict.existing_modified.is_some() => parts.push("даты совпадают".to_string()),
        None => {}
    }
    match (conflict.source_size, conflict.existing_size) {
        (Some(a), Some(b)) if a == b => parts.push(format!("оба {}", format_size(a))),
        (Some(a), Some(b)) => parts.push(format!("{}, в папке {}", format_size(a), format_size(b))),
        _ => {}
    }
    parts.join(" · ")
}

/// Status text for items a transfer left out, empty when none were.
pub(crate) fn pruned_note(pruned: &Pruned) -> String {
    let mut parts = Vec::new();
    if !pruned.nested.is_empty() { parts.push(format!("вложенных в выбранные папки: {}", pruned.nested.len())); }
    if !pruned.in_place.is_empty() { parts.push(format!("уже в этой папке: {}", pruned.in_place.len())); }
    if parts.is_empty() { String::new() } else { format!("Пропущено — {}", parts.join(", ")) }
}

/// The errors as plain text, to paste into a message.
pub(crate) fn failures_text(title: &str, failures: &[Failure]) -> String {
    let mut text = String::from(title);
    for failure in failures {
        text.push_str(&format!("\n{}: {}", failure.path.display(), failure.error));
    }
    text
}

#[derive(Default)]
pub(crate) struct Transfers {
    /// What the transfer panel shows of the running copy or move.
    pub(crate) live: Option<Live>,
    /// The result of the last one, until dismissed.
    pub(crate) report: Option<Report>,
    /// The panel was folded away by the user; an error unfolds it.
    pub(crate) panel_hidden: bool,
    reports: u64,
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
    /// What the starting transfer left out («Пропущено — …»): said again
    /// when it ends, since its progress replaces the status meanwhile.
    pub(crate) skipped_note: Option<String>,
    /// Transfer paused on name conflicts, with the conflicting items.
    pub(crate) pending: Option<(Transfer, Vec<Conflict>)>,
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

    /// A transfer starts: its panel appears, and a result without errors
    /// makes way (a list of errors stays until it is read).
    pub(crate) fn begin(&mut self, action: &'static str, target: PathBuf) {
        if self.report.as_ref().is_some_and(|report| report.failures.is_empty()) { self.report = None; }
        self.panel_hidden = false;
        self.live = Some(Live { action, target, phase: String::new(), processed: 0, total: 0, bytes: 0, total_bytes: 0,
            left: None, current: None });
    }

    /// A transfer ended. Returns the report's id when it may fade by
    /// itself: nothing failed, so there is nothing to read.
    pub(crate) fn finish(&mut self, title: String, target: PathBuf, failures: Vec<Failure>) -> Option<u64> {
        self.live = None;
        self.reports = self.reports.wrapping_add(1);
        // Errors nobody closed yet are not replaced by the next result
        // (a queued transfer may end seconds later): they pile up.
        let report = match self.report.take().filter(|unread| !unread.failures.is_empty()) {
            Some(mut unread) => {
                if !failures.is_empty() {
                    unread.failures.extend(failures);
                    unread.title = format!("Ошибок в переносах: {}", unread.failures.len());
                }
                Report { id: self.reports, ..unread }
            }
            None => Report { id: self.reports, title, target, failures },
        };
        let quiet = report.failures.is_empty();
        // Errors are shown even if the panel was folded away.
        if !quiet { self.panel_hidden = false; }
        self.report = Some(report);
        quiet.then_some(self.reports)
    }

    /// Removes report `id` if it is still the one shown.
    pub(crate) fn dismiss(&mut self, id: u64) {
        if self.report.as_ref().is_some_and(|report| report.id == id) { self.report = None; }
    }

    pub(crate) fn panel_visible(&self) -> bool {
        !self.panel_hidden && (self.live.is_some() || self.report.is_some())
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

    fn conflict(source: Option<u64>, existing: Option<u64>) -> Conflict {
        Conflict { source: PathBuf::from("a/x"), source_size: source, existing_size: existing,
            source_modified: None, existing_modified: None }
    }

    #[test]
    fn conflict_note_names_the_newer_side_and_sizes() {
        let now = std::time::SystemTime::now();
        let mut c = conflict(Some(10), Some(3));
        c.source_modified = Some(now);
        c.existing_modified = Some(now - std::time::Duration::from_secs(60));
        assert_eq!(conflict_note(&c), "переносимый новее · 10 Б, в папке 3 Б");
        c.existing_modified = Some(now + std::time::Duration::from_secs(60));
        c.existing_size = Some(10);
        assert_eq!(conflict_note(&c), "в папке новее · оба 10 Б");
        assert_eq!(conflict_note(&conflict(None, None)), "");
    }

    #[test]
    fn pruned_note_counts_both_kinds() {
        let mut pruned = Pruned::default();
        assert_eq!(pruned_note(&pruned), "");
        pruned.nested.push("a".into());
        pruned.in_place.push("b".into());
        pruned.in_place.push("c".into());
        assert_eq!(pruned_note(&pruned), "Пропущено — вложенных в выбранные папки: 1, уже в этой папке: 2");
    }

    #[test]
    fn an_open_conflict_dialog_holds_the_queue() {
        let mut transfers = Transfers::default();
        transfers.queue.push_back(transfer("b"));
        transfers.pending = Some((transfer("a"), vec![conflict(None, None)]));
        assert!(transfers.occupied());
        assert!(transfers.next().is_none());
        transfers.pending = None;
        assert!(transfers.next().is_some());
    }

    #[test]
    fn a_clean_result_fades_and_errors_stay() {
        let mut transfers = Transfers::default();
        transfers.begin("Копирование", "t".into());
        assert!(transfers.panel_visible());
        transfers.panel_hidden = true;
        assert!(!transfers.panel_visible());

        let id = transfers.finish(report_title("Скопировано", 3, 0, 0), "t".into(), Vec::new()).unwrap();
        assert!(transfers.live.is_none());
        transfers.dismiss(id + 1);
        assert!(transfers.report.is_some(), "another report's timer does not close this one");
        transfers.dismiss(id);
        assert!(transfers.report.is_none());

        let failure = Failure { path: "a/x.dwg".into(), error: "Отказано в доступе".into() };
        assert_eq!(transfers.finish(report_title("Скопировано", 3, 1, 0), "t".into(), vec![failure.clone()]), None);
        assert!(transfers.panel_visible(), "errors unfold a hidden panel");
        // The next transfer starts: the unread errors stay beside it.
        transfers.begin("Перемещение", "u".into());
        assert_eq!(transfers.report.as_ref().unwrap().failures, vec![failure.clone()]);
        // Its clean result does not wipe them, and more errors join them.
        assert_eq!(transfers.finish(report_title("Перемещено", 1, 0, 0), "u".into(), Vec::new()), None);
        assert_eq!(transfers.report.as_ref().unwrap().title, "Скопировано: 3, ошибок: 1");
        let second = Failure { path: "b/y.dwg".into(), error: "Диск переполнен".into() };
        transfers.finish(report_title("Перемещено", 0, 1, 0), "u".into(), vec![second]);
        let report = transfers.report.as_ref().unwrap();
        assert_eq!((report.title.as_str(), report.failures.len()), ("Ошибок в переносах: 2", 2));
        let id = report.id;
        transfers.dismiss(id);
        // A clean result is replaced by the next start.
        transfers.finish(report_title("Перемещено", 1, 0, 0), "u".into(), Vec::new());
        transfers.begin("Копирование", "v".into());
        assert!(transfers.report.is_none());
    }

    #[test]
    fn stopped_items_are_told_apart_from_failures() {
        use std::io::{Error, ErrorKind};
        assert!(is_cancellation(&Error::new(ErrorKind::Interrupted, "Copy cancelled")));
        assert!(is_cancellation(&Error::from_raw_os_error(1235)));
        assert!(!is_cancellation(&Error::new(ErrorKind::PermissionDenied, "Отказано в доступе")));
        assert!(!is_cancellation(&Error::from_raw_os_error(112)), "a full disk is an error, cancelled or not");
    }

    #[test]
    fn titles_and_error_text_name_only_what_happened() {
        assert_eq!(report_title("Скопировано", 5, 0, 0), "Скопировано: 5");
        assert_eq!(report_title("Перемещено", 5, 2, 3), "Перемещено: 5, ошибок: 2, отменено: 3");
        let failures = [Failure { path: "a/x.dwg".into(), error: "Отказано в доступе".into() }];
        assert_eq!(failures_text("Скопировано: 0, ошибок: 1", &failures), "Скопировано: 0, ошибок: 1\na/x.dwg: Отказано в доступе");
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
