//! Quick jump to a folder (Ctrl+P): matching typed words against folder
//! paths, and collecting the subfolders of the places worth jumping to.

use std::path::{Path, PathBuf};

/// How well `query` names `path`, higher is better; None when it does not.
/// Every word must be found: in the folder's own name best, anywhere in the
/// path next, or with its letters in order in the name. Words typed in the
/// English layout also match Russian names.
pub fn score(query: &str, path: &Path) -> Option<i64> {
    let text = path.to_string_lossy().to_lowercase();
    let name = path.file_name().map_or_else(|| text.clone(), |name| name.to_string_lossy().to_lowercase());
    let depth = path.components().count() as i64;
    let mut total = -depth;
    for word in query.split_whitespace().map(str::to_lowercase) {
        let layout = crate::interface::to_russian_layout(&word);
        let latin = transliterate(&word);
        let best = [word.as_str(), layout.as_str(), latin.as_str()].into_iter()
            .filter_map(|word| word_score(word, &name, &text))
            .max()?;
        total += best;
    }
    Some(total)
}

/// Russian letters for a word typed in Latin ones: "proekt" is "проект".
fn transliterate(word: &str) -> String {
    const PAIRS: [(&str, &str); 12] = [
        ("shch", "щ"), ("sch", "щ"), ("zh", "ж"), ("kh", "х"), ("ch", "ч"), ("sh", "ш"),
        ("ts", "ц"), ("yu", "ю"), ("ya", "я"), ("yo", "ё"), ("ye", "е"), ("iy", "ий"),
    ];
    const SINGLE: [(char, char); 26] = [
        ('a', 'а'), ('b', 'б'), ('c', 'ц'), ('d', 'д'), ('e', 'е'), ('f', 'ф'), ('g', 'г'), ('h', 'х'),
        ('i', 'и'), ('j', 'й'), ('k', 'к'), ('l', 'л'), ('m', 'м'), ('n', 'н'), ('o', 'о'), ('p', 'п'),
        ('q', 'к'), ('r', 'р'), ('s', 'с'), ('t', 'т'), ('u', 'у'), ('v', 'в'), ('w', 'в'), ('x', 'х'),
        ('y', 'ы'), ('z', 'з'),
    ];
    let mut out = String::new();
    let mut rest = word;
    'next: while let Some(first) = rest.chars().next() {
        for (latin, russian) in PAIRS {
            if let Some(after) = rest.strip_prefix(latin) {
                out.push_str(russian);
                rest = after;
                continue 'next;
            }
        }
        out.push(SINGLE.iter().find(|(latin, _)| *latin == first).map_or(first, |(_, russian)| *russian));
        rest = &rest[first.len_utf8()..];
    }
    out
}

fn word_score(word: &str, name: &str, text: &str) -> Option<i64> {
    if name == word { return Some(300); }
    if name.starts_with(word) { return Some(200 - name.chars().count() as i64 / 4); }
    if name.contains(word) { return Some(120 - name.chars().count() as i64 / 4); }
    if text.contains(word) { return Some(50); }
    letters_score(word, name)
}

/// Letters of `word` found in order in `name`, scored by where they fall:
/// on the starts of words ("жкс" in «ЖК Северный») and in runs score more
/// than letters scattered through the middle. Between 1 and 110, below a
/// name that contains the word whole.
fn letters_score(word: &str, name: &str) -> Option<i64> {
    const START: i64 = 16;
    const RUN: i64 = 8;
    const GAP: i64 = 3;
    let wanted: Vec<char> = word.chars().collect();
    let letters: Vec<char> = name.chars().collect();
    if wanted.is_empty() || wanted.len() > letters.len() { return None; }
    let starts: Vec<bool> = (0..letters.len()).map(|j| j == 0 || {
        let before = letters[j - 1];
        !before.is_alphanumeric() || (before.is_alphabetic() != letters[j].is_alphabetic())
    }).collect();
    // best[j]: the best score with the current letter of `word` at `name[j]`.
    let none = i64::MIN / 2;
    let mut best: Vec<i64> = (0..letters.len())
        .map(|j| if letters[j] == wanted[0] { if starts[j] { START } else { 0 } } else { none })
        .collect();
    for &letter in &wanted[1..] {
        let mut next = vec![none; letters.len()];
        let mut earlier = none; // the best of best[k] for k < j - 1
        for j in 1..letters.len() {
            if j >= 2 { earlier = earlier.max(best[j - 2]); }
            if letters[j] != letter { continue; }
            let bonus = if starts[j] { START } else { 0 };
            let run = if best[j - 1] > none { best[j - 1] + RUN } else { none };
            let from = run.max(earlier - GAP);
            if from > none { next[j] = from + bonus; }
        }
        best = next;
    }
    let top = *best.iter().max()?;
    (top > none).then(|| (20 + top).clamp(1, 110))
}

/// Subfolders of `roots` down to `depth` levels, at most `limit` of them,
/// skipping hidden and system folders. Unreadable folders are passed over.
pub fn subfolders(roots: &[PathBuf], depth: usize, limit: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut level: Vec<PathBuf> = roots.to_vec();
    for _ in 0..depth {
        let mut next = Vec::new();
        for folder in &level {
            let Ok(entries) = std::fs::read_dir(folder) else { continue };
            for entry in entries.flatten() {
                if found.len() >= limit { return found; }
                let Ok(kind) = entry.file_type() else { continue };
                if !kind.is_dir() { continue; }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name.starts_with('$') || hidden(&entry) { continue; }
                found.push(entry.path());
                next.push(entry.path());
            }
        }
        level = next;
    }
    found
}

#[cfg(windows)]
fn hidden(entry: &std::fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN_OR_SYSTEM: u32 = 0x2 | 0x4;
    entry.metadata().is_ok_and(|meta| meta.file_attributes() & HIDDEN_OR_SYSTEM != 0)
}

#[cfg(not(windows))]
fn hidden(_entry: &std::fs::DirEntry) -> bool { false }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_name_counts_most() {
        let project = Path::new("/p/Объекты/ЖК Северный/Расчёты");
        let other = Path::new("/p/Расчёты старые/архив");
        assert!(score("расч", project).unwrap() > score("расч", other).unwrap());
        assert!(score("северный расч", project).is_some());
        assert!(score("северный южный", project).is_none());
        // Letters in order: "рсч" finds "Расчёты".
        assert!(score("рсч", project).is_some());
        // Typed in the English layout, or in Latin letters.
        assert!(score("hfcx", project).is_some());
        assert!(score("severn", project).is_some());
        assert_eq!(transliterate("proekt"), "проект");
        assert_eq!(transliterate("shchit"), "щит");
        assert_eq!(score("", project), Some(-(project.components().count() as i64)));
    }

    #[test]
    fn letters_on_word_starts_beat_scattered_ones() {
        // «ЖК Северный»: "жкс" takes the word starts, "жкй" ends inside a word.
        let starts = letters_score("жкс", "жк северный").unwrap();
        let scattered = letters_score("жкй", "жк северный").unwrap();
        assert!(starts > scattered, "{starts} {scattered}");
        // Initials of a long name rank it above a name the letters just occur in.
        let initials = score("пзр", Path::new("/p/Пояснительная записка разделы")).unwrap();
        let loose = score("пзр", Path::new("/p/Прозрачность")).unwrap();
        assert!(initials > loose, "{initials} {loose}");
        // Still below a name that holds the word whole.
        assert!(letters_score("сев", "жк северный").unwrap() < 120);
        assert_eq!(letters_score("яя", "я"), None);
        assert_eq!(letters_score("ба", "аб"), None);
    }

    #[test]
    fn subfolders_stop_at_depth_and_limit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b/c")).unwrap();
        std::fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        std::fs::write(dir.path().join("a/file.txt"), b"x").unwrap();
        let roots = [dir.path().to_path_buf()];
        let two = subfolders(&roots, 2, 100);
        assert_eq!(two.len(), 2);
        assert!(two.contains(&dir.path().join("a/b")));
        assert_eq!(subfolders(&roots, 3, 1).len(), 1);
    }
}
