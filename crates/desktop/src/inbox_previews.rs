//! Inbox pictures have their own small queue: no Shell helper or tile-view
//! visibility gate, no batch barrier before the first image can appear.
use std::{collections::{HashMap, HashSet, VecDeque}, path::PathBuf};
use filemanager_core::inbox_preview::Key;

pub(crate) const SHOWN: usize = 40;
const WORKERS: usize = 2;

#[derive(Default)]
pub(crate) struct InboxPreviews {
    wanted: HashSet<Key>,
    queued: VecDeque<Key>,
    active: HashSet<Key>,
    ready: HashMap<Key, Result<PathBuf, String>>,
}

impl InboxPreviews {
    pub(crate) fn select(&mut self, keys: Vec<Key>) {
        self.wanted = keys.iter().cloned().collect();
        // A reopened project checks its disk cache off-thread (old cache
        // files may have been pruned). Current pictures never reload.
        self.ready.retain(|key, result| self.wanted.contains(key) && result.is_ok());
        let mut seen = HashSet::new();
        // Refreshes keep both ready pictures and in-flight requests.
        self.queued = keys.into_iter().filter(|key| seen.insert(key.clone()) && !self.ready.contains_key(key) && !self.active.contains(key)).collect();
    }

    pub(crate) fn next(&mut self) -> Option<Key> {
        if self.active.len() >= WORKERS { return None; }
        let key = self.queued.pop_front()?;
        self.active.insert(key.clone());
        Some(key)
    }

    pub(crate) fn finish(&mut self, key: Key, result: Result<PathBuf, String>) {
        self.active.remove(&key);
        if self.wanted.contains(&key) { self.ready.insert(key, result); }
    }

    pub(crate) fn get(&self, key: &Key) -> Option<&Result<PathBuf, String>> { self.ready.get(key) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(name: &str) -> Key { Key { source: name.into(), size: 1, modified: None } }

    #[test]
    fn an_old_completion_cannot_replace_a_changed_picture() {
        let mut previews = InboxPreviews::default();
        let old = key("same.png");
        let new = Key { size: 2, ..old.clone() };
        previews.select(vec![old.clone()]);
        assert_eq!(previews.next(), Some(old.clone()));
        previews.select(vec![new.clone()]);
        assert_eq!(previews.next(), Some(new.clone()));
        previews.finish(old.clone(), Ok("old-small.png".into()));
        assert!(previews.get(&old).is_none());
        previews.finish(new.clone(), Ok("new-small.png".into()));
        assert_eq!(previews.get(&new).unwrap().as_ref().unwrap(), &PathBuf::from("new-small.png"));
    }

    #[test]
    fn newest_first_with_two_workers_and_incremental_completion() {
        let mut previews = InboxPreviews::default();
        previews.select(vec![key("new"), key("new"), key("middle"), key("old")]);
        assert_eq!(previews.next(), Some(key("new")));
        assert_eq!(previews.next(), Some(key("middle")));
        assert_eq!(previews.next(), None);
        previews.finish(key("middle"), Ok("small.png".into()));
        assert!(previews.get(&key("middle")).unwrap().is_ok(), "one slow picture does not hide completed ones");
        assert_eq!(previews.next(), Some(key("old")));
        previews.select(vec![key("new"), key("middle"), key("old")]);
        assert_eq!(previews.next(), None, "polling does not duplicate running work");
    }

    #[test]
    fn switching_projects_drops_queued_work_and_rechecks_disk_cache_on_return() {
        let mut previews = InboxPreviews::default();
        previews.select(vec![key("a"), key("b"), key("c")]);
        previews.next();
        previews.finish(key("a"), Ok("a-small.png".into()));
        previews.next();
        previews.select(vec![key("other")]);
        assert_eq!(previews.next(), Some(key("other")));
        previews.finish(key("b"), Ok("b-small.png".into()));
        assert!(previews.get(&key("b")).is_none());
        previews.finish(key("other"), Err("bad picture".into()));
        previews.select(vec![key("a"), key("other")]);
        assert_eq!(previews.next(), Some(key("a")), "reopening rechecks the disk cache in the background");
        assert_eq!(previews.next(), Some(key("other")), "a failed I/O request can recover on the next poll");
    }
}
