//! The object passport: an `object.json` kept in an object's folder by the
//! calculation programs (geology base, models and their units, iterations,
//! warnings, a journal of runs). Papka shows it in the project card and
//! changes a block only when the user edits it.
//!
//! The file belongs to those programs and may grow new blocks, so nothing
//! here depends on its exact shape: known blocks (name, date, warnings,
//! journal) go to their places, everything else is shown as it is, block
//! by block. See `docs/PROJECT_CARD.md`.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Map, Value};

pub const FILE_NAME: &str = "object.json";
/// The read-only page the same programs write beside the passport.
pub const PAGE_NAME: &str = "object.html";
/// How far above a project folder the passport is looked for.
const LEVELS_UP: usize = 5;
/// A passport is a page of text; anything larger is something else.
const MAX_BYTES: u64 = 2 << 20;

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
    /// The block's key in the file; empty for the loose values on top.
    pub key: String,
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
    read_raw(path).map(|(raw, _)| from_raw(&raw))
}

pub fn parse(text: &str) -> io::Result<Passport> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("паспорт не читается как JSON: {error}")))?;
    let Value::Object(object) = value else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "паспорт должен быть объектом JSON"));
    };
    Ok(from_raw(&object))
}

/// The passport as shown, from its raw file.
pub fn from_raw(object: &Map<String, Value>) -> Passport {
    let mut passport = Passport::default();
    let mut general = Section { key: String::new(), title: "Объект".into(), rows: Vec::new() };
    let mut updated: [Option<String>; 2] = [None, None];
    for (key, value) in object {
        match role(key) {
            Role::Title => passport.title = scalar(value).filter(|text| !text.is_empty()),
            Role::Updated => updated[0] = scalar(value),
            Role::UpdatedBy => updated[1] = scalar(value),
            Role::Warnings => passport.warnings = lines(value),
            Role::Journal => passport.journal = lines(value),
            Role::Skip => {}
            Role::Block => match value {
                Value::Object(fields) => {
                    let rows: Vec<_> = fields.iter().filter_map(|(key, value)| Some((label(key), flat(value)?))).collect();
                    passport.sections.push(Section { key: key.clone(), title: label(key), rows });
                }
                Value::Array(items) => {
                    let rows: Vec<_> = items.iter().filter_map(|item| Some((item_label(item), item_text(item)?))).collect();
                    passport.sections.push(Section { key: key.clone(), title: label(key), rows });
                }
                other => if let Some(text) = flat(other) { general.rows.push((label(key), text)); },
            },
        }
    }
    if !general.rows.is_empty() { passport.sections.insert(0, general); }
    let updated: Vec<String> = updated.into_iter().flatten().filter(|text| !text.is_empty()).collect();
    if !updated.is_empty() { passport.updated = Some(updated.join(" · ")); }
    passport
}

enum Role { Title, Updated, UpdatedBy, Warnings, Journal, Skip, Block }

fn role(key: &str) -> Role {
    match key.to_lowercase().as_str() {
        // Not "объект": that is the block of the object's own values.
        "name" | "title" | "object" | "название" => Role::Title,
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

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.trim().to_string()),
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
    (!text.is_empty()).then_some(text)
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
    // A named field still empty ("Шифр: ") stays, to be filled in.
    (!text.is_empty() || named.is_some()).then_some(text)
}

/// Warnings and journal entries: one line each, whatever their shape.
fn lines(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items.iter().filter_map(|item| match item {
            Value::Object(fields) => {
                let text = fields.values().filter_map(flat).collect::<Vec<_>>().join(" · ");
                (!text.is_empty()).then_some(text)
            }
            other => flat(other),
        }).collect(),
        other => flat(other).into_iter().collect(),
    }
}

// ---------------------------------------------------------------- editing
//
// No calculation program keeps the passport yet, so Papka does: it creates
// `object.json` in the object's folder and edits it block by block. A block
// Papka writes is a list — `{"name": label, "text": value}` for a
// «Метка: значение» line, a plain string for other lines — which the reader
// above shows exactly as typed. Blocks nobody edited keep whatever another
// program wrote. The file is replaced whole, and not if someone changed it
// since it was read (a shared drive with colleagues).

pub type Raw = Map<String, Value>;

/// Today as the passport writes dates: «17.11.2026».
pub fn today() -> String {
    chrono::Local::now().format("%d.%m.%Y").to_string()
}

/// The key the loose values on top are kept under once edited.
pub const GENERAL_KEY: &str = "Объект";

/// The raw passport and when it was last written, for `write`.
pub fn read_raw(path: &Path) -> io::Result<(Raw, Option<SystemTime>)> {
    let file = fs::File::open(path)?;
    let modified = file.metadata()?.modified().ok();
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "файл паспорта слишком велик (не более 2 МиБ)"));
    }
    let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes));
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(raw)) => Ok((raw, modified)),
        Ok(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "паспорт должен быть объектом JSON")),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidData, format!("паспорт не читается как JSON: {error}"))),
    }
}

/// A new passport: the object's name and empty blocks to fill.
pub fn template(name: &str) -> Raw {
    let field = |label: &str| serde_json::json!({ "name": label, "text": "" });
    let mut raw = Raw::new();
    raw.insert("format".into(), Value::from(1));
    raw.insert("name".into(), Value::String(name.into()));
    raw.insert(GENERAL_KEY.into(), Value::Array(["Шифр", "Адрес", "Заказчик", "Стадия", "ГИП"].iter().map(|l| field(l)).collect()));
    raw.insert("Геология".into(), Value::Array(Vec::new()));
    raw.insert("Модели".into(), Value::Array(Vec::new()));
    raw.insert("warnings".into(), Value::Array(Vec::new()));
    raw.insert("journal".into(), Value::Array(Vec::new()));
    raw
}

/// A block's rows as text to edit: «Метка: значение» or the value alone.
pub fn block_text(section: &Section) -> String {
    section.rows.iter()
        .map(|(label, value)| if label.is_empty() { value.clone() } else { format!("{label}: {value}") })
        .collect::<Vec<_>>().join("\n")
}

/// The text typed for a block as what is written: a list of fields and lines.
fn block_value(text: &str) -> Value {
    Value::Array(text.lines().filter_map(|line| {
        let line = line.trim();
        if line.is_empty() { return None; }
        // A short label before ": " (or a bare "Метка:") is a field.
        match line.split_once(':') {
            Some((label, rest)) if !label.trim().is_empty() && label.chars().count() <= 40 && !label.contains(['\\', '/'])
                && (rest.is_empty() || rest.starts_with(' ')) =>
                Some(serde_json::json!({ "name": label.trim(), "text": rest.trim() })),
            _ => Some(Value::String(line.to_string())),
        }
    }).collect())
}

fn index_of(raw: &Raw, key: &str) -> Option<usize> {
    raw.keys().position(|existing| existing == key)
}

fn key_of(raw: &Raw, wanted: fn(&str) -> bool) -> Option<String> {
    raw.keys().find(|key| wanted(key)).cloned()
}

fn is_loose(key: &str, value: &Value) -> bool {
    matches!(role(key), Role::Block) && !matches!(value, Value::Object(_) | Value::Array(_) | Value::Null)
}

/// Where a new block goes: after the last block, before the warnings and
/// the journal if they come last.
fn block_end(raw: &Raw) -> usize {
    raw.iter().rposition(|(key, _)| matches!(role(key), Role::Block)).map_or(raw.len(), |index| index + 1)
}

/// Writes block `old_key` (`""`: the loose values on top) with `title` and
/// the typed `text`, in its place. A new title becomes the block's key; a
/// block that is new goes after the others.
pub fn set_block(raw: &mut Raw, old_key: Option<&str>, title: &str, text: &str) {
    let title = title.trim();
    // Merely opening a machine-written block must preserve its schema,
    // nulls, numeric types and fields that the text view does not name.
    if let Some(key) = old_key {
        if from_raw(raw).sections.iter().any(|section| section.key == key
            && section.title == title && block_text(section) == text) { return; }
    }
    let mut position = None;
    match old_key {
        Some("") => {
            // The loose values give way to one «Объект» block where the first was.
            let loose: Vec<String> = raw.iter().filter(|(key, value)| is_loose(key, value)).map(|(key, _)| key.clone()).collect();
            position = loose.first().and_then(|first| index_of(raw, first));
            for key in loose { raw.shift_remove(&key); }
        }
        Some(key) => {
            position = index_of(raw, key);
            raw.shift_remove(key);
        }
        None => {}
    }
    let keep = old_key.filter(|key| !key.is_empty() && label(key) == title).map(str::to_string);
    let mut key = keep.unwrap_or_else(|| if title.is_empty() { "Без названия".into() } else { title.to_string() });
    // Not on top of another block, nor of a field with a role.
    let base = key.clone();
    let mut n = 2;
    while raw.contains_key(&key) || !matches!(role(&key), Role::Block) {
        key = format!("{base} {n}");
        n += 1;
    }
    let at = position.unwrap_or_else(|| block_end(raw)).min(raw.len());
    raw.shift_insert(at, key, block_value(text));
}

pub fn remove_block(raw: &mut Raw, key: &str) {
    if key.is_empty() {
        let loose: Vec<String> = raw.iter().filter(|(key, value)| is_loose(key, value)).map(|(key, _)| key.clone()).collect();
        for key in loose { raw.shift_remove(&key); }
    } else {
        raw.shift_remove(key);
    }
}

/// The warnings (one a line), under whatever key the file uses for them.
pub fn set_warnings(raw: &mut Raw, text: &str) {
    if from_raw(raw).warnings.join("\n") == text { return; }
    let key = key_of(raw, |key| matches!(role(key), Role::Warnings)).unwrap_or_else(|| "warnings".into());
    raw.insert(key, Value::Array(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| Value::String(l.into())).collect()));
}

/// A line at the end of the journal: «17.11.2026 — text».
pub fn add_journal(raw: &mut Raw, date: &str, text: &str) {
    let key = key_of(raw, |key| matches!(role(key), Role::Journal)).unwrap_or_else(|| "journal".into());
    let line = Value::String(format!("{date} — {}", text.trim()));
    match raw.get_mut(&key) {
        Some(Value::Array(items)) => items.push(line),
        _ => { raw.insert(key, Value::Array(vec![line])); }
    }
}

/// The whole journal as typed (one entry a line).
pub fn set_journal(raw: &mut Raw, text: &str) {
    if from_raw(raw).journal.join("\n") == text { return; }
    let key = key_of(raw, |key| matches!(role(key), Role::Journal)).unwrap_or_else(|| "journal".into());
    raw.insert(key, Value::Array(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| Value::String(l.into())).collect()));
}

pub fn set_title(raw: &mut Raw, name: &str) {
    let key = key_of(raw, |key| matches!(role(key), Role::Title)).unwrap_or_else(|| "name".into());
    if raw.contains_key(&key) { raw.insert(key, Value::String(name.trim().into())); }
    else { raw.shift_insert(0, key, Value::String(name.trim().into())); }
}

/// «обновлено …»: the date and who wrote it last.
pub fn touch(raw: &mut Raw, date: &str, by: &str) {
    let updated = key_of(raw, |key| matches!(role(key), Role::Updated)).unwrap_or_else(|| "updated".into());
    let writer = key_of(raw, |key| matches!(role(key), Role::UpdatedBy)).unwrap_or_else(|| "updated_by".into());
    raw.insert(updated, Value::String(date.into()));
    raw.insert(writer, Value::String(by.into()));
}

/// Writes the passport. `expected` is when it was read (`None`: it did not
/// exist); if the file changed since, nothing is written and the error
/// says so. Returns when it was written.
pub fn write(path: &Path, raw: &Raw, expected: Option<SystemTime>) -> io::Result<Option<SystemTime>> {
    let now = fs::metadata(path).ok().map(|metadata| metadata.modified().ok());
    let unchanged = match (now, expected) {
        (None, None) => true,
        (Some(now), Some(expected)) => now == Some(expected),
        _ => false,
    };
    if !unchanged {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists,
            "паспорт изменён в другом месте — он перечитан, повторите правку"));
    }
    let text = serde_json::to_string_pretty(raw).map_err(io::Error::other)?;
    if text.len() as u64 > MAX_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "файл паспорта слишком велик (не более 2 МиБ)"));
    }
    crate::path_utils::write_whole(path, text.as_bytes())?;
    Ok(fs::metadata(path)?.modified().ok())
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
    fn an_oversized_edit_leaves_the_existing_passport_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE_NAME);
        let mut raw = template("Объект");
        let stamp = write(&file, &raw, None).unwrap();
        let before = fs::read(&file).unwrap();
        raw.insert("notes".into(), Value::String("я".repeat(MAX_BYTES as usize)));
        assert_eq!(write(&file, &raw, stamp).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&file).unwrap(), before);
    }

    #[test]
    fn editing_keeps_all_rows_long_text_and_unchanged_machine_schema() {
        let long = "я".repeat(1000);
        let mut raw = template("Объект");
        let text = (0..80).map(|i| format!("Модель {i}: {long}")).collect::<Vec<_>>().join("\n");
        set_block(&mut raw, Some("Модели"), "Модели", &text);
        let shown = from_raw(&raw);
        let section = shown.sections.iter().find(|s| s.key == "Модели").unwrap();
        assert_eq!(section.rows.len(), 80);
        assert_eq!(block_text(section), text);
        let edited = format!("{text}\nЕщё одна строка");
        set_block(&mut raw, Some("Модели"), "Модели", &edited);
        assert_eq!(from_raw(&raw).sections.iter().find(|s| s.key == "Модели").unwrap().rows.len(), 81);
        raw.insert("geology".into(), serde_json::json!({"ige": 9, "units": {"stress": "kPa"}, "nullable": null}));
        raw.insert("journal".into(), serde_json::json!([{"date":"19.11", "text":"Расчёт"}]));
        let before = raw.clone();
        let shown = from_raw(&raw);
        let section = shown.sections.iter().find(|s| s.key == "geology").unwrap();
        set_block(&mut raw, Some("geology"), &section.title, &block_text(section));
        set_journal(&mut raw, &shown.journal.join("\n"));
        assert_eq!(raw, before, "an unchanged text view preserves the original JSON");
    }

    #[test]
    fn the_journal_keeps_its_newest_entries_after_sixty_and_round_trips() {
        let mut raw = template("Объект");
        for i in 0..100 { add_journal(&mut raw, "19.11", &format!("Запись {i}")); }
        let shown = from_raw(&raw);
        assert_eq!(shown.journal.len(), 100);
        assert!(shown.journal.last().unwrap().ends_with("Запись 99"));
        let text = format!("{}\nНовая запись", shown.journal.join("\n"));
        set_journal(&mut raw, &text);
        assert_eq!(from_raw(&raw).journal.len(), 101);
        assert_eq!(from_raw(&raw).journal.last().unwrap(), "Новая запись");
    }

    #[test]
    fn both_file_readers_refuse_oversized_input_and_accept_bom() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE_NAME);
        fs::write(&file, format!("\u{feff}{}", serde_json::json!({"name":"Объект"}))).unwrap();
        assert_eq!(read_raw(&file).unwrap().0["name"], "Объект");
        fs::write(&file, vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert_eq!(read_raw(&file).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(read(&file).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

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
        assert_eq!(titles, ["Объект", "Геология", "Модели", "Итерации", "empty"], "the file's order; empty blocks stay, to be filled");
        assert_eq!(passport.sections[0].rows, [("Шифр".to_string(), "044".to_string())]);
        assert_eq!(passport.sections[1].key, "geology");
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
        assert_eq!(parse(&long).unwrap().sections[0].rows[0].1.chars().count(), 1000);
    }

    #[test]
    fn papka_keeps_a_passport_block_by_block() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE_NAME);
        let mut raw = template("Корпус 2");
        assert_eq!(write(&file, &raw, Some(SystemTime::UNIX_EPOCH)).unwrap_err().kind(), io::ErrorKind::AlreadyExists,
            "a passport that is not there is not \"unchanged\"");
        let mut stamp = write(&file, &raw, None).unwrap();
        let shown = read(&file).unwrap();
        assert_eq!(shown.title.as_deref(), Some("Корпус 2"));
        let general = shown.sections.iter().find(|s| s.key == GENERAL_KEY).unwrap();
        assert_eq!(block_text(general).lines().next(), Some("Шифр: "));

        set_block(&mut raw, Some(GENERAL_KEY), "Объект", "Шифр: 044\nАдрес: Москва, ЗАО\nсвободная строка");
        set_block(&mut raw, Some("Геология"), "Инженерная геология", "База: Z:/044/geo.sqlite\nИГЭ: 9");
        set_block(&mut raw, None, "Нагрузки", "Сбор: 12.11");
        set_warnings(&mut raw, "Модель ЛИРА изменена после расчёта\n\n");
        add_journal(&mut raw, "17.11.2026", "Пересчитан C1");
        touch(&mut raw, "17.11.2026", "Papka");
        stamp = write(&file, &raw, stamp).unwrap();
        let shown = read(&file).unwrap();
        let titles: Vec<&str> = shown.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Объект", "Инженерная геология", "Модели", "Нагрузки"], "renamed in place, new after the others");
        assert_eq!(shown.sections[0].rows, [("Шифр".into(), "044".into()), ("Адрес".into(), "Москва, ЗАО".into()), (String::new(), "свободная строка".into())]);
        assert_eq!(block_text(&shown.sections[1]), "База: Z:/044/geo.sqlite\nИГЭ: 9", "what is typed comes back");
        assert_eq!(shown.warnings, ["Модель ЛИРА изменена после расчёта"]);
        assert_eq!(shown.journal, ["17.11.2026 — Пересчитан C1"]);
        assert_eq!(shown.updated.as_deref(), Some("17.11.2026 · Papka"));

        // Someone else saved meanwhile: refused, nothing lost on disk.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&file, "{\"name\":\"чужая правка\"}").unwrap();
        let other = fs::read_to_string(&file).unwrap();
        assert!(write(&file, &raw, stamp).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), other);
        remove_block(&mut raw, "Нагрузки");
        assert!(!raw.contains_key("Нагрузки"));
    }

    #[test]
    fn loose_values_of_another_program_become_one_block_when_edited() {
        let mut raw = match serde_json::from_str::<Value>(r#"{"name":"Остров","code":"044","geology":{"ige":9},"address":"Москва"}"#).unwrap() {
            Value::Object(raw) => raw, _ => unreachable!() };
        let shown = parse(&Value::Object(raw.clone()).to_string()).unwrap();
        assert_eq!(shown.sections[0].key, "", "the loose values");
        set_block(&mut raw, Some(""), "Объект", "Шифр: 044-1");
        assert!(!raw.contains_key("code") && !raw.contains_key("address"));
        let keys: Vec<&str> = raw.keys().map(String::as_str).collect();
        assert_eq!(keys, ["name", GENERAL_KEY, "geology"], "where the first loose value was");
        // A block named like a role or like another block gets a number.
        set_block(&mut raw, None, "geology", "x: 1");
        set_block(&mut raw, None, "journal", "y: 2");
        assert!(raw.contains_key("geology 2") && raw.contains_key("journal 2"));
        set_title(&mut raw, "Остров-2");
        assert_eq!(raw["name"], "Остров-2");
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
