//! Items marked with Ctrl/Shift clicks, the marquee or Ctrl+A, in display
//! order, with a set beside the list: every visible row asks "am I marked?"
//! on every frame. A plain list made that a pass over all marked paths per
//! row, and on Windows each path comparison parses both paths (std's fast
//! byte comparison is skipped for paths with a drive or server prefix).

use std::collections::HashSet;
use std::ops::Deref;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub(crate) struct Marked {
    list: Vec<PathBuf>,
    set: HashSet<PathBuf>,
}

impl Marked {
    pub(crate) fn contains(&self, path: &Path) -> bool { self.set.contains(path) }

    pub(crate) fn clear(&mut self) {
        self.list.clear();
        self.set.clear();
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&Path) -> bool) {
        self.list.retain(|path| keep(path));
        self.set = self.list.iter().cloned().collect();
    }

    /// Adds the paths not marked yet, at the end.
    pub(crate) fn extend(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if self.set.insert(path.clone()) { self.list.push(path); }
        }
    }

    /// Marks `path` if it is not, unmarks it otherwise.
    pub(crate) fn toggle(&mut self, path: &Path) {
        if self.set.remove(path) {
            self.list.retain(|p| p.as_os_str() != path.as_os_str());
        } else {
            self.set.insert(path.to_path_buf());
            self.list.push(path.to_path_buf());
        }
    }

    /// Ctrl-click keeps the focused selection inside the marked set. A
    /// removed path must not remain selected through the single-item fallback.
    pub(crate) fn toggle_selection(&mut self, selected: &mut Option<PathBuf>, path: &Path, order: &[PathBuf]) {
        if self.is_empty() { self.extend(selected.clone()); }
        self.toggle(path);
        self.order_by(order);
        *selected = if self.contains(path) { Some(path.to_path_buf()) } else { self.last().cloned() };
    }

    /// Puts the list in the order of `order` (the folder as shown); paths
    /// not in it go last.
    pub(crate) fn order_by(&mut self, order: &[PathBuf]) {
        let place: std::collections::HashMap<&Path, usize> = order.iter().enumerate().map(|(i, p)| (p.as_path(), i)).collect();
        self.list.sort_by_key(|p| place.get(p.as_path()).copied().unwrap_or(usize::MAX));
    }
}

impl Deref for Marked {
    type Target = [PathBuf];
    fn deref(&self) -> &[PathBuf] { &self.list }
}

impl From<Vec<PathBuf>> for Marked {
    fn from(list: Vec<PathBuf>) -> Self {
        let mut marked = Self::default();
        marked.extend(list);
        marked
    }
}

impl FromIterator<PathBuf> for Marked {
    fn from_iter<I: IntoIterator<Item = PathBuf>>(paths: I) -> Self {
        let mut marked = Self::default();
        marked.extend(paths);
        marked
    }
}

impl From<Marked> for Vec<PathBuf> {
    fn from(marked: Marked) -> Self { marked.list }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_and_answers_membership() {
        let mut marked = Marked::from(vec![PathBuf::from("/a/2"), PathBuf::from("/a/1"), PathBuf::from("/a/2")]);
        assert_eq!(marked.len(), 2);
        assert!(marked.contains(Path::new("/a/1")));
        marked.toggle(Path::new("/a/3"));
        marked.toggle(Path::new("/a/2"));
        assert_eq!(&*marked, &[PathBuf::from("/a/1"), PathBuf::from("/a/3")]);
        assert!(!marked.contains(Path::new("/a/2")));
        marked.order_by(&[PathBuf::from("/a/3"), PathBuf::from("/a/1")]);
        assert_eq!(marked[0], PathBuf::from("/a/3"));
        marked.clear();
        assert!(marked.is_empty() && !marked.contains(Path::new("/a/1")));
    }

    #[test]
    fn ctrl_click_removes_only_the_pointed_item_including_the_last() {
        let order: Vec<_> = ["a", "b", "c"].into_iter().map(PathBuf::from).collect();
        let mut marked = Marked::default();
        let mut selected = Some(order[0].clone());
        marked.toggle_selection(&mut selected, &order[1], &order);
        marked.toggle_selection(&mut selected, &order[2], &order);
        assert_eq!(&*marked, &order);
        for index in [1, 0, 2] {
            let before = marked.to_vec();
            marked.toggle_selection(&mut selected, &order[index], &order);
            assert_eq!(marked.to_vec(), before.into_iter().filter(|p| p != &order[index]).collect::<Vec<_>>());
            assert_ne!(selected.as_ref(), Some(&order[index]));
            assert!(selected.as_ref().is_none_or(|p| marked.contains(p)));
        }
        assert!(selected.is_none() && marked.is_empty());
        marked.toggle_selection(&mut selected, &order[1], &order);
        assert_eq!(&*marked, &[order[1].clone()]);
        marked.toggle_selection(&mut selected, &order[1], &order);
        assert!(selected.is_none() && marked.is_empty());
    }
}
