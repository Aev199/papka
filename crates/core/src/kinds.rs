//! Kinds of files a folder can be narrowed to in one click: drawings, PDF,
//! documents and so on, by extension.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Drawings,
    Pdf,
    Documents,
    Sheets,
    Images,
    Archives,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Drawings, Kind::Pdf, Kind::Documents, Kind::Sheets, Kind::Images, Kind::Archives];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Drawings => "Чертежи и модели",
            Kind::Pdf => "PDF",
            Kind::Documents => "Документы",
            Kind::Sheets => "Таблицы",
            Kind::Images => "Изображения",
            Kind::Archives => "Архивы",
        }
    }

    fn extensions(self) -> &'static [&'static str] {
        match self {
            Kind::Drawings => &["dwg", "dxf", "dwf", "dwfx", "dws", "dwt", "dgn", "rvt", "rfa", "rte", "ifc", "nwd", "nwc", "nwf", "pln", "pla", "skp", "step", "stp", "sat"],
            Kind::Pdf => &["pdf"],
            Kind::Documents => &["doc", "docx", "docm", "rtf", "odt", "txt", "md", "ppt", "pptx", "odp"],
            Kind::Sheets => &["xls", "xlsx", "xlsm", "xlsb", "csv", "ods"],
            Kind::Images => &["png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp", "heic", "svg"],
            Kind::Archives => &["zip", "7z", "rar", "tar", "gz", "bz2", "xz"],
        }
    }

    /// The kind of a file by its extension, if it is one of these.
    pub fn of(path: &Path) -> Option<Kind> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        Kind::ALL.into_iter().find(|kind| kind.extensions().contains(&extension.as_str()))
    }
}

/// How many files of each kind `names` hold, kinds with none left out.
pub fn count<'a>(names: impl IntoIterator<Item = &'a Path>) -> Vec<(Kind, usize)> {
    let mut counts = [0usize; 6];
    for name in names {
        if let Some(kind) = Kind::of(name) { counts[Kind::ALL.iter().position(|k| *k == kind).unwrap_or(0)] += 1; }
    }
    Kind::ALL.into_iter().zip(counts).filter(|(_, n)| *n > 0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension_in_any_case() {
        assert_eq!(Kind::of(Path::new("План.DWG")), Some(Kind::Drawings));
        assert_eq!(Kind::of(Path::new("model.rvt")), Some(Kind::Drawings));
        assert_eq!(Kind::of(Path::new("Отчёт_v_1_6.docx")), Some(Kind::Documents));
        assert_eq!(Kind::of(Path::new("scan.pdf")), Some(Kind::Pdf));
        assert_eq!(Kind::of(Path::new("notes")), None);
        assert_eq!(Kind::of(Path::new("tool.exe")), None);
        let names = ["a.dwg", "b.dwg", "c.pdf", "d.exe"].map(Path::new);
        assert_eq!(count(names), vec![(Kind::Drawings, 2), (Kind::Pdf, 1)]);
    }
}
