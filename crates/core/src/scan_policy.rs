//! Folders background walks (search, the file index and its updates) do not
//! go into unless asked to start there: Windows itself, ProgramData, temp
//! folders and Papka's own profile. They are large, change all the time and
//! hold nothing a user looks for; an explicit listing still shows them.
use std::path::{Path, PathBuf};

/// Folder names skipped wherever they occur, whatever the root.
pub const SKIPPED_NAMES: &[&str] = &[
    "$recycle.bin", "system volume information", "$windows.~bt", "$windows.~ws",
    "config.msi", ".git", ".svn", ".hg",
];

/// Whole subtrees left out of one walk.
#[derive(Clone, Debug, Default)]
pub struct ScanPolicy {
    roots: Vec<String>,
}

impl ScanPolicy {
    /// The rules for a walk from `root`: the system folders, except those
    /// the walk starts in or under (searching C:\Windows looks there).
    pub fn for_root(root: &Path) -> Self {
        Self::with_roots(root, &system_folders())
    }

    pub fn with_roots(root: &Path, folders: &[PathBuf]) -> Self {
        let start = key(root);
        let roots = folders.iter().map(|folder| key(folder))
            .filter(|folder| !folder.is_empty() && !within(&start, folder))
            .collect();
        Self { roots }
    }

    /// Whether the walk leaves `path` (and all below it) out.
    pub fn skips(&self, path: &Path) -> bool {
        let path = key(path);
        self.roots.iter().any(|root| within(&path, root))
    }

    /// Whether a folder with this name is never gone into.
    pub fn skips_name(name: &str) -> bool {
        SKIPPED_NAMES.contains(&name.to_lowercase().as_str())
    }
}

/// `path` is `folder` or lies below it.
fn within(path: &str, folder: &str) -> bool {
    path == folder || path.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}

/// A comparable spelling: no `\\?\`, forward slashes, no trailing slash,
/// lower case on Windows.
fn key(path: &Path) -> String {
    let path = crate::path_utils::normalize_extended_path(path);
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.trim_end_matches('/').to_string();
    if cfg!(windows) { text.to_lowercase() } else { text }
}

fn system_folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if cfg!(windows) {
        for name in ["WINDIR", "ProgramData", "TEMP"] {
            if let Some(value) = std::env::var_os(name) { folders.push(PathBuf::from(value)); }
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            folders.push(PathBuf::from(local).join("Temp"));
        }
    }
    folders.push(crate::path_utils::app_data_dir());
    folders
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_folders_are_skipped_only_from_above() {
        let windows = PathBuf::from("/c/Windows");
        let data = PathBuf::from("/c/ProgramData");
        let from_drive = ScanPolicy::with_roots(Path::new("/c"), &[windows.clone(), data.clone()]);
        assert!(from_drive.skips(Path::new("/c/Windows")));
        assert!(from_drive.skips(Path::new("/c/Windows/System32/a.dll")));
        assert!(from_drive.skips(Path::new("/c/ProgramData/vendor")));
        assert!(!from_drive.skips(Path::new("/c/WindowsApps")));
        assert!(!from_drive.skips(Path::new("/c/Users/me/Windows")));
        // A search that starts inside Windows looks there.
        let inside = ScanPolicy::with_roots(Path::new("/c/Windows/Fonts"), &[windows, data]);
        assert!(!inside.skips(Path::new("/c/Windows/Fonts/a.ttf")));
        assert!(inside.skips(Path::new("/c/ProgramData/x")));
    }

    #[test]
    fn names_are_skipped_in_any_case() {
        assert!(ScanPolicy::skips_name("$RECYCLE.BIN"));
        assert!(ScanPolicy::skips_name("System Volume Information"));
        assert!(!ScanPolicy::skips_name("Windows"));
    }
}
