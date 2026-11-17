//! Folder listings kept between renders: what is loaded, what is being
//! read, and which result may still be stored when a scan comes back.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Listings kept at most; folders on screen do not count against it.
const SLOTS: usize = 16;

pub(crate) struct DirectoryCache<L> {
    pub(crate) cache: HashMap<PathBuf, Arc<L>>,
    pub(crate) errors: HashMap<PathBuf, String>,
    loading: HashMap<PathBuf, u64>,
    /// Changed while being read: read once more when the scan returns.
    dirty: HashSet<PathBuf>,
    order: VecDeque<PathBuf>,
    request: u64,
}

impl<L> Default for DirectoryCache<L> {
    fn default() -> Self {
        Self { cache: HashMap::new(), errors: HashMap::new(), loading: HashMap::new(), dirty: HashSet::new(),
            order: VecDeque::new(), request: 0 }
    }
}

/// What became of a scan's result.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Finished {
    /// A newer scan of the folder is running; this one is dropped.
    Stale,
    /// The folder changed meanwhile: read it again.
    Reload,
    Stored,
}

impl<L> DirectoryCache<L> {
    pub(crate) fn is_loading(&self, folder: &Path) -> bool { self.loading.contains_key(folder) }

    /// The number of the scan to start, or `None` when nothing is to be read:
    /// the folder is known (and not forced), or a scan is already running.
    pub(crate) fn begin(&mut self, folder: &Path, force: bool) -> Option<u64> {
        if !force && (self.cache.contains_key(folder) || self.loading.contains_key(folder) || self.errors.contains_key(folder)) {
            return None;
        }
        if force && self.loading.contains_key(folder) {
            // Keep the event until the in-flight scan finishes. A scan can
            // otherwise overwrite the invalidation with a stale snapshot.
            self.dirty.insert(folder.to_path_buf());
            return None;
        }
        // A monotonically increasing request ID prevents a slower, outdated
        // result from replacing a user-requested refresh.
        self.request = self.request.wrapping_add(1);
        if force { self.cache.remove(folder); }
        self.errors.remove(folder);
        self.loading.insert(folder.to_path_buf(), self.request);
        Some(self.request)
    }

    /// Stores the result of scan `request`. Folders in `visible` are never
    /// evicted: dropping one would load it again at once, and so on forever.
    pub(crate) fn finish(&mut self, folder: &Path, request: u64, result: Result<L, String>, visible: &HashSet<PathBuf>) -> Finished {
        if self.loading.get(folder) != Some(&request) { return Finished::Stale; }
        self.loading.remove(folder);
        if self.dirty.remove(folder) { return Finished::Reload; }
        match result {
            Ok(listing) => {
                self.order.retain(|old| old != folder);
                self.order.push_back(folder.to_path_buf());
                self.cache.insert(folder.to_path_buf(), Arc::new(listing));
                let mut kept = VecDeque::new();
                while self.order.len() + kept.len() > SLOTS {
                    let Some(old) = self.order.pop_front() else { break };
                    if visible.contains(&old) { kept.push_back(old); } else { self.cache.remove(&old); }
                }
                kept.append(&mut self.order);
                self.order = kept;
            }
            Err(error) => { self.errors.insert(folder.to_path_buf(), error); }
        }
        Finished::Stored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> HashSet<PathBuf> { HashSet::new() }

    #[test]
    fn a_known_folder_is_not_read_again_unless_forced() {
        let mut dirs = DirectoryCache::<u32>::default();
        let a = Path::new("a");
        let first = dirs.begin(a, false).unwrap();
        assert!(dirs.is_loading(a));
        assert_eq!(dirs.begin(a, false), None, "already being read");
        assert_eq!(dirs.finish(a, first, Ok(1), &none()), Finished::Stored);
        assert_eq!(dirs.begin(a, false), None, "already known");
        assert!(dirs.begin(a, true).is_some());
        assert!(!dirs.cache.contains_key(a), "a forced read drops the old listing");
    }

    #[test]
    fn a_change_during_a_scan_reads_the_folder_once_more() {
        let mut dirs = DirectoryCache::<u32>::default();
        let a = Path::new("a");
        let first = dirs.begin(a, false).unwrap();
        assert_eq!(dirs.begin(a, true), None);
        assert_eq!(dirs.finish(a, first, Ok(1), &none()), Finished::Reload);
        assert!(!dirs.cache.contains_key(a), "the stale snapshot is not stored");
        let second = dirs.begin(a, true).unwrap();
        assert_eq!(dirs.finish(a, second, Ok(2), &none()), Finished::Stored);
        assert_eq!(dirs.cache.get(a).map(|listing| **listing), Some(2));
    }

    #[test]
    fn an_outdated_scan_does_not_replace_a_newer_one() {
        let mut dirs = DirectoryCache::<u32>::default();
        let a = Path::new("a");
        let first = dirs.begin(a, false).unwrap();
        assert_eq!(dirs.finish(a, first + 1, Ok(9), &none()), Finished::Stale);
        assert!(dirs.is_loading(a));
        assert_eq!(dirs.finish(a, first, Ok(1), &none()), Finished::Stored);
    }

    #[test]
    fn an_error_is_kept_until_a_forced_read() {
        let mut dirs = DirectoryCache::<u32>::default();
        let a = Path::new("a");
        let first = dirs.begin(a, false).unwrap();
        dirs.finish(a, first, Err("нет доступа".into()), &none());
        assert_eq!(dirs.errors.get(a).map(String::as_str), Some("нет доступа"));
        assert_eq!(dirs.begin(a, false), None);
        assert!(dirs.begin(a, true).is_some());
        assert!(dirs.errors.is_empty());
    }

    #[test]
    fn old_listings_are_dropped_but_never_visible_ones() {
        let mut dirs = DirectoryCache::<usize>::default();
        let visible: HashSet<PathBuf> = [PathBuf::from("0")].into();
        for index in 0..SLOTS + 4 {
            let folder = PathBuf::from(index.to_string());
            let request = dirs.begin(&folder, false).unwrap();
            dirs.finish(&folder, request, Ok(index), &visible);
        }
        assert_eq!(dirs.cache.len(), SLOTS);
        assert!(dirs.cache.contains_key(Path::new("0")), "on screen");
        assert!(!dirs.cache.contains_key(Path::new("1")));
        assert!(dirs.cache.contains_key(Path::new("19")));
    }
}
