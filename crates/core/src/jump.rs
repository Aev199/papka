//! Quick jump to a folder (Ctrl+P): matching typed words against folder
//! paths, and collecting the subfolders of the places worth jumping to.

use std::collections::HashMap;
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

/// How often and how lately folders were opened, to put the ones in daily
/// use first in the jump list. Only folders stayed in count (the window
/// notes the pane's folder every couple of seconds), not those passed
/// through. Kept as text: `count<TAB>last-ms<TAB>path` per line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Visits {
    folders: HashMap<String, Visit>,
}

#[derive(Clone, Debug, PartialEq)]
struct Visit {
    path: PathBuf,
    count: u32,
    last_ms: i64,
}

/// Folders remembered at most; the least used go first.
const VISITS_KEPT: usize = 400;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

impl Visits {
    pub fn parse(text: &str) -> Self {
        let mut visits = Self::default();
        for line in text.lines() {
            let mut parts = line.splitn(3, '\t');
            let (Some(count), Some(last), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
            let (Ok(count), Ok(last_ms)) = (count.parse(), last.parse()) else { continue };
            let path = PathBuf::from(path);
            visits.folders.insert(crate::labels::key(&path), Visit { path, count, last_ms });
        }
        visits
    }

    pub fn to_text(&self) -> String {
        let mut rows: Vec<&Visit> = self.folders.values().collect();
        rows.sort_by(|a, b| b.last_ms.cmp(&a.last_ms));
        rows.iter().map(|visit| format!("{}\t{}\t{}", visit.count, visit.last_ms, visit.path.display())).collect::<Vec<_>>().join("\n")
    }

    /// One more stay in `path`.
    pub fn visit(&mut self, path: &Path, now_ms: i64) {
        let visit = self.folders.entry(crate::labels::key(path))
            .or_insert_with(|| Visit { path: path.to_path_buf(), count: 0, last_ms: now_ms });
        visit.count = visit.count.saturating_add(1);
        visit.last_ms = now_ms;
        if self.folders.len() > VISITS_KEPT {
            let weakest = self.folders.iter()
                .min_by(|a, b| frecency(a.1, now_ms).total_cmp(&frecency(b.1, now_ms)))
                .map(|(key, _)| key.clone());
            if let Some(key) = weakest { self.folders.remove(&key); }
        }
    }

    /// Added to a folder's match score: up to 100 for a folder opened
    /// often and lately; enough to order folders that match about equally,
    /// not to lift a poor match over a good one.
    pub fn bonus(&self, path: &Path, now_ms: i64) -> i64 {
        self.folders.get(&crate::labels::key(path))
            .map_or(0, |visit| ((frecency(visit, now_ms) as f64).ln_1p() * 25.).min(100.) as i64)
    }

    /// The most used folders, best first.
    pub fn top(&self, count: usize, now_ms: i64) -> Vec<PathBuf> {
        let mut rows: Vec<&Visit> = self.folders.values().collect();
        rows.sort_by(|a, b| frecency(b, now_ms).total_cmp(&frecency(a, now_ms)));
        rows.into_iter().take(count).map(|visit| visit.path.clone()).collect()
    }
}

/// Stays weighted by how long ago the last one was: a folder of today's
/// work counts four times, one untouched for a month half.
fn frecency(visit: &Visit, now_ms: i64) -> f32 {
    let age = now_ms - visit.last_ms;
    let weight = if age < DAY_MS { 4. } else if age < 7 * DAY_MS { 2. } else if age < 30 * DAY_MS { 1. } else { 0.5 };
    visit.count.min(100) as f32 * weight
}

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
    fn frequent_and_recent_folders_get_a_bonus() {
        let now = 100 * DAY_MS;
        let mut visits = Visits::default();
        let daily = Path::new("/p/Объект 12/Чертежи");
        let old = Path::new("/p/Архив/Чертежи");
        for _ in 0..10 { visits.visit(daily, now); }
        for _ in 0..10 { visits.visit(old, now - 60 * DAY_MS); }
        assert!(visits.bonus(daily, now) > visits.bonus(old, now));
        assert!(visits.bonus(daily, now) <= 100);
        assert_eq!(visits.bonus(Path::new("/p/never"), now), 0);
        // Of two equal matches the daily one wins; a far better match stays first.
        let tie = |path: &Path| score("черт", path).unwrap() + visits.bonus(path, now);
        assert!(tie(daily) > tie(old));
        assert_eq!(visits.top(1, now), vec![daily.to_path_buf()]);
        // Saved and read back.
        assert_eq!(Visits::parse(&visits.to_text()), visits);
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
