//! The project card: the user's own page about a project — notes, facts,
//! links to folders — kept by Papka with the project's name. A section is
//! a title and free text; how a line reads decides how it is shown:
//!
//! - `Заказчик: ООО «Мост»` — a field, label and value in two columns;
//! - `Z:\Объекты\044\ИГИ` — a link that opens the folder in a new tab;
//! - `Модель: Z:\Объекты\044\model.txt` — a field whose value is a link;
//! - anything else — a line of text.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Sections Papka fills in itself; the user can only hide or show them.
pub const BUILT_IN: [(&str, &str); 4] = [
    ("inbox", "Входящие"),
    ("folders", "Папки проекта"),
    ("passport", "Паспорт объекта"),
    ("changes", "Последние изменения"),
];

const MAX_SECTIONS: usize = 40;
const MAX_TEXT: usize = 20_000;
const MAX_LABEL: usize = 40;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub title: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    #[serde(default)]
    pub sections: Vec<Section>,
    /// Keys of built-in sections the user has hidden.
    #[serde(default)]
    pub hidden: Vec<String>,
}

/// The settings key a project's card is stored under.
pub fn storage_key(project: &str) -> String {
    format!("project_card:{project}")
}

impl Card {
    /// What a project without a saved card starts with.
    pub fn starter() -> Card {
        Card { sections: vec![Section { title: "Заметки".into(), text: String::new() }], hidden: Vec::new() }
    }

    /// The saved card; a project that has none (or whose record cannot
    /// be read) gets the starter.
    pub fn load(saved: Option<&str>) -> Card {
        saved.and_then(|text| serde_json::from_str::<Card>(text).ok()).unwrap_or_else(Card::starter)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// Adds an empty section and returns its index; `None` at the limit.
    pub fn add(&mut self, title: &str) -> Option<usize> {
        if self.sections.len() >= MAX_SECTIONS { return None; }
        self.sections.push(Section { title: title.into(), text: String::new() });
        Some(self.sections.len() - 1)
    }

    /// Replaces a section's title and text, tidied: an empty title gets a
    /// name, the text loses trailing blank lines and anything past the limit.
    pub fn set(&mut self, index: usize, title: &str, text: &str) {
        let Some(section) = self.sections.get_mut(index) else { return };
        let title = title.trim();
        section.title = if title.is_empty() { "Без названия".into() } else { title.chars().take(120).collect() };
        section.text = text.replace("\r\n", "\n").trim_end().chars().take(MAX_TEXT).collect();
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.sections.len() { self.sections.remove(index); }
    }

    /// Moves a section one place up; the first one stays.
    pub fn move_up(&mut self, index: usize) {
        if index > 0 && index < self.sections.len() { self.sections.swap(index - 1, index); }
    }

    pub fn is_hidden(&self, key: &str) -> bool { self.hidden.iter().any(|hidden| hidden == key) }

    pub fn set_hidden(&mut self, key: &str, hidden: bool) {
        self.hidden.retain(|old| old != key);
        if hidden && BUILT_IN.iter().any(|(known, _)| *known == key) { self.hidden.push(key.into()); }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    Blank,
    Text(String),
    Link(PathBuf),
    Field { label: String, value: String, link: Option<PathBuf> },
}

/// `Z:\…`, `\\server\…` or `/…`: a place on a disk rather than a phrase.
pub fn as_path(text: &str) -> Option<PathBuf> {
    let text = text.trim().trim_matches('"');
    let bytes = text.as_bytes();
    let drive = bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/');
    let share = text.starts_with(r"\\") && text.len() > 3;
    let unix = cfg!(not(windows)) && text.starts_with('/') && text.len() > 1 && !text.starts_with("//");
    (drive || share || unix).then(|| PathBuf::from(text))
}

/// How one line of a section's text is shown.
pub fn read_line(line: &str) -> Line {
    let line = line.trim();
    if line.is_empty() { return Line::Blank; }
    if let Some(path) = as_path(line) { return Line::Link(path); }
    // "Label: value" — a short label, then a colon followed by a space
    // or the end; "12:30" and "https://…" are not fields.
    if let Some((label, rest)) = line.split_once(':') {
        let label = label.trim();
        let plain = !label.is_empty() && label.chars().count() <= MAX_LABEL && !label.contains(['\\', '/', '.', '?', '!']);
        if plain && (rest.is_empty() || rest.starts_with(' ')) {
            let value = rest.trim().to_string();
            return Line::Field { label: label.to_string(), link: as_path(&value), value };
        }
    }
    Line::Text(line.to_string())
}

pub fn read_lines(text: &str) -> Vec<Line> {
    text.lines().map(read_line).collect()
}

/// Every place the card's sections link to, once each.
pub fn links(card: &Card) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for line in card.sections.iter().flat_map(|section| read_lines(&section.text)) {
        let path = match line {
            Line::Link(path) | Line::Field { link: Some(path), .. } => path,
            _ => continue,
        };
        if !found.contains(&path) { found.push(path); }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_read_as_fields_links_or_text() {
        assert_eq!(read_line("  "), Line::Blank);
        assert_eq!(read_line("Заказчик: ООО «Мост»"), Line::Field { label: "Заказчик".into(), value: "ООО «Мост»".into(), link: None });
        assert_eq!(read_line("Шифр:"), Line::Field { label: "Шифр".into(), value: String::new(), link: None });
        assert_eq!(read_line(r"Z:\Объекты\044\ИГИ"), Line::Link(PathBuf::from(r"Z:\Объекты\044\ИГИ")));
        assert_eq!(read_line(r"\\server\share\Проект"), Line::Link(PathBuf::from(r"\\server\share\Проект")));
        assert_eq!(read_line(r#""C:/Work/a b""#), Line::Link(PathBuf::from("C:/Work/a b")));
        assert_eq!(read_line(r"Модель: Z:\Объекты\044\model.txt"),
            Line::Field { label: "Модель".into(), value: r"Z:\Объекты\044\model.txt".into(), link: Some(PathBuf::from(r"Z:\Объекты\044\model.txt")) });
        // Not fields: a time, an address, a sentence with a colon far in.
        for text in ["Созвон в 12:30", "https://example.com/a", "Позвонить. Срок: пятница", r"C:без косой"] {
            assert_eq!(read_line(text), Line::Text(text.into()), "{text}");
        }
        assert_eq!(read_lines("а\n\nб: в").len(), 3);
    }

    #[test]
    fn a_card_survives_saving_and_bad_records_give_the_starter() {
        let mut card = Card::starter();
        assert_eq!(card.sections[0].title, "Заметки");
        let index = card.add("Новый раздел").unwrap();
        card.set(index, "  Контакты  ", "ГИП: Иванов\r\nтел. 123\r\n\r\n");
        assert_eq!(card.sections[index], Section { title: "Контакты".into(), text: "ГИП: Иванов\nтел. 123".into() });
        card.set(index, "   ", "x");
        assert_eq!(card.sections[index].title, "Без названия");
        card.set_hidden("changes", true);
        card.set_hidden("nonsense", true);
        assert_eq!(card.hidden, ["changes"]);
        assert_eq!(Card::load(Some(&card.to_json())), card);
        assert_eq!(Card::load(None), Card::starter());
        assert_eq!(Card::load(Some("{ broken")), Card::starter());
        // A card the user emptied stays empty: it is not a missing one.
        assert_eq!(Card::load(Some(r#"{"sections":[]}"#)).sections, Vec::new());
        card.set_hidden("changes", false);
        assert!(!card.is_hidden("changes"));
    }

    #[test]
    fn links_are_collected_once_from_every_section() {
        let card = Card { sections: vec![
            Section { title: "а".into(), text: "Z:\\A\nМодель: Z:\\A\nтекст".into() },
            Section { title: "б".into(), text: "\\\\srv\\x".into() },
        ], hidden: Vec::new() };
        assert_eq!(links(&card), [PathBuf::from("Z:\\A"), PathBuf::from("\\\\srv\\x")]);
    }

    #[test]
    fn sections_move_up_and_go_away() {
        let mut card = Card { sections: ["а", "б", "в"].map(|title| Section { title: title.into(), text: String::new() }).into(), hidden: Vec::new() };
        card.move_up(2);
        card.move_up(0);
        card.move_up(9);
        assert_eq!(card.sections.iter().map(|section| section.title.as_str()).collect::<Vec<_>>(), ["а", "в", "б"]);
        card.remove(0);
        card.remove(9);
        assert_eq!(card.sections.len(), 2);
        for _ in 0..100 { card.add("x"); }
        assert_eq!(card.sections.len(), MAX_SECTIONS);
        assert_eq!(storage_key("Сити 2 Лот 3"), "project_card:Сити 2 Лот 3");
    }
}
