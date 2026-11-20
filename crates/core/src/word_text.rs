//! Read-only, bounded text preview. No Office automation or persistent text cache.
//! Binary Word 97–2003: MS-DOC 2.4.1, FIB -> CLX -> PlcPcd -> text pieces.
use std::{fs::File, io::{self, Cursor, Read}, path::Path};

const FILE_LIMIT: u64 = 64 * 1024 * 1024;
const XML_LIMIT: u64 = 16 * 1024 * 1024;
const TEXT_LIMIT: usize = 64 * 1024;

#[derive(Debug)]
pub struct Preview {
    pub text: String,
    pub truncated: bool,
}

fn invalid() -> io::Error { io::Error::other("Формат Word не поддерживается или документ повреждён") }

pub fn supports(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("doc") || s.eq_ignore_ascii_case("docx"))
}

pub fn read(path: &Path) -> io::Result<Preview> {
    crate::guard::catch("Текстовый предпросмотр Word", || read_inner(path))?
}

fn read_inner(path: &Path) -> io::Result<Preview> {
    crate::folder_copy::reject_link(path)?;
    if !supports(path) { return Err(invalid()); }
    let file = File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > FILE_LIMIT {
        return Err(io::Error::other("Текстовый предпросмотр Word поддерживает файлы до 64 МБ"));
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_LIMIT { return Err(invalid()); }
    if bytes.starts_with(b"PK") { docx(bytes) } else { doc(bytes) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn put16(bytes: &mut [u8], at: usize, value: u16) { bytes[at..at+2].copy_from_slice(&value.to_le_bytes()); }
    fn put32(bytes: &mut [u8], at: usize, value: u32) { bytes[at..at+4].copy_from_slice(&value.to_le_bytes()); }

    // A genuine OLE container with a Word FIB and two pieces deliberately
    // stored in reverse physical order, then an out-of-body comment piece.
    fn binary(path: &Path, encrypted: bool, table_one: bool) {
        let first = "Записка\r".encode_utf16().collect::<Vec<_>>();
        let second = b"A\x13 SECRET \x14shown\x15 B\x97C\r";
        let main = first.len() + second.len();
        let mut word = vec![0; 2048];
        put16(&mut word, 0, 0xa5ec); put16(&mut word, 2, 0xc1);
        put16(&mut word, 10, if encrypted { 0x100 } else if table_one { 0x200 } else { 0 });
        put16(&mut word, 32, 14); put16(&mut word, 62, 22); put32(&mut word, 76, main as u32);
        put16(&mut word, 152, 93);
        word[512..512+second.len()].copy_from_slice(second);
        for (i, c) in first.iter().enumerate() { put16(&mut word, 1024+i*2, *c); }
        word[1500..1506].copy_from_slice(b"SECRET");
        let mut plc = vec![0; 40];
        for (i, cp) in [0, first.len(), main, main+6].into_iter().enumerate() { put32(&mut plc, i*4, cp as u32); }
        put32(&mut plc, 18, 1024);
        put32(&mut plc, 26, 0x4000_0000 | (512*2));
        put32(&mut plc, 34, 0x4000_0000 | (1500*2));
        let mut table = vec![1, 2, 0, 0, 0, 2]; // one formatting group
        table.extend_from_slice(&(plc.len() as u32).to_le_bytes()); table.extend(plc);
        put32(&mut word, 418, 0); put32(&mut word, 422, table.len() as u32);
        let mut cfb = cfb::create(path).unwrap();
        cfb.create_stream("WordDocument").unwrap().write_all(&word).unwrap();
        cfb.create_stream(if table_one { "1Table" } else { "0Table" }).unwrap().write_all(&table).unwrap();
    }

    fn xml(path: &Path, content: &str) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(content.as_bytes()).unwrap(); zip.finish().unwrap();
    }

    #[test]
    fn binary_word_reads_cyrillic_piece_order_and_field_results_only() {
        let dir = tempfile::tempdir().unwrap();
        for one in [false, true] {
            let path = dir.path().join("записка.DOC"); binary(&path, false, one);
            let before = std::fs::read(&path).unwrap();
            let preview = read(&path).unwrap();
            assert_eq!(preview.text, "Записка\nAshown B—C");
            assert!(!preview.truncated);
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
    }

    #[test]
    fn encrypted_corrupt_and_oversized_files_fail_without_text() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("a.doc");
        binary(&path, true, true);
        assert!(read(&path).unwrap_err().to_string().contains("паролем"));
        std::fs::write(&path, b"not Word").unwrap(); assert!(read(&path).is_err());
        File::create(&path).unwrap().set_len(FILE_LIMIT+1).unwrap(); assert!(read(&path).is_err());
        binary(&path, false, true);
        let mut cfb = cfb::open_rw(&path).unwrap();
        use std::io::{Seek, SeekFrom};
        let mut stream = cfb.open_stream("1Table").unwrap();
        stream.seek(SeekFrom::Start(28)).unwrap(); stream.write_all(&u32::MAX.to_le_bytes()).unwrap();
        drop(stream); drop(cfb); assert!(read(&path).is_err());
    }

    #[test]
    fn docx_keeps_paragraphs_unicode_entities_and_breaks_but_not_deleted_text() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("a.docx");
        xml(&path, r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Привет &amp; мир</w:t><w:tab/><w:t>А</w:t><w:br/><w:t>Б</w:t></w:r><w:del><w:r><w:t>deleted</w:t></w:r></w:del></w:p><w:p><w:r><w:instrText>SECRET</w:instrText><w:t>Второй абзац</w:t></w:r></w:p></w:body></w:document>"#);
        assert_eq!(read(&path).unwrap().text, "Привет & мир\tА\nБ\nВторой абзац");
        // No text is written beside the source document.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn docx_text_is_bounded_and_dtd_is_rejected() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("a.docx");
        xml(&path, &format!("<document><t>{}</t></document>", "Я".repeat(TEXT_LIMIT+10)));
        let preview = read(&path).unwrap(); assert!(preview.truncated);
        assert_eq!(preview.text.chars().count(), TEXT_LIMIT);
        xml(&path, "<!DOCTYPE document [<!ENTITY secret SYSTEM 'file:///secret'>]><document><t>&secret;</t></document>");
        assert!(read(&path).is_err());
        xml(&path, "<document><t>broken</document>"); assert!(read(&path).is_err());
    }
}

fn u16_at(bytes: &[u8], at: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes(bytes.get(at..at + 2).ok_or_else(invalid)?.try_into().unwrap()))
}
fn u32_at(bytes: &[u8], at: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(bytes.get(at..at + 4).ok_or_else(invalid)?.try_into().unwrap()))
}
fn bounded(mut input: impl Read, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    (&mut input).take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit { return Err(invalid()); }
    Ok(bytes)
}

fn doc(bytes: Vec<u8>) -> io::Result<Preview> {
    let mut compound = cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|_| invalid())?;
    let word = bounded(compound.open_stream("WordDocument")?, FILE_LIMIT)?;
    if u16_at(&word, 0)? != 0xa5ec || u16_at(&word, 2)? < 0xc1 { return Err(invalid()); }
    let flags = u16_at(&word, 10)?;
    if flags & 0x8100 != 0 { return Err(io::Error::other("Документ защищён паролем. Откройте его в Word")); }
    let lw_count = 34 + u16_at(&word, 32)? as usize * 2;
    let lw = lw_count + 2;
    let count = u16_at(&word, lw_count)? as usize;
    if count < 4 { return Err(invalid()); }
    let main_chars = u32_at(&word, lw + 12)? as usize;
    let fc_count = lw + count * 4;
    if u16_at(&word, fc_count)? < 34 { return Err(invalid()); }
    let clx_at = u32_at(&word, fc_count + 2 + 33 * 8)? as usize;
    let clx_len = u32_at(&word, fc_count + 6 + 33 * 8)? as usize;
    let table = bounded(compound.open_stream(if flags & 0x0200 != 0 { "1Table" } else { "0Table" })?, FILE_LIMIT)?;
    let clx = table.get(clx_at..clx_at.checked_add(clx_len).ok_or_else(invalid)?).ok_or_else(invalid)?;
    let mut at = 0;
    // Optional formatting groups precede the piece table.
    while clx.get(at) == Some(&1) {
        at += 3 + u16_at(clx, at + 1)? as usize;
    }
    if clx.get(at) != Some(&2) { return Err(invalid()); }
    let len = u32_at(clx, at + 1)? as usize;
    let plc = clx.get(at + 5..(at + 5).checked_add(len).ok_or_else(invalid)?).ok_or_else(invalid)?;
    if len < 4 || (len - 4) % 12 != 0 { return Err(invalid()); }
    let pieces = (len - 4) / 12;
    let descriptors = (pieces + 1) * 4;
    if u32_at(plc, 0)? != 0 { return Err(invalid()); }
    let mut units = Vec::new();
    let limit = main_chars.min(TEXT_LIMIT + 1);
    for index in 0..pieces {
        let cp = u32_at(plc, index * 4)? as usize;
        let next = u32_at(plc, (index + 1) * 4)? as usize;
        if next < cp { return Err(invalid()); }
        if cp >= limit { break; }
        let n = next.min(limit) - cp;
        let fc = u32_at(plc, descriptors + index * 8 + 2)?;
        let compressed = fc & 0x4000_0000 != 0;
        let mut offset = (fc & 0x3fff_ffff) as usize;
        let width = if compressed { offset /= 2; 1 } else { 2 };
        let text = word.get(offset..offset.checked_add(n * width).ok_or_else(invalid)?).ok_or_else(invalid)?;
        if compressed {
            // MS-DOC FcCompressed: 8-bit Unicode with the specified punctuation map.
            for &b in text {
                let c = match b {
                    0x82 => 0x201a, 0x83 => 0x0192, 0x84 => 0x201e, 0x85 => 0x2026,
                    0x86 => 0x2020, 0x87 => 0x2021, 0x88 => 0x02c6, 0x89 => 0x2030,
                    0x8a => 0x0160, 0x8b => 0x2039, 0x8c => 0x0152,
                    0x91 => 0x2018, 0x92 => 0x2019, 0x93 => 0x201c, 0x94 => 0x201d,
                    0x95 => 0x2022, 0x96 => 0x2013, 0x97 => 0x2014, 0x98 => 0x02dc,
                    0x99 => 0x2122, 0x9a => 0x0161, 0x9b => 0x203a, 0x9c => 0x0153,
                    0x9f => 0x0178, _ => b as u16,
                };
                units.push(c);
            }
        } else {
            units.extend(text.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])));
        }
    }
    if units.len() < limit { return Err(invalid()); }
    let mut fields = Vec::new();
    let text: String = String::from_utf16_lossy(&units).chars().filter_map(|c| {
        match c {
            '\u{13}' => { fields.push(false); None }
            '\u{14}' => { if let Some(result) = fields.last_mut() { *result = true; } None }
            '\u{15}' => { fields.pop(); None }
            _ if fields.iter().any(|result| !result) => None,
            '\r' | '\u{b}' | '\u{c}' => Some('\n'),
            '\u{7}' => Some('\t'),
            _ if c.is_control() && c != '\t' && c != '\n' => None,
            _ => Some(c),
        }
    }).collect();
    Ok(Preview { text: text.trim().to_string(), truncated: main_chars > TEXT_LIMIT })
}

fn docx(bytes: Vec<u8>) -> io::Result<Preview> {
    use quick_xml::{Reader, events::Event};
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid())?;
    let document = archive.by_name("word/document.xml").map_err(|_| invalid())?;
    if document.size() > XML_LIMIT { return Err(io::Error::other("Текстовая часть документа слишком велика для быстрого просмотра")); }
    let xml = bounded(document, XML_LIMIT)?;
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut text = String::new();
    let mut in_text = false;
    let mut deleted = 0usize;
    let mut chars = 0;
    let mut truncated = false;
    loop {
        let event = reader.read_event().map_err(|_| invalid())?;
        let mut value = String::new();
        match event {
            Event::Start(e) => match e.local_name().as_ref() {
                b"del" => deleted += 1,
                b"t" if deleted == 0 => in_text = true,
                b"tab" if deleted == 0 => value.push('\t'),
                b"br" | b"cr" if deleted == 0 => value.push('\n'),
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                b"del" => deleted = deleted.saturating_sub(1),
                b"t" => in_text = false,
                b"p" | b"tr" if deleted == 0 => value.push('\n'),
                b"tc" if deleted == 0 => value.push('\t'),
                _ => {}
            },
            Event::Empty(e) if deleted == 0 => match e.local_name().as_ref() {
                b"tab" => value.push('\t'), b"br" | b"cr" => value.push('\n'), _ => {}
            },
            Event::Text(e) if in_text => value = e.unescape().map_err(|_| invalid())?.into_owned(),
            Event::DocType(_) => return Err(invalid()),
            Event::Eof => break,
            _ => {}
        }
        for c in value.chars() {
            if chars == TEXT_LIMIT { truncated = true; break; }
            text.push(c);
            chars += 1;
        }
        if truncated { break; }
    }
    Ok(Preview { text: text.trim().to_string(), truncated })
}
