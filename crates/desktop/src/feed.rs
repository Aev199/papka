//! The feed of changes in the current folder and its subfolders, recorded
//! while Papka shows those folders (names and times only, no contents).

use std::path::{Path, PathBuf};

use filemanager_core::history::{self, Event};
use gpui::{AppContext, Context, Focusable, Window};

use crate::Explorer;

/// Period filter: label and how many days back (None: everything).
pub(crate) const PERIODS: [(&str, Option<i64>); 4] = [
    ("Сегодня", Some(0)),
    ("Неделя", Some(7)),
    ("Месяц", Some(30)),
    ("Всё", None),
];

/// Rows read at most; the feed is for recent work, not an archive.
const LIMIT: usize = 400;

/// Rows drawn at first and added per «Показать ещё»: the panel is laid out
/// on every frame, and hundreds of rows made scrolling the files stutter.
pub(crate) const PAGE: usize = 50;

/// A line of the feed, worked out once when the events arrive: local dates
/// and relative paths are too slow to compute for every row on every frame.
#[derive(Debug, PartialEq)]
pub(crate) struct FeedRow {
    /// Position in `Feed::events` of the run's first event.
    pub(crate) index: usize,
    /// Same file, same change, same day: one row with the count.
    pub(crate) repeats: usize,
    pub(crate) day: String,
    pub(crate) clock: String,
    pub(crate) name: String,
    /// Where in the folder, when not directly in it.
    pub(crate) place: Option<String>,
}

/// The rows for `events` of `folder`: runs of the same plain entry merged.
pub(crate) fn feed_rows(events: &[Event], folder: &Path) -> Vec<FeedRow> {
    let mut rows: Vec<FeedRow> = Vec::new();
    for (index, event) in events.iter().enumerate() {
        let plain = event.comment.trim().is_empty() && event.author.as_deref().is_none_or(str::is_empty);
        let day = event.day_label();
        if let Some(last) = rows.last_mut() {
            let first = &events[last.index];
            if plain && first.path == event.path && first.kind == event.kind && last.day == day {
                last.repeats += 1;
                continue;
            }
        }
        rows.push(FeedRow {
            index,
            repeats: 1,
            day,
            clock: event.clock_label(),
            name: filemanager_core::browser::display_name(&event.path),
            place: event.path.parent().and_then(|parent| parent.strip_prefix(folder).ok())
                .map(|relative| relative.display().to_string()).filter(|relative| !relative.is_empty()),
        });
    }
    rows
}

#[derive(Default)]
pub(crate) struct Feed {
    pub(crate) folder: Option<PathBuf>,
    pub(crate) period: usize,
    pub(crate) events: Vec<Event>,
    shown_period: usize,
    loading: bool,
    /// Files changed since the feed was read.
    pub(crate) dirty: bool,
    /// The entry whose comment is being written.
    pub(crate) editing: Option<i64>,
    /// `events` as drawn; rebuilt whenever they change.
    pub(crate) rows: Vec<FeedRow>,
    /// How many rows are drawn.
    pub(crate) shown: usize,
}

impl Feed {
    pub(crate) fn new() -> Feed {
        Feed { period: 1, shown_period: 1, shown: PAGE, ..Feed::default() }
    }

    fn rebuild(&mut self) {
        self.rows = match &self.folder {
            Some(folder) => feed_rows(&self.events, folder),
            None => Vec::new(),
        };
    }
}

impl Explorer {
    /// Reads the feed for the active pane's folder when it is out of date.
    pub(crate) fn load_feed(&mut self, cx: &mut Context<Self>) {
        if !self.show_inspector || self.feed.loading { return; }
        let Some(journal) = self.journal.clone() else { return };
        let folder = self.pane_path(self.active_side());
        let period = self.feed.period;
        if self.feed.folder.as_ref() == Some(&folder) && self.feed.shown_period == period && !self.feed.dirty { return; }
        self.feed.loading = true;
        self.feed.dirty = false;
        let since = PERIODS[period].1.map(history::days_ago_ms);
        let target = folder.clone();
        let task = cx.background_spawn(async move { journal.folder_events(&target, since, LIMIT) });
        cx.spawn(async move |this, cx| {
            let events = task.await;
            let _ = this.update(cx, |this, cx| {
                this.feed.loading = false;
                if this.feed.folder.as_ref() != Some(&folder) || this.feed.shown_period != period {
                    this.feed.events.clear();
                    this.feed.shown = PAGE;
                }
                this.feed.folder = Some(folder);
                this.feed.shown_period = period;
                match events {
                    Ok(events) => this.feed.events = events,
                    Err(error) => this.status = format!("Лента изменений недоступна: {error}"),
                }
                this.feed.rebuild();
                cx.notify();
            });
        }).detach();
    }

    /// Opens the comment fields under a feed entry, filled with what it has.
    pub(crate) fn edit_feed_comment(&mut self, event_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.feed.editing == Some(event_id) {
            self.feed.editing = None;
            cx.notify();
            return;
        }
        let Some(event) = self.feed.events.iter().find(|event| event.id == event_id) else { return };
        let (comment, author) = (event.comment.clone(), event.author.clone().unwrap_or_default());
        self.feed.editing = Some(event_id);
        self.comment_input.update(cx, |input, cx| input.set_value(comment, window, cx));
        self.author_input.update(cx, |input, cx| input.set_value(author, window, cx));
        let focus = self.comment_input.focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Saves who changed the file and why; Enter in the comment does it too.
    pub(crate) fn save_feed_comment(&mut self, cx: &mut Context<Self>) {
        let (Some(event_id), Some(journal)) = (self.feed.editing, self.journal.clone()) else { return };
        let comment = self.comment_input.read(cx).value().trim().to_string();
        let author = self.author_input.read(cx).value().trim().to_string();
        let (saved_comment, saved_author) = (comment.clone(), author.clone());
        let task = cx.background_spawn(async move {
            journal.annotate(event_id, (!author.is_empty()).then_some(author.as_str()), &comment)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(true) => {
                        if let Some(event) = this.feed.events.iter_mut().find(|event| event.id == event_id) {
                            event.comment = saved_comment;
                            event.author = (!saved_author.is_empty()).then_some(saved_author);
                        }
                        this.feed.rebuild();
                        if this.feed.editing == Some(event_id) { this.feed.editing = None; }
                        this.status = "Комментарий сохранён".into();
                    }
                    Ok(false) => this.status = "Запись ленты не найдена".into(),
                    Err(error) => this.status = format!("Комментарий не сохранён: {error}"),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(crate) fn show_more_feed(&mut self, cx: &mut Context<Self>) {
        self.feed.shown += PAGE;
        cx.notify();
    }

    pub(crate) fn set_feed_period(&mut self, period: usize, cx: &mut Context<Self>) {
        self.feed.period = period.min(PERIODS.len() - 1);
        cx.notify();
    }

    /// A click on a feed row: the file, selected in its folder.
    pub(crate) fn open_feed_event(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let side = self.active_side();
        let Some(parent) = path.parent().map(|parent| parent.to_path_buf()) else { return };
        if !parent.is_dir() {
            self.status = format!("Папки «{}» больше нет", parent.display());
            cx.notify();
            return;
        }
        let select = path.exists().then(|| path.clone());
        if select.is_none() { self.status = format!("Файла «{}» больше нет", path.display()); }
        self.navigate_side(side, &parent, select, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: i64, path: &str, kind: &str, comment: &str) -> Event {
        Event { id, path: PathBuf::from(path), kind: kind.into(), observed_ms: 1_700_000_000_000, recorded_by: String::new(), author: None, comment: comment.into() }
    }

    #[test]
    fn runs_merge_and_places_are_relative() {
        let events = vec![
            event(1, "/w/a.txt", "modified", ""),
            event(2, "/w/a.txt", "modified", ""),
            event(3, "/w/sub/b.txt", "modified", ""),
            event(4, "/w/sub/b.txt", "modified", "проверил"),
        ];
        let rows = feed_rows(&events, Path::new("/w"));
        assert_eq!(rows.len(), 3);
        assert_eq!((rows[0].index, rows[0].repeats, rows[0].name.as_str()), (0, 2, "a.txt"));
        assert_eq!(rows[0].place, None);
        assert_eq!(rows[1].place.as_deref(), Some("sub"));
        // A commented entry is never folded into a run.
        assert_eq!((rows[2].index, rows[2].repeats), (3, 1));
    }
}
