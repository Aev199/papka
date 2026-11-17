//! Project inboxes: material other programs (Notch) send to a project —
//! a screenshot with a caption, a piece of text, a link to a file.
//!
//! Papka publishes its projects in `%LOCALAPPDATA%\Papka\projects.json`;
//! a sender picks a project there and writes an entry into its inbox
//! folder. Papka only reads entries and never changes them. The format is
//! described in `docs/INBOX.md`; the two sides share no code, only it.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const REGISTRY_FILE: &str = "projects.json";
/// The inbox inside a project's folder.
pub const INBOX_DIR: &str = "_Входящие";
pub const FORMAT: u32 = 1;
/// Entries read at most; an inbox is for what has not been sorted yet.
const MAX_ENTRIES: usize = 300;
const MAX_SIDECAR: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryProject {
    pub id: String,
    pub name: String,
    /// The object's folder, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
    pub inbox: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    pub format: u32,
    pub program: String,
    pub projects: Vec<RegistryProject>,
}

pub fn registry_path() -> PathBuf {
    crate::path_utils::app_data_dir().join(REGISTRY_FILE)
}

/// Where a project's inbox lives: in its folder when it has one, so the
/// material stays with the object; otherwise in Papka's profile.
pub fn inbox_for(root: Option<&Path>, id: &str) -> PathBuf {
    match root {
        Some(root) => root.join(INBOX_DIR),
        None => crate::path_utils::app_data_dir().join("inbox").join(id),
    }
}

/// Writes the registry so that a reader never sees half a file: a
/// temporary file beside it, then a rename over the old one.
pub fn write_registry(path: &Path, projects: Vec<RegistryProject>) -> io::Result<()> {
    let registry = Registry { format: FORMAT, program: "Papka".into(), projects };
    let text = serde_json::to_string_pretty(&registry).map_err(io::Error::other)?;
    let folder = path.parent().ok_or_else(|| io::Error::other("у файла проектов нет папки"))?;
    fs::create_dir_all(folder)?;
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// A folder deep enough to be an object, not `Z:\` or `\\server\share`.
fn deep_enough(path: &Path) -> bool {
    use std::path::Component;
    path.components().filter(|part| matches!(part, Component::Normal(_))).count() >= 2
}

/// The object's folder of a project without one set by hand: where its
/// passport lies, else the folder all its tabs share — never a drive or
/// share root.
pub fn derive_root(passport: Option<&Path>, folders: &[PathBuf]) -> Option<PathBuf> {
    if let Some(folder) = passport.and_then(Path::parent).filter(|folder| deep_enough(folder)) {
        return Some(folder.to_path_buf());
    }
    let mut common: Option<PathBuf> = None;
    for folder in folders {
        common = Some(match common {
            None => folder.clone(),
            Some(shared) => shared.ancestors().find(|ancestor| folder.starts_with(ancestor))?.to_path_buf(),
        });
    }
    common.filter(|folder| deep_enough(folder))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Image,
    Text,
    File,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub window: String,
}

/// The `.json` written for each entry, as `docs/INBOX.md` describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    #[serde(default)]
    pub format: u32,
    #[serde(default)]
    pub kind: Kind,
    /// Local time with its offset, e.g. "2026-11-17T14:32:05+03:00".
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub caption: String,
    /// The text sent (kind "text").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The picture beside this file (kind "image"): its name only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// The file it is about: the file sent (kind "file") or the document a
    /// screenshot was taken from, if the sender knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub project: String,
}

/// One entry of an inbox as Papka shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub sidecar_path: PathBuf,
    pub sidecar: Sidecar,
    /// The picture, if the entry has one and it is there.
    pub image: Option<PathBuf>,
}

impl Entry {
    /// "17.11 14:32" from `created`; the file name's stamp if that is unreadable.
    pub fn when(&self) -> String {
        let created = &self.sidecar.created;
        chrono::DateTime::parse_from_rfc3339(created)
            .map(|time| time.with_timezone(&chrono::Local).format("%d.%m %H:%M").to_string())
            .unwrap_or_else(|_| created.chars().take(16).collect())
    }

    /// What the entry is called: its caption, else what it holds.
    pub fn title(&self) -> String {
        let caption = self.sidecar.caption.trim();
        if !caption.is_empty() { return caption.to_string(); }
        match self.sidecar.kind {
            Kind::Text => self.sidecar.text.as_deref().unwrap_or_default().lines().next().unwrap_or_default().chars().take(80).collect(),
            Kind::File => self.sidecar.file.as_deref().map(crate::browser::display_name).unwrap_or_default(),
            Kind::Image => "Снимок без подписи".into(),
        }
    }
}

/// The entries of `inbox`, newest first. A sender writes the picture
/// first and the `.json` last, so an entry is read only once complete;
/// temporary files and broken `.json` are passed over.
pub fn read(inbox: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(inbox)? {
        let path = item?.path();
        let is_sidecar = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
        if !is_sidecar || path.metadata().map_or(true, |metadata| metadata.len() > MAX_SIDECAR) { continue; }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(sidecar) = serde_json::from_str::<Sidecar>(text.trim_start_matches('\u{feff}')) else { continue };
        let image = sidecar.image.as_deref()
            .filter(|name| !name.contains(['/', '\\']) && !name.contains(".."))
            .map(|name| inbox.join(name)).filter(|image| image.is_file());
        entries.push(Entry { sidecar_path: path, sidecar, image });
    }
    // Names start with a sortable stamp; the newest first.
    entries.sort_by(|a, b| b.sidecar_path.file_name().cmp(&a.sidecar_path.file_name()));
    entries.truncate(MAX_ENTRIES);
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_is_written_whole_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Papka").join(REGISTRY_FILE);
        let project = RegistryProject { id: "p-1".into(), name: "Корпус 2".into(), root: Some(PathBuf::from("Z:/044")), inbox: PathBuf::from("Z:/044/_Входящие") };
        write_registry(&path, vec![project.clone()]).unwrap();
        write_registry(&path, vec![project.clone()]).unwrap();
        let read: Registry = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((read.format, read.projects), (FORMAT, vec![project]));
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1, "no temporary files left");
    }

    #[test]
    fn the_object_folder_is_the_passport_or_what_the_tabs_share() {
        let tabs = [PathBuf::from("/srv/044 Сити/ИГИ"), PathBuf::from("/srv/044 Сити/КЖ/Сваи")];
        assert_eq!(derive_root(None, &tabs), Some(PathBuf::from("/srv/044 Сити")));
        assert_eq!(derive_root(Some(Path::new("/srv/044 Сити/Корпус 2/object.json")), &tabs), Some(PathBuf::from("/srv/044 Сити/Корпус 2")));
        // Only a drive or share in common: no folder of the object.
        assert_eq!(derive_root(None, &[PathBuf::from("/srv/a/x"), PathBuf::from("/mnt/b")]), None);
        assert_eq!(derive_root(None, &[PathBuf::from("/srv")]), None);
        assert_eq!(derive_root(None, &[]), None);
        assert_eq!(derive_root(None, &[PathBuf::from("/srv/044/ИГИ")]), Some(PathBuf::from("/srv/044/ИГИ")));
        assert_eq!(inbox_for(Some(Path::new("/srv/044")), "p-1"), PathBuf::from("/srv/044").join(INBOX_DIR));
        assert!(inbox_for(None, "p-1").ends_with("inbox/p-1"));
    }

    #[test]
    fn complete_entries_are_read_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path();
        fs::write(inbox.join("20261117-143205 Привязка С-18.png"), b"png").unwrap();
        fs::write(inbox.join("20261117-143205 Привязка С-18.json"), r#"{"format":1,"kind":"image","created":"2026-11-17T14:32:05+03:00",
            "caption":"Сверить привязку С-18","image":"20261117-143205 Привязка С-18.png","source":{"app":"acad.exe","window":"Корпус 2.dwg"},"project":"p-1"}"#).unwrap();
        fs::write(inbox.join("20261117-150000 Текст.json"), "\u{feff}{\"kind\":\"text\",\"text\":\"Позвонить ГИПу\\nвторая строка\"}").unwrap();
        fs::write(inbox.join("20261117-151000 Файл.json"), r#"{"kind":"file","file":"/srv/044/Сваи.dwg"}"#).unwrap();
        // Half written, a broken one, a picture that never got its .json, one that points outside.
        fs::write(inbox.join("20261117-160000 x.json.tmp"), "{").unwrap();
        fs::write(inbox.join("20261117-161000 broken.json"), "{ not json").unwrap();
        fs::write(inbox.join("20261117-162000 alone.png"), b"png").unwrap();
        fs::write(inbox.join("20261117-163000 escape.json"), r#"{"kind":"image","image":"..\\secret.png"}"#).unwrap();
        let entries = read(inbox).unwrap();
        let titles: Vec<String> = entries.iter().map(Entry::title).collect();
        assert_eq!(titles, ["Снимок без подписи", "Сваи.dwg", "Позвонить ГИПу", "Сверить привязку С-18"]);
        assert_eq!(entries[0].image, None, "a picture outside the inbox is not shown");
        assert_eq!(entries[3].image, Some(inbox.join("20261117-143205 Привязка С-18.png")));
        assert_eq!(entries[3].sidecar.source.app, "acad.exe");
        assert!(read(&inbox.join("missing")).is_err());
    }
}
