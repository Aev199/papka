//! Local presentation order only: no file-system operation is performed here.
use std::{collections::{HashMap, HashSet}, path::{Path, PathBuf}};

use serde::{Deserialize, Serialize};

use crate::browser::Entry;

#[derive(Clone, Default, Serialize, Deserialize)]
struct FolderOrder {
    enabled: bool,
    files: Vec<PathBuf>,
    #[serde(skip)]
    revision: u64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct ManualOrders {
    folders: HashMap<PathBuf, FolderOrder>,
    #[serde(skip)]
    revision: u64,
}

impl ManualOrders {
    pub fn decode(text: &str) -> Self { serde_json::from_str(text).unwrap_or_default() }
    pub fn encode(&self) -> Result<String, serde_json::Error> { serde_json::to_string(self) }
    pub fn revision_for(&self, folder: &Path) -> u64 {
        self.folders.get(folder).map_or(0, |order| order.revision)
    }
    pub fn enabled(&self, folder: &Path) -> bool {
        self.folders.get(folder).is_some_and(|order| order.enabled)
    }

    pub fn set_enabled(&mut self, folder: &Path, enabled: bool) {
        if self.enabled(folder) == enabled { return; }
        self.revision = self.revision.wrapping_add(1);
        let order = self.folders.entry(folder.to_path_buf()).or_default();
        order.enabled = enabled;
        order.revision = self.revision;
    }

    /// Preserve the ordinary folder-first order, and apply the saved order
    /// only to files. Previously unseen files follow in ordinary sorted order.
    pub fn apply(&self, folder: &Path, entries: &[Entry], indices: &mut [usize]) {
        let Some(order) = self.folders.get(folder).filter(|o| o.enabled) else { return; };
        let ranks: HashMap<_, _> = order.files.iter().enumerate().map(|(i, path)| (path.as_path(), i)).collect();
        indices.sort_by_key(|&index| {
            let entry = &entries[index];
            if entry.is_directory { (0, 0) } else { (1, ranks.get(entry.path.as_path()).copied().unwrap_or(usize::MAX)) }
        });
    }

    /// Save a complete, validated presentation order. A caller cannot insert
    /// another directory's paths or duplicates into a folder's order.
    pub fn save(&mut self, folder: &Path, paths: &[PathBuf]) -> bool {
        if !self.enabled(folder) || paths.iter().any(|path| path.parent() != Some(folder)) { return false; }
        let mut seen = HashSet::new();
        if paths.iter().any(|path| !seen.insert(path)) { return false; }
        let order = self.folders.get_mut(folder).expect("enabled order exists");
        if order.files == paths { return false; }
        order.files = paths.to_vec();
        self.revision = self.revision.wrapping_add(1);
        order.revision = self.revision;
        true
    }
}

/// Reorder selected positions as one block, preserving their relative order.
/// Boundary is an insertion point in the ORIGINAL sequence, before removals.
/// This also supplies the exact preview order, so hover and commit agree.
pub fn reorder(count: usize, selected: &[usize], boundary: usize) -> Option<Vec<usize>> {
    if selected.is_empty() || boundary > count || selected.iter().any(|&i| i >= count) { return None; }
    let selected: HashSet<_> = selected.iter().copied().collect();
    let moved: Vec<_> = (0..count).filter(|i| selected.contains(i)).collect();
    let insertion = boundary - moved.iter().filter(|&&i| i < boundary).count();
    let mut result: Vec<_> = (0..count).filter(|i| !selected.contains(i)).collect();
    result.splice(insertion..insertion, moved);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_keeps_its_order_when_moved_down_or_up() {
        assert_eq!(reorder(6, &[3, 1], 6), Some(vec![0, 2, 4, 5, 1, 3]));
        assert_eq!(reorder(6, &[3, 1], 0), Some(vec![1, 3, 0, 2, 4, 5]));
        assert_eq!(reorder(5, &[1, 2], 2), Some(vec![0, 1, 2, 3, 4]));
    }

    #[test]
    fn invalid_boundaries_and_empty_selection_are_rejected() {
        assert_eq!(reorder(4, &[4], 2), None);
        assert_eq!(reorder(4, &[0], 5), None);
        assert_eq!(reorder(4, &[], 0), None);
    }

    fn entry(path: &Path, directory: bool) -> Entry {
        Entry { name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path: path.to_path_buf(), is_directory: directory, is_symlink: false,
            hidden: false, size: 0, modified: None }
    }

    #[test]
    fn changing_one_folder_does_not_invalidate_another() {
        let mut orders = ManualOrders::default();
        let a = Path::new("a");
        let b = Path::new("b");
        orders.set_enabled(a, true);
        orders.set_enabled(b, true);
        let revision = orders.revision_for(b);
        assert!(orders.save(a, &[a.join("file")]));
        assert_eq!(orders.revision_for(b), revision);
        assert_ne!(orders.revision_for(a), revision);
        orders.set_enabled(a, false);
        assert_eq!(orders.revision_for(b), revision);
    }

    #[test]
    fn manual_order_is_opt_in_persists_and_keeps_folders_first() {
        let folder = Path::new("folder");
        let a = folder.join("a");
        let b = folder.join("b");
        let c = folder.join("c");
        let entries = vec![entry(&folder.join("sub"), true), entry(&a, false),
            entry(&b, false), entry(&c, false)];
        let mut orders = ManualOrders::default();
        assert!(!orders.save(folder, &[b.clone(), a.clone()]));
        orders.set_enabled(folder, true);
        assert!(orders.save(folder, &[b.clone(), a.clone()]));
        assert!(!orders.save(folder, &[b.clone(), b.clone()]));
        assert!(!orders.save(folder, &[PathBuf::from("other/a")]));
        let mut restored = ManualOrders::decode(&orders.encode().unwrap());
        let mut indices = vec![0, 1, 2, 3];
        restored.apply(folder, &entries, &mut indices);
        assert_eq!(indices, vec![0, 2, 1, 3]);
        restored.set_enabled(folder, false);
        let mut indices = vec![0, 1, 2, 3];
        restored.apply(folder, &entries, &mut indices);
        assert_eq!(indices, vec![0, 1, 2, 3]);
    }
}
