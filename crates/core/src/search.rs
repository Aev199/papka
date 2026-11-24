//! In-memory name index for a selected directory tree (no content indexing).
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Default)]
pub struct SearchIndex {
    root: PathBuf,
    files: Vec<PathBuf>,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct Preview {
    pub kind: &'static str,
    pub description: String,
}

impl SearchIndex {
    pub fn build(root: &Path, max_entries: usize) -> io::Result<Self> {
        if !root.is_dir() || max_entries == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Choose an existing directory and a nonzero limit"));
        }
        let mut result = Self { root: root.to_owned(), ..Default::default() };
        let policy = crate::scan_policy::ScanPolicy::for_root(root);
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_entry(|e| {
            !e.file_name().to_str().is_some_and(|s| {
                [".git", ".svn", "node_modules", "target"].contains(&s)
            }) && !(e.depth() > 0 && e.file_type().is_dir()
                && (crate::scan_policy::ScanPolicy::skips_name(&e.file_name().to_string_lossy()) || policy.skips(e.path())))
        }) {
            // A permission-denied subfolder must not discard every result
            // found so far. Keep the rest of the index usable.
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => { result.truncated = true; continue; }
            };
            if entry.depth() == 0 || entry.file_type().is_symlink() { continue; }
            if result.files.len() >= max_entries {
                result.truncated = true;
                break;
            }
            result.files.push(entry.path().to_owned());
        }
        Ok(result)
    }

    pub fn count(&self) -> usize { self.files.len() }
    pub fn root(&self) -> &Path { &self.root }

    pub fn query(&self, needle: &str, limit: usize) -> Vec<PathBuf> {
        if needle.trim().is_empty() || limit == 0 { return Vec::new(); }
        let mut scored: Vec<(i32, &PathBuf)> = self.files.iter()
            .filter_map(|p| subsequence_score(&p.file_name()?.to_string_lossy(), needle).map(|v| (v,p)))
            .collect();
        scored.sort_unstable_by(|a,b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        scored.into_iter().take(limit).map(|(_, path)| path.clone()).collect()
    }
}

/// Simple case-insensitive fuzzy match. Consecutive matches score higher.
pub fn subsequence_score(candidate: &str, needle: &str) -> Option<i32> {
    let candidate = candidate.to_lowercase();
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() { return None; }
    let mut positions = candidate.char_indices();
    let mut score = 0i32;
    let mut previous: Option<(usize, usize)> = None;
    for desired in needle.chars() {
        let (position, _) = positions.find(|(_, c)| *c == desired)?;
        score += 10;
        // Use the byte width of the PREVIOUS matched character. Russian
        // and other multi-byte UTF-8 filenames otherwise score incorrectly.
        if previous.is_some_and(|(at, bytes)| position == at + bytes) { score += 7; }
        if position == 0 { score += 4; }
        previous = Some((position, desired.len_utf8()));
    }
    Some(score - (candidate.chars().count() as i32 / 5))
}

/// Only bounded text preview is supported. Never loads a multi-GB CAD/PDF.
pub fn preview(path: &Path, max_bytes: u64) -> io::Result<Preview> {
    let meta = fs::metadata(path)?;
    if meta.is_dir() {
        return Ok(Preview { kind: "folder", description: "Папка".to_owned() });
    }
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    if ["pdf", "doc", "docx", "xls", "xlsx", "dwg", "dxf", "gts", "mec", "out", "zip", "7z"].contains(&extension.as_str()) {
        return Ok(Preview { kind: "metadata", description: format!("Файл {} · {} байт. Содержимое открывается в связанной программе.", extension.to_uppercase(), meta.len()) });
    }
    if ["png", "jpg", "jpeg", "webp", "gif", "bmp"].contains(&extension.as_str()) {
        return Ok(Preview { kind: "image", description: format!("Изображение · {} байт. Если миниатюра недоступна, откройте файл обычной командой.", meta.len()) });
    }
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(max_bytes.min(16 * 1024)).read_to_end(&mut bytes)?;
    if bytes.contains(&0) {
        return Ok(Preview { kind: "binary", description: format!("Двоичный файл · {} байт", meta.len()) });
    }
    // A byte-limited read may end inside a valid UTF-8 character. Do not
    // classify a Russian text file as binary just because of that boundary.
    let truncated = meta.len() > bytes.len() as u64;
    if truncated {
        if let Err(error) = std::str::from_utf8(&bytes) {
            if error.error_len().is_none() { bytes.truncate(error.valid_up_to()); }
        }
    }
    match String::from_utf8(bytes) {
        Ok(mut text) => {
            if truncated { text.push_str("\n… (показано начало файла)"); }
            Ok(Preview { kind: "text", description: text })
        }
        Err(_) => Ok(Preview { kind: "binary", description: format!("Двоичный файл или не UTF-8 · {} байт", meta.len()) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_does_not_misclassify_a_split_utf8_character() {
        let temp=tempfile::tempdir().unwrap();
        let path=temp.path().join("test.txt");
        fs::write(&path,"Привет").unwrap();
        let result=preview(&path,3).unwrap();
        assert_eq!(result.kind,"text");
        assert!(result.description.starts_with("П\n"));
        fs::write(&path,[0xff,0x80,0x41]).unwrap();
        assert_eq!(preview(&path,2).unwrap().kind,"binary");
    }

    #[test]
    fn fuzzy_and_bounded_preview() {
        assert!(subsequence_score("foundation_model.xlsx", "fmdl").is_some());
        assert!(subsequence_score("report.txt", "zzz").is_none());
        assert!(subsequence_score("Расчётная_модель.xlsx", "расмод").is_some());
        assert!(subsequence_score("Фундамент.txt", "фун").is_some());
        assert!(subsequence_score("модель.txt", "ьлем").is_none());
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("foundation_model.txt");
        fs::write(&a, "original document").unwrap();
        let idx = SearchIndex::build(temp.path(), 100).unwrap();
        assert_eq!(idx.query("fmdl", 10), vec![a.clone()]);
        let prev = preview(&a, 8).unwrap();
        assert_eq!(prev.kind, "text");
        assert!(prev.description.contains("показано начало"));
    }
}
