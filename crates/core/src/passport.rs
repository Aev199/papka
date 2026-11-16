//! The object passport: an `object.json` kept in an object's folder by the
//! calculation programs (geology base, models and their units, iterations,
//! warnings, a journal of runs). Papka only reads it, for the project card.
//!
//! The file belongs to those programs and may grow new blocks, so nothing
//! here depends on its exact shape: known blocks (name, date, warnings,
//! journal) go to their places, everything else is shown as it is, block
//! by block. See `docs/PROJECT_CARD.md`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub const FILE_NAME: &str = "object.json";
/// The read-only page the same programs write beside the passport.
pub const PAGE_NAME: &str = "object.html";
/// How far above a project folder the passport is looked for.
const LEVELS_UP: usize = 5;
/// A passport is a page of text; anything larger is something else.
const MAX_BYTES: u64 = 2 << 20;
const MAX_ROWS: usize = 60;
const MAX_TEXT: usize = 400;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Passport {
    pub title: Option<String>,
    /// "05.11.2026 · SoilStiffness 1.8.2", as written by the programs.
    pub updated: Option<String>,
    pub warnings: Vec<String>,
    pub sections: Vec<Section>,
    /// Runs of the programs, in the file's order.
    pub journal: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    /// Label (may be empty) and value.
    pub rows: Vec<(String, String)>,
}

/// The passport of the object the folders belong to: the first
/// `object.json` in one of them or up to five levels above it.
pub fn find(folders: &[PathBuf]) -> Option<PathBuf> {
    folders.iter().find_map(|folder| {
        folder.ancestors().take(LEVELS_UP + 1).map(|level| level.join(FILE_NAME)).find(|file| file.is_file())
    })
}

/// The `object.html` beside a passport, if the programs wrote one.
pub fn page_of(passport: &Path) -> Option<PathBuf> {
    Some(passport.with_file_name(PAGE_NAME)).filter(|page| page.is_file())
}

pub fn read(path: &Path) -> io::Result<Passport> {
    if fs::metadata(path)?.len() > MAX_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "файл паспорта слишком велик"));
    }
    let bytes = fs::read(path)?;
    // Programs on Windows often write a byte order mark first.
    let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes));
    parse(&text)
}

pub fn parse(text: &str) -> io::Result<Passport> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("паспорт не читается как JSON: {error}")))?;
    let Value::Object(object) = value else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "паспорт должен быть объектом JSON"));
    };
    let mut passport = Passport::default();
    let mut general = Section { title: "Объект".into(), rows: Vec::new() };
    let mut updated: [Option<String>; 2] = [None, None];
    for (key, value) in &object {
        match role(key) {
            Role::Title => passport.title = scalar(value).filter(|text| !text.is_empty()),
            Role::Updated => updated[0] = scalar(value),
            Role::UpdatedBy => updated[1] = scalar(value),
            Role::Warnings => passport.warnings = lines(value),
            Role::Journal => passport.journal = lines(value),
            Role::Skip => {}
            Role::Block => match value {
                Value::Object(fields) => {
                    let rows: Vec<_> = fields.iter().filter_map(|(key, value)| Some((label(key), flat(value)?))).take(MAX_ROWS).collect();
                    if !rows.is_empty() { passport.sections.push(Section { title: label(key), rows }); }
                }
                Value::Array(items) => {
                    let rows: Vec<_> = items.iter().filter_map(|item| Some((item_label(item), item_text(item)?))).take(MAX_ROWS).collect();
                    if !rows.is_empty() { passport.sections.push(Section { title: label(key), rows }); }
                }
                other => if let Some(text) = flat(other) { general.rows.push((label(key), text)); },
            },
        }
    }
    if !general.rows.is_empty() { passport.sections.insert(0, general); }
    let updated: Vec<String> = updated.into_iter().flatten().filter(|text| !text.is_empty()).collect();
    if !updated.is_empty() { passport.updated = Some(updated.join(" · ")); }
    Ok(passport)
}

enum Role { Title, Updated, UpdatedBy, Warnings, Journal, Skip, Block }

fn role(key: &str) -> Role {
    match key.to_lowercase().as_str() {
        "name" | "title" | "object" | "название" | "объект" => Role::Title,
        "updated" | "updated_at" | "modified" | "обновлено" => Role::Updated,
        "updated_by" | "program" | "writer" | "программа" => Role::UpdatedBy,
        "warnings" | "alerts" | "предупреждения" => Role::Warnings,
        "journal" | "log" | "history" | "журнал" => Role::Journal,
        "schema" | "version" | "$schema" | "format" => Role::Skip,
        _ => Role::Block,
    }
}

/// Russian names of the blocks the programs are known to write; any
/// other key is shown as written, with spaces for underscores.
fn label(key: &str) -> String {
    let known = match key.to_lowercase().as_str() {
        "geology" => "Геология",
        "models" => "Модели",
        "iterations" => "Итерации",
        "units" => "Единицы",
        "file" | "path" => "Файл",
        "revision" => "Ревизия",
        "coordinates" | "crs" => "Система координат",
        "loads" => "Нагрузки",
        "piles" => "Сваи",
        "settlement" => "Осадка",
        "boreholes" => "Скважины",
        "layers" | "ige" => "ИГЭ",
        "modified" | "changed" => "Изменён",
        "checksum" | "sha256" | "hash" => "Контрольная сумма",
        "address" => "Адрес",
        "customer" => "Заказчик",
        "stage" => "Стадия",
        "code" | "cipher" => "Шифр",
        "comment" | "note" | "notes" => "Примечание",
        _ => return key.replace('_', " "),
    };
    known.to_string()
}

fn clip(text: String) -> String {
    if text.chars().count() <= MAX_TEXT { return text; }
    format!("{}…", text.chars().take(MAX_TEXT).collect::<String>().trim_end())
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(clip(text.trim().to_string())),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "да" } else { "нет" }.to_string()),
        _ => None,
    }
}

/// Any value on one line; `None` for nothing to show.
fn flat(value: &Value) -> Option<String> {
    let text = match value {
        Value::Null => return None,
        Value::Array(items) => items.iter().filter_map(flat).collect::<Vec<_>>().join(", "),
        Value::Object(fields) => fields.iter().filter_map(|(key, value)| Some(format!("{}: {}", label(key), flat(value)?)))
            .collect::<Vec<_>>().join(" · "),
        other => scalar(other)?,
    };
    (!text.is_empty()).then(|| clip(text))
}

const NAMING: [&str; 6] = ["name", "kind", "type", "program", "id", "no"];

/// What a list item is called: its name, kind or number, if it has one.
fn item_label(item: &Value) -> String {
    let Value::Object(fields) = item else { return String::new() };
    NAMING.iter().find_map(|key| fields.get(*key).and_then(scalar)).unwrap_or_default()
}

/// A list item on one line, without the field used as its label.
fn item_text(item: &Value) -> Option<String> {
    let Value::Object(fields) = item else { return flat(item) };
    let named = NAMING.iter().find(|key| fields.get(**key).and_then(scalar).is_some());
    let text = fields.iter().filter(|(key, _)| Some(&key.as_str()) != named)
        .filter_map(|(key, value)| Some(match key.as_str() {
            // Self-explanatory values read better without their key.
            "file" | "path" | "text" | "message" | "date" | "units" => flat(value)?,
            _ => format!("{}: {}", label(key), flat(value)?),
        }))
        .collect::<Vec<_>>().join(" · ");
    (!text.is_empty()).then(|| clip(text))
}

/// Warnings and journal entries: one line each, whatever their shape.
fn lines(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items.iter().filter_map(|item| match item {
            Value::Object(fields) => {
                let text = fields.values().filter_map(flat).collect::<Vec<_>>().join(" · ");
                (!text.is_empty()).then(|| clip(text))
            }
            other => flat(other),
        }).take(MAX_ROWS).collect(),
        other => flat(other).into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "schema": 1,
        "name": "ЖК Остров",
        "updated": "05.11.2026",
        "updated_by": "SoilStiffness 1.8.2",
        "code": "044",
        "geology": {"file": "ostrov.sqlite", "revision": "14.10", "ige": 9, "boreholes": 23},
        "models": [
            {"kind": "ЛИРА", "file": "lira/ostrov.txt", "units": "м, кН", "modified": "03.11"},
            {"kind": "MIDAS", "file": "midas/ostrov.mec", "units": "мм, Н", "shift": [1250, -340], "angle": 12.5}
        ],
        "iterations": [{"no": 2, "text": "C1 ±6 % к №1, осадка 109 → 112 мм"}],
        "warnings": ["C1 iter2 считан по ЛИРА от 01.11, модель изменена 03.11 — устарело",
                     {"program": "Femgeo", "text": "расхождение собственного веса с ЛИРА 4,7 %"}],
        "journal": [{"date": "05.11", "program": "SoilStiffness", "text": "MIDAS → ЛИРА 2024, 3 предупреждения"},
                    "01.11 Femgeo подбор поля, 91 свая"],
        "empty": {},
        "nothing": null
    }"#;

    #[test]
    fn known_blocks_go_to_their_places_and_the_rest_is_shown_as_it_is() {
        let passport = parse(SAMPLE).unwrap();
        assert_eq!(passport.title.as_deref(), Some("ЖК Остров"));
        assert_eq!(passport.updated.as_deref(), Some("05.11.2026 · SoilStiffness 1.8.2"));
        assert_eq!(passport.warnings, [
            "C1 iter2 считан по ЛИРА от 01.11, модель изменена 03.11 — устарело",
            "Femgeo · расхождение собственного веса с ЛИРА 4,7 %"]);
        assert_eq!(passport.journal, ["05.11 · SoilStiffness · MIDAS → ЛИРА 2024, 3 предупреждения", "01.11 Femgeo подбор поля, 91 свая"]);
        let titles: Vec<&str> = passport.sections.iter().map(|section| section.title.as_str()).collect();
        assert_eq!(titles, ["Объект", "Геология", "Модели", "Итерации"], "the file's order; empty blocks are left out");
        assert_eq!(passport.sections[0].rows, [("Шифр".to_string(), "044".to_string())]);
        assert_eq!(passport.sections[1].rows[0], ("Файл".to_string(), "ostrov.sqlite".to_string()));
        assert_eq!(passport.sections[1].rows[2], ("ИГЭ".to_string(), "9".to_string()));
        let models = &passport.sections[2].rows;
        assert_eq!(models[0], ("ЛИРА".to_string(), "lira/ostrov.txt · м, кН · Изменён: 03.11".to_string()));
        assert_eq!(models[1], ("MIDAS".to_string(), "midas/ostrov.mec · мм, Н · shift: 1250, -340 · angle: 12.5".to_string()));
        assert_eq!(passport.sections[3].rows, [("2".to_string(), "C1 ±6 % к №1, осадка 109 → 112 мм".to_string())]);
    }

    #[test]
    fn a_file_that_is_not_a_passport_is_refused_with_a_reason() {
        assert!(parse("[1, 2]").unwrap_err().to_string().contains("объектом"));
        assert!(parse("{ not json").unwrap_err().to_string().contains("JSON"));
        assert_eq!(parse("{}").unwrap(), Passport::default());
        let long = format!(r#"{{"note": "{}"}}"#, "я".repeat(1000));
        assert!(parse(&long).unwrap().sections[0].rows[0].1.chars().count() <= MAX_TEXT + 1);
    }

    #[test]
    fn the_passport_is_found_in_the_folder_or_a_few_levels_above() {
        let dir = tempfile::tempdir().unwrap();
        let object = dir.path().join("044 Сити-2");
        let deep = object.join("2.13 ЭАО/01. Исходные данные/ИГИ");
        fs::create_dir_all(&deep).unwrap();
        let elsewhere = dir.path().join("other");
        fs::create_dir(&elsewhere).unwrap();
        assert_eq!(find(&[deep.clone()]), None);
        fs::write(object.join(FILE_NAME), "\u{feff}{\"name\": \"Сити-2\"}").unwrap();
        assert_eq!(find(&[elsewhere.clone(), deep.clone()]), Some(object.join(FILE_NAME)));
        assert_eq!(find(&[elsewhere]), None);
        let passport = read(&object.join(FILE_NAME)).unwrap();
        assert_eq!(passport.title.as_deref(), Some("Сити-2"), "a byte order mark is not part of the text");
        assert_eq!(page_of(&object.join(FILE_NAME)), None);
        fs::write(object.join(PAGE_NAME), "<html>").unwrap();
        assert_eq!(page_of(&object.join(FILE_NAME)), Some(object.join(PAGE_NAME)));
        // Too far above: the search stops after five levels.
        let far = deep.join("a/b/c/d");
        fs::create_dir_all(&far).unwrap();
        assert_eq!(find(&[far]), None);
    }
}
