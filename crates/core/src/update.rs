//! Self-update of the portable EXE from GitHub Releases.
//!
//! The manual build publishes a release with `Papka.exe` and `latest.json`
//! (version, commit, SHA-256, change list). The app picks the newest release
//! of its channel, downloads the EXE, checks its hash and swaps files by
//! renaming: Windows lets a running EXE be renamed, not overwritten. The
//! previous EXE stays beside it as `<name>.old.exe` for one-click rollback.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const REPOSITORY: &str = "Aev199/papka";
pub const EXE_ASSET: &str = "Papka.exe";
pub const MANIFEST_ASSET: &str = "latest.json";
/// No Papka EXE comes near this; a longer answer is not one.
pub const MAX_DOWNLOAD: u64 = 512 << 20;

fn too_large() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "файл слишком велик для обновления Papka")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Channel {
    /// Releases marked as final only.
    #[default]
    Stable,
    /// Every build, test ones included.
    Beta,
}

impl Channel {
    pub fn key(self) -> &'static str {
        match self { Channel::Stable => "stable", Channel::Beta => "beta" }
    }
    pub fn from_key(key: &str) -> Channel {
        if key == "stable" { Channel::Stable } else { Channel::Beta }
    }
    pub fn label(self) -> &'static str {
        match self { Channel::Stable => "Стабильные", Channel::Beta => "Все сборки (бета)" }
    }
}

/// "v0.1.44" / "0.1.44-dev" → [0, 1, 44].
pub fn parse_version(text: &str) -> Vec<u64> {
    text.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']).map_while(|part| part.parse().ok()).collect()
}

pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let (a, b) = (parse_version(a), parse_version(b));
    (0..a.len().max(b.len()))
        .map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0)))
        .find(|ordering| ordering.is_ne())
        .unwrap_or(Ordering::Equal)
}

#[derive(Deserialize)]
struct GithubAsset { name: String, browser_download_url: String }

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)] draft: bool,
    #[serde(default)] prerelease: bool,
    #[serde(default)] assets: Vec<GithubAsset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub version: String,
    pub page: String,
    pub exe_url: String,
    pub manifest_url: String,
    pub prerelease: bool,
}

/// The newest complete release of `channel` in GitHub's release list.
pub fn pick_release(list: &[u8], channel: Channel) -> io::Result<Option<Candidate>> {
    let releases: Vec<GithubRelease> = serde_json::from_slice(list).map_err(io::Error::other)?;
    Ok(releases.into_iter()
        .filter(|release| !release.draft && release.tag_name != NEWEST_TAG && (channel == Channel::Beta || !release.prerelease))
        .filter_map(|release| {
            let url = |name: &str| release.assets.iter().find(|asset| asset.name == name).map(|asset| asset.browser_download_url.clone());
            Some(Candidate {
                exe_url: url(EXE_ASSET)?,
                manifest_url: url(MANIFEST_ASSET)?,
                version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
                page: release.html_url,
                prerelease: release.prerelease,
            })
        })
        .max_by(|a, b| compare_versions(&a.version, &b.version)))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub version: String,
    #[serde(default)] pub commit: String,
    pub sha256: String,
    #[serde(default)] pub notes: Vec<String>,
    #[serde(default)] pub channel: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub commit: String,
    pub notes: Vec<String>,
    pub sha256: String,
    pub exe_url: String,
    pub page: String,
    pub prerelease: bool,
    pub origin: Origin,
}

/// Where an offered EXE comes from, and how far it can be trusted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Origin {
    /// Downloaded by Papka from the release; checked against its SHA-256.
    #[default]
    Release,
    /// A file from Downloads whose SHA-256 is the one published with the
    /// release of its version.
    DownloadedVerified,
    /// A file from Downloads that could not be compared: GitHub is out of
    /// reach. Only its name and version say it is Papka.
    DownloadedUnverified,
}

/// Fetches a URL; the app passes WinHTTP, tests pass a stub.
pub type Fetch<'a> = &'a dyn Fn(&str, &mut dyn FnMut(u64, Option<u64>)) -> io::Result<Vec<u8>>;

pub fn releases_url() -> String {
    format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page=100")
}

/// A newer version than `current` on `channel`, if there is one.
pub fn check(current: &str, channel: Channel, fetch: Fetch) -> io::Result<Option<Update>> {
    let list = fetch(&releases_url(), &mut |_, _| {})?;
    check_in(&list, current, channel, fetch)
}

/// `check` over a release list that is already fetched.
pub fn check_in(list: &[u8], current: &str, channel: Channel, fetch: Fetch) -> io::Result<Option<Update>> {
    let Some(candidate) = pick_release(list, channel)? else { return Ok(None) };
    if compare_versions(&candidate.version, current) != Ordering::Greater { return Ok(None); }
    let manifest: Manifest = serde_json::from_slice(&fetch(&candidate.manifest_url, &mut |_, _| {})?).map_err(io::Error::other)?;
    if compare_versions(&manifest.version, &candidate.version) != Ordering::Equal || manifest.sha256.len() != 64 {
        return Err(io::Error::other("описание выпуска не совпадает с самим выпуском"));
    }
    Ok(Some(Update {
        version: candidate.version,
        commit: manifest.commit,
        notes: manifest.notes,
        sha256: manifest.sha256.to_ascii_lowercase(),
        exe_url: candidate.exe_url,
        page: candidate.page,
        prerelease: candidate.prerelease,
        origin: Origin::Release,
    }))
}

/// The published release of exactly `version`, on any channel.
fn release_of(list: &[u8], version: &str) -> io::Result<Option<Candidate>> {
    let releases: Vec<GithubRelease> = serde_json::from_slice(list).map_err(io::Error::other)?;
    Ok(releases.into_iter()
        .filter(|release| !release.draft && release.tag_name != NEWEST_TAG
            && compare_versions(&release.tag_name, version) == Ordering::Equal)
        .find_map(|release| {
            let url = |name: &str| release.assets.iter().find(|asset| asset.name == name).map(|asset| asset.browser_download_url.clone());
            Some(Candidate {
                exe_url: url(EXE_ASSET)?,
                manifest_url: url(MANIFEST_ASSET)?,
                version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
                page: release.html_url,
                prerelease: release.prerelease,
            })
        }))
}

/// Compares a file found in Downloads with the release of its version.
/// `list` is GitHub's release list, or `None` when it could not be fetched.
///
/// The file is refused (`InvalidData`) when the releases can be read and
/// either have no such version or publish another SHA-256 for it: a file
/// that merely calls itself "Papka 9.9" is not an update. Without the
/// network the file is offered as `DownloadedUnverified`, and the interface
/// says so.
pub fn vet_downloaded(mut found: Update, list: Option<&[u8]>, fetch: Fetch) -> io::Result<Update> {
    found.origin = Origin::DownloadedUnverified;
    let Some(list) = list else { return Ok(found) };
    let Ok(release) = release_of(list, &found.version) else { return Ok(found) };
    let Some(release) = release else {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            format!("версии {} нет среди выпусков Papka — файл не установлен", found.version)));
    };
    let Ok(manifest) = fetch(&release.manifest_url, &mut |_, _| {})
        .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).map_err(io::Error::other)) else { return Ok(found) };
    if !manifest.sha256.trim().eq_ignore_ascii_case(&found.sha256) {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            format!("файл версии {} отличается от опубликованного выпуска — он не установлен", found.version)));
    }
    found.origin = Origin::DownloadedVerified;
    found.commit = manifest.commit;
    found.notes = manifest.notes;
    found.prerelease = release.prerelease;
    Ok(found)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Where the replaced EXE is kept for rollback.
pub fn previous_path(exe: &Path) -> PathBuf {
    let stem = exe.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "Papka".into());
    exe.with_file_name(format!("{stem}.old.exe"))
}

fn staged_path(exe: &Path) -> PathBuf {
    let stem = exe.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "Papka".into());
    exe.with_file_name(format!("{stem}.new.exe"))
}

/// Puts `bytes` in place of `exe` after checking the hash; the old EXE
/// becomes `previous_path(exe)`. Nothing changes if any step fails.
pub fn install(exe: &Path, bytes: &[u8], sha256: &str) -> io::Result<()> {
    if !sha256_hex(bytes).eq_ignore_ascii_case(sha256.trim()) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "контрольная сумма загруженного файла не совпала"));
    }
    if !bytes.starts_with(b"MZ") {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "загруженный файл — не программа Windows"));
    }
    let staged = staged_path(exe);
    let previous = previous_path(exe);
    let written = (|| {
        let mut file = fs::File::create(&staged)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = written {
        let _ = fs::remove_file(&staged);
        return Err(error);
    }
    // The kept previous version steps aside rather than being deleted
    // first: if the swap fails it returns, and if it is the running EXE
    // (installed again without a restart) renaming still works.
    let retired = exe.with_file_name(format!("{}.retired", previous.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()));
    let had_previous = previous.exists();
    if had_previous {
        let _ = fs::remove_file(&retired);
        if let Err(error) = fs::rename(&previous, &retired) {
            let _ = fs::remove_file(&staged);
            return Err(error);
        }
    }
    let restore_previous = || { if had_previous { let _ = fs::rename(&retired, &previous); } };
    if let Err(error) = fs::rename(exe, &previous) {
        let _ = fs::remove_file(&staged);
        restore_previous();
        return Err(error);
    }
    if let Err(error) = fs::rename(&staged, exe) {
        let _ = fs::rename(&previous, exe);
        let _ = fs::remove_file(&staged);
        restore_previous();
        return Err(error);
    }
    // A running image cannot be deleted; it goes on a later install.
    let _ = fs::remove_file(&retired);
    Ok(())
}

/// Swaps `exe` with the kept previous version, so it can be undone too.
pub fn rollback(exe: &Path) -> io::Result<()> {
    let previous = previous_path(exe);
    if !previous.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "предыдущей версии нет"));
    }
    let swap = staged_path(exe);
    let _ = fs::remove_file(&swap);
    fs::rename(exe, &swap)?;
    if let Err(error) = fs::rename(&previous, exe) {
        let _ = fs::rename(&swap, exe);
        return Err(error);
    }
    if let Err(error) = fs::rename(&swap, &previous) {
        // The previous version runs either way; only the copy to undo is lost.
        let _ = fs::remove_file(&swap);
        return Err(error);
    }
    Ok(())
}

/// A release that only holds the newest build's EXE under a fixed tag:
/// GitHub's own "latest" link skips test builds. It is not an update.
pub const NEWEST_TAG: &str = "beta";

/// The newest EXE of the channel, fetched by the browser when Papka itself
/// has no way out.
pub fn browser_url(channel: Channel) -> String {
    match channel {
        Channel::Stable => format!("https://github.com/{REPOSITORY}/releases/latest/download/{EXE_ASSET}"),
        Channel::Beta => format!("https://github.com/{REPOSITORY}/releases/download/{NEWEST_TAG}/{EXE_ASSET}"),
    }
}

/// The version in a Papka EXE's file properties ("0.1.47").
#[cfg(windows)]
pub fn exe_version(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO};
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let size = GetFileVersionInfoSizeW(name.as_ptr(), std::ptr::null_mut());
        if size == 0 { return None; }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(name.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 { return None; }
        let root = [u16::from(b'\\'), 0];
        let mut info: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut length = 0u32;
        if VerQueryValueW(data.as_ptr().cast(), root.as_ptr(), &mut info, &mut length) == 0 || info.is_null() { return None; }
        let info = &*(info as *const VS_FIXEDFILEINFO);
        let (ms, ls) = (info.dwFileVersionMS, info.dwFileVersionLS);
        Some(format!("{}.{}.{}", ms >> 16, ms & 0xFFFF, ls >> 16))
    }
}

#[cfg(not(windows))]
pub fn exe_version(_path: &Path) -> Option<String> { None }

/// The user's Downloads folder, wherever it was moved.
#[cfg(windows)]
pub fn downloads_dir() -> Option<PathBuf> {
    use windows::Win32::UI::Shell::{FOLDERID_Downloads, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Downloads, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let text = path.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(path.0 as *const _));
        text.map(PathBuf::from)
    }
}

#[cfg(not(windows))]
pub fn downloads_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Downloads"))
}

/// The newest `Papka*.exe` in `folder` (browsers add " (1)" to repeated
/// downloads) that is newer than `current`, with its version.
pub fn find_downloaded(folder: &Path, current: &str, version_of: &dyn Fn(&Path) -> Option<String>) -> Option<(PathBuf, String)> {
    downloaded_candidates(folder, current, version_of).into_iter().next()
}

/// A file as it was when looked at: a refused download is not compared
/// again (nor GitHub asked again) until it changes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileStamp {
    pub path: PathBuf,
    pub len: u64,
    pub modified: Option<std::time::SystemTime>,
}

pub fn stamp(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileStamp { path: path.to_path_buf(), len: metadata.len(), modified: metadata.modified().ok() })
}

/// Every `Papka*.exe` in `folder` newer than `current`, newest first: a
/// refused file must not hide a genuine one behind it.
pub fn downloaded_candidates(folder: &Path, current: &str, version_of: &dyn Fn(&Path) -> Option<String>) -> Vec<(PathBuf, String)> {
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    let mut found: Vec<(PathBuf, String)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
            name.starts_with("papka") && name.ends_with(".exe") && path.is_file()
        })
        .filter_map(|path| version_of(&path).map(|version| (path, version)))
        .filter(|(_, version)| compare_versions(version, current) == Ordering::Greater)
        .collect();
    found.sort_by(|a, b| compare_versions(&b.1, &a.1).then_with(|| a.0.cmp(&b.0)));
    found
}

/// The files in Downloads still worth a look: newer than `current` and not
/// refused before in their present state.
pub fn fresh_candidates(folder: &Path, current: &str, version_of: &dyn Fn(&Path) -> Option<String>,
    refused: &HashSet<FileStamp>) -> Vec<(PathBuf, String)> {
    downloaded_candidates(folder, current, version_of).into_iter()
        .filter(|(path, _)| stamp(path).is_none_or(|stamp| !refused.contains(&stamp)))
        .collect()
}

/// What a look at Downloads found.
#[derive(Debug, Default)]
pub struct DownloadedOffer {
    /// The first file (newest first) that passes `vet_downloaded`, or why
    /// the newest one was refused.
    pub offer: Option<io::Result<Update>>,
    /// Files refused for good in their present state: another version than
    /// published, or not Papka at all.
    pub refused: Vec<FileStamp>,
}

pub fn downloaded_offer(folder: &Path, current: &str, version_of: &dyn Fn(&Path) -> Option<String>,
    list: Option<&[u8]>, fetch: Fetch, refused: &HashSet<FileStamp>) -> DownloadedOffer {
    let mut result = DownloadedOffer::default();
    let mut refusal = None;
    for (path, version) in fresh_candidates(folder, current, version_of, refused).into_iter().take(5) {
        let checked = downloaded_update(&path, &version).and_then(|found| vet_downloaded(found, list, fetch));
        match checked {
            Ok(update) => { result.offer = Some(Ok(update)); return result; }
            Err(error) => {
                if error.kind() == io::ErrorKind::InvalidData { result.refused.extend(stamp(&path)); }
                refusal.get_or_insert(error);
            }
        }
    }
    result.offer = refusal.map(Err);
    result
}

/// An update offered from a file the browser downloaded.
pub fn downloaded_update(path: &Path, version: &str) -> io::Result<Update> {
    if fs::metadata(path)?.len() > MAX_DOWNLOAD { return Err(too_large()); }
    let bytes = fs::read(path)?;
    Ok(Update {
        version: version.to_string(),
        commit: String::new(),
        notes: Vec::new(),
        sha256: sha256_hex(&bytes),
        exe_url: path.to_string_lossy().into_owned(),
        page: path.parent().map(|parent| parent.to_string_lossy().into_owned()).unwrap_or_default(),
        prerelease: false,
        origin: Origin::DownloadedUnverified,
    })
}

/// The update's EXE: downloaded for a release, read for a file the browser fetched.
pub fn get(location: &str, progress: &mut dyn FnMut(u64, Option<u64>)) -> io::Result<Vec<u8>> {
    if location.starts_with("https://") { return fetch(location, progress); }
    let mut file = fs::File::open(location)?;
    let total = file.metadata().ok().map(|metadata| metadata.len());
    if total.is_some_and(|total| total > MAX_DOWNLOAD) { return Err(too_large()); }
    let mut body = Vec::with_capacity(total.unwrap_or(0).min(64 << 20) as usize);
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = io::Read::read(&mut file, &mut buffer)?;
        if read == 0 { break; }
        body.extend_from_slice(&buffer[..read]);
        if body.len() as u64 > MAX_DOWNLOAD { return Err(too_large()); }
        progress(body.len() as u64, total);
    }
    Ok(body)
}

/// PowerShell writes errors to a redirected stream as CLIXML; this is
/// the readable text inside.
pub fn clixml_text(output: &str) -> String {
    if !output.trim_start().starts_with("#< CLIXML") { return output.trim().to_string(); }
    let mut parts = Vec::new();
    let mut rest = output;
    while let Some(start) = rest.find("<S S=\"Error\">") {
        rest = &rest[start + "<S S=\"Error\">".len()..];
        let Some(end) = rest.find("</S>") else { break };
        parts.push(&rest[..end]);
        rest = &rest[end..];
    }
    let text = parts.concat()
        .replace("_x000D_", "").replace("_x000A_", " ")
        .replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One readable sentence of an error text: no line breaks, no stack, at
/// most about 160 characters.
pub fn short_error(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = text.trim_end_matches('.');
    if text.is_empty() { return "неизвестная ошибка".into(); }
    if text.chars().count() <= 160 { return text.to_string(); }
    format!("{}…", text.chars().take(160).collect::<String>().trim_end())
}

/// Readable text for WinHTTP's own error codes, which Windows cannot format.
pub fn winhttp_message(code: i32) -> Option<&'static str> {
    Some(match code {
        12002 => "сервер не ответил вовремя",
        12005 | 12006 => "неверный адрес",
        12007 => "сервер не найден: нет интернета или DNS",
        12029 => "не удалось подключиться к серверу: сеть, прокси, брандмауэр или антивирус не пропускают программу",
        12030 | 12031 => "соединение прервано",
        12038 | 12044 | 12045 | 12057 | 12157 | 12175 => "ошибка защищённого соединения (сертификат или TLS); возможно, его подменяет прокси или антивирус",
        12152 => "неверный ответ сервера или прокси",
        12180 => "не удалось определить настройки прокси",
        _ => return None,
    })
}

pub fn describe(error: &io::Error) -> String {
    match error.raw_os_error() {
        Some(code) => match winhttp_message(code) {
            Some(text) => format!("{text} (код {code})"),
            None => error.to_string(),
        },
        None => error.to_string(),
    }
}

/// GET over HTTPS. WinHTTP with the system proxy settings first; if the
/// connection itself fails, PowerShell's web client (system proxy and the
/// Windows login for it), which corporate filters often let through.
#[cfg(windows)]
pub fn fetch(url: &str, progress: &mut dyn FnMut(u64, Option<u64>)) -> io::Result<Vec<u8>> {
    match winhttp_get(url, progress) {
        Ok(body) => Ok(body),
        // The server answered: another route would get the same answer.
        Err(error) if error.kind() == io::ErrorKind::InvalidData => Err(error),
        Err(first) => match powershell_get(url) {
            Ok(body) => {
                progress(body.len() as u64, Some(body.len() as u64));
                Ok(body)
            }
            Err(second) => Err(io::Error::other(format!(
                "{}. Через PowerShell тоже не вышло: {second}", describe(&first)))),
        },
    }
}

#[cfg(windows)]
fn winhttp_get(url: &str, progress: &mut dyn FnMut(u64, Option<u64>)) -> io::Result<Vec<u8>> {
    use windows_sys::Win32::Networking::WinHttp::*;
    struct Handle(*mut core::ffi::c_void);
    impl Drop for Handle {
        fn drop(&mut self) { if !self.0.is_null() { unsafe { WinHttpCloseHandle(self.0); } } }
    }
    fn wide(text: &str) -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() }
    fn check(handle: *mut core::ffi::c_void) -> io::Result<Handle> {
        if handle.is_null() { Err(io::Error::last_os_error()) } else { Ok(Handle(handle)) }
    }
    let rest = url.strip_prefix("https://").ok_or_else(|| io::Error::other("поддерживается только https"))?;
    let (host, path) = rest.split_once('/').map_or((rest, "/".to_string()), |(host, path)| (host, format!("/{path}")));
    unsafe {
        let agent = wide(&format!("Papka/{}", env!("CARGO_PKG_VERSION")));
        let session = check(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, std::ptr::null(), std::ptr::null(), 0))?;
        WinHttpSetTimeouts(session.0, 15_000, 15_000, 30_000, 60_000);
        // TLS 1.2 is not on by default in older Windows builds; 1.3 where known.
        for protocols in [WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2] {
            if WinHttpSetOption(session.0, WINHTTP_OPTION_SECURE_PROTOCOLS, (&protocols as *const u32).cast(), 4) != 0 { break; }
        }
        let connection = check(WinHttpConnect(session.0, wide(host).as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0))?;
        let verb = wide("GET");
        let request = check(WinHttpOpenRequest(connection.0, verb.as_ptr(), wide(&path).as_ptr(), std::ptr::null(),
            std::ptr::null(), std::ptr::null(), WINHTTP_FLAG_SECURE))?;
        // A proxy asking for the Windows login gets it, as in the browser.
        let policy = WINHTTP_AUTOLOGON_SECURITY_LEVEL_LOW;
        WinHttpSetOption(request.0, WINHTTP_OPTION_AUTOLOGON_POLICY, (&policy as *const u32).cast(), 4);
        let headers = wide("Accept: application/vnd.github+json, application/octet-stream\r\n");
        if WinHttpSendRequest(request.0, headers.as_ptr(), u32::MAX, std::ptr::null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0 {
            return Err(io::Error::last_os_error());
        }
        let number = |query: u32| {
            let mut value = 0u32;
            let mut size = 4u32;
            (WinHttpQueryHeaders(request.0, query | WINHTTP_QUERY_FLAG_NUMBER, std::ptr::null(),
                (&mut value as *mut u32).cast(), &mut size, std::ptr::null_mut()) != 0).then_some(value)
        };
        let status = number(WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
        match status {
            200 => {}
            // The proxy refused the login: worth the other route.
            407 => return Err(io::Error::new(io::ErrorKind::PermissionDenied, "прокси требует входа (407)")),
            403 | 429 => return Err(io::Error::new(io::ErrorKind::InvalidData, "GitHub временно ограничил запросы, попробуйте через час")),
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("сервер ответил {status}"))),
        }
        let total = number(WINHTTP_QUERY_CONTENT_LENGTH).map(u64::from);
        if total.is_some_and(|total| total > MAX_DOWNLOAD) { return Err(too_large()); }
        let mut body = Vec::with_capacity(total.unwrap_or(0).min(64 << 20) as usize);
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            if WinHttpReadData(request.0, buffer.as_mut_ptr().cast(), buffer.len() as u32, &mut read) == 0 {
                return Err(io::Error::last_os_error());
            }
            if read == 0 { break; }
            body.extend_from_slice(&buffer[..read as usize]);
            if body.len() as u64 > MAX_DOWNLOAD { return Err(too_large()); }
            progress(body.len() as u64, total);
        }
        Ok(body)
    }
}

/// The PowerShell script that downloads `url` into `target`. A failure
/// leaves only its innermost message, in UTF-8, in `target` + ".err": the
/// console's own encoding garbles Russian text and adds a stack of noise.
pub fn powershell_script(url: &str, target: &Path) -> String {
    let quote = |text: &str| format!("'{}'", text.replace('\'', "''"));
    let error_file = format!("{}.err", target.to_string_lossy());
    format!("$ErrorActionPreference='Stop'; try {{ \
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12; \
        $c = New-Object Net.WebClient; \
        $c.Headers.Add('User-Agent', 'Papka'); \
        $c.Headers.Add('Accept', 'application/vnd.github+json, application/octet-stream'); \
        $c.Proxy = [Net.WebRequest]::GetSystemWebProxy(); \
        $c.Proxy.Credentials = [Net.CredentialCache]::DefaultNetworkCredentials; \
        $c.DownloadFile({}, {}) }} catch {{ \
        $e = $_.Exception; while ($e.InnerException) {{ $e = $e.InnerException }}; \
        [IO.File]::WriteAllText({}, $e.Message, [Text.Encoding]::UTF8); exit 1 }}",
        quote(url), quote(&target.to_string_lossy()), quote(&error_file))
}

/// `-EncodedCommand` text: Base64 of UTF-16LE, so no quoting can break it.
pub fn encode_command(script: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() { out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char) } else { out.push('=') }
        }
    }
    out
}

#[cfg(windows)]
fn powershell_get(url: &str) -> io::Result<Vec<u8>> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let target = std::env::temp_dir().join(format!("papka-download-{}-{}.tmp", std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand",
            &encode_command(&powershell_script(url, &target))])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let result = match output {
        Ok(output) if output.status.success() => match fs::metadata(&target) {
            Ok(metadata) if metadata.len() > MAX_DOWNLOAD => Err(too_large()),
            _ => fs::read(&target),
        },
        Ok(output) => {
            let error_file = PathBuf::from(format!("{}.err", target.to_string_lossy()));
            let text = fs::read_to_string(&error_file).map(|text| text.trim_start_matches('\u{feff}').trim().to_string())
                .unwrap_or_else(|_| clixml_text(&String::from_utf8_lossy(&output.stderr)));
            let _ = fs::remove_file(&error_file);
            Err(io::Error::other(short_error(&text)))
        }
        Err(error) => Err(error),
    };
    let _ = fs::remove_file(&target);
    result
}

/// Development builds elsewhere use curl; updates themselves are Windows-only.
#[cfg(not(windows))]
pub fn fetch(url: &str, progress: &mut dyn FnMut(u64, Option<u64>)) -> io::Result<Vec<u8>> {
    let output = std::process::Command::new("curl").args(["-fsSL", "-H", "User-Agent: Papka", url]).output()?;
    if !output.status.success() {
        return Err(io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_string()));
    }
    if output.stdout.len() as u64 > MAX_DOWNLOAD { return Err(too_large()); }
    progress(output.stdout.len() as u64, Some(output.stdout.len() as u64));
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"[
        {"tag_name":"v0.1.46","html_url":"h46","draft":true,"prerelease":false,
         "assets":[{"name":"Papka.exe","browser_download_url":"e46"},{"name":"latest.json","browser_download_url":"m46"}]},
        {"tag_name":"v0.1.45","html_url":"h45","draft":false,"prerelease":true,
         "assets":[{"name":"Papka.exe","browser_download_url":"e45"},{"name":"latest.json","browser_download_url":"m45"}]},
        {"tag_name":"v0.1.44","html_url":"h44","draft":false,"prerelease":false,
         "assets":[{"name":"Papka.exe","browser_download_url":"e44"},{"name":"latest.json","browser_download_url":"m44"}]},
        {"tag_name":"v0.1.50","html_url":"h50","draft":false,"prerelease":false,
         "assets":[{"name":"Papka-win64.zip","browser_download_url":"z50"}]}
    ]"#;

    #[test]
    fn powershell_route_is_encoded_safely() {
        // "abc" in UTF-16LE is 61 00 62 00 63 00.
        assert_eq!(encode_command("abc"), "YQBiAGMA");
        assert_eq!(encode_command("a"), "YQA=");
        let script = powershell_script("https://x/y?a='b'", Path::new(r"C:\Temp\O'Neil.tmp"));
        assert!(script.contains(r"'https://x/y?a=''b'''"));
        assert!(script.contains(r"'C:\Temp\O''Neil.tmp'"));
    }

    #[test]
    fn winhttp_codes_read_in_russian() {
        assert!(winhttp_message(12029).unwrap().contains("подключиться"));
        assert!(describe(&io::Error::from_raw_os_error(12029)).ends_with("(код 12029)"));
        assert_eq!(winhttp_message(5), None);
    }

    #[test]
    fn errors_are_one_short_line() {
        assert_eq!(short_error("  Невозможно соединиться\r\n с удаленным сервером. "), "Невозможно соединиться с удаленным сервером");
        assert!(short_error(&"а".repeat(400)).chars().count() <= 161);
        assert_eq!(short_error(""), "неизвестная ошибка");
        assert!(powershell_script("https://x", Path::new(r"C:\T\a.tmp")).contains(r"'C:\T\a.tmp.err'"));
    }

    #[test]
    fn clixml_errors_become_plain_text() {
        let raw = "#< CLIXML\r\n<Objs Version=\"1.1.0.1\"><S S=\"Error\">Exception calling \"DownloadFile\": _x000D__x000A_</S><S S=\"Error\">Unable to connect &lt;remote&gt;_x000D__x000A_</S></Objs>";
        assert_eq!(clixml_text(raw), "Exception calling \"DownloadFile\": Unable to connect <remote>");
        assert_eq!(clixml_text("  plain error \n"), "plain error");
    }

    #[test]
    fn newest_downloaded_papka_is_found() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["Papka.exe", "Papka (1).exe", "papka (2).exe", "Other.exe", "Papka.zip"] {
            fs::write(dir.path().join(name), b"MZ").unwrap();
        }
        let version_of = |path: &Path| Some(match path.file_name()?.to_str()? {
            "Papka.exe" => "0.1.45", "Papka (1).exe" => "0.1.48", "papka (2).exe" => "0.1.47", _ => "9.9.9",
        }.to_string());
        let (path, version) = find_downloaded(dir.path(), "0.1.46", &version_of).unwrap();
        assert_eq!((path.file_name().unwrap().to_str().unwrap(), version.as_str()), ("Papka (1).exe", "0.1.48"));
        assert!(find_downloaded(dir.path(), "0.1.48", &version_of).is_none());
        let update = downloaded_update(&path, &version).unwrap();
        assert_eq!(update.sha256, sha256_hex(b"MZ"));
    }

    #[test]
    fn browser_links_follow_the_channel() {
        assert!(browser_url(Channel::Stable).ends_with("/releases/latest/download/Papka.exe"));
        assert!(browser_url(Channel::Beta).ends_with("/releases/download/beta/Papka.exe"));
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(compare_versions("0.1.10", "0.1.9"), Ordering::Greater);
        assert_eq!(compare_versions("v0.1.44", "0.1.44"), Ordering::Equal);
        assert_eq!(compare_versions("0.2", "0.1.99"), Ordering::Greater);
        assert_eq!(compare_versions("0.1.0-dev", "0.1.1"), Ordering::Less);
    }

    #[test]
    fn channel_picks_newest_complete_release() {
        let beta = pick_release(LIST.as_bytes(), Channel::Beta).unwrap().unwrap();
        assert_eq!((beta.version.as_str(), beta.exe_url.as_str(), beta.prerelease), ("0.1.45", "e45", true));
        let stable = pick_release(LIST.as_bytes(), Channel::Stable).unwrap().unwrap();
        assert_eq!((stable.version.as_str(), stable.manifest_url.as_str()), ("0.1.44", "m44"));
        assert!(pick_release(b"[]", Channel::Beta).unwrap().is_none());
        // The fixed "beta" release is a download link, never an update.
        let fixed = br#"[{"tag_name":"beta","draft":false,"prerelease":true,"html_url":"p","assets":[
            {"name":"Papka.exe","browser_download_url":"e"},{"name":"latest.json","browser_download_url":"m"}]}]"#;
        assert!(pick_release(fixed, Channel::Beta).unwrap().is_none());
    }

    fn stub(manifest: &'static str) -> impl Fn(&str, &mut dyn FnMut(u64, Option<u64>)) -> io::Result<Vec<u8>> {
        move |url, _| Ok(if url == releases_url() { LIST.as_bytes().to_vec() } else { manifest.as_bytes().to_vec() })
    }

    #[test]
    fn check_reads_the_manifest_of_a_newer_release() {
        let sha = "a".repeat(64);
        let manifest: &'static str = Box::leak(format!(r#"{{"version":"0.1.45","commit":"abc","sha256":"{sha}","notes":["Поиск"]}}"#).into_boxed_str());
        let found = check("0.1.44", Channel::Beta, &stub(manifest)).unwrap().unwrap();
        assert_eq!((found.version.as_str(), found.notes.as_slice(), found.exe_url.as_str()), ("0.1.45", &["Поиск".to_string()][..], "e45"));
        assert!(check("0.1.45", Channel::Beta, &stub(manifest)).unwrap().is_none());
        assert!(check("0.1.43", Channel::Stable, &stub(manifest)).is_err(), "manifest of another version is refused");
    }

    /// A release list and manifest for the file `bytes` published as 0.1.48.
    fn published(bytes: &[u8]) -> impl Fn(&str, &mut dyn FnMut(u64, Option<u64>)) -> io::Result<Vec<u8>> {
        let manifest = format!(r#"{{"version":"0.1.48","commit":"abc","sha256":"{}","notes":["Очередь"]}}"#, sha256_hex(bytes));
        move |url, _| if url == "m48" { Ok(manifest.clone().into_bytes()) } else { Err(io::Error::other("нет сети")) }
    }
    const LIST_48: &str = r#"[{"tag_name":"v0.1.48","html_url":"h48","draft":false,"prerelease":true,
        "assets":[{"name":"Papka.exe","browser_download_url":"e48"},{"name":"latest.json","browser_download_url":"m48"}]}]"#;

    #[test]
    fn a_downloaded_file_is_compared_with_its_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Papka.exe");
        fs::write(&path, b"MZ genuine").unwrap();
        let found = downloaded_update(&path, "0.1.48").unwrap();
        assert_eq!(found.origin, Origin::DownloadedUnverified);

        let verified = vet_downloaded(found.clone(), Some(LIST_48.as_bytes()), &published(b"MZ genuine")).unwrap();
        assert_eq!((verified.origin, verified.notes.as_slice(), verified.prerelease),
            (Origin::DownloadedVerified, &["Очередь".to_string()][..], true));

        let other = vet_downloaded(found.clone(), Some(LIST_48.as_bytes()), &published(b"MZ another build"));
        assert_eq!(other.unwrap_err().kind(), io::ErrorKind::InvalidData, "another file under the same version");

        let mut invented = found.clone();
        invented.version = "9.9.9".into();
        assert!(vet_downloaded(invented, Some(LIST_48.as_bytes()), &published(b"MZ genuine")).is_err(), "no such release");

        // No network at all, or the manifest cannot be read: offered, but marked.
        assert_eq!(vet_downloaded(found.clone(), None, &published(b"MZ genuine")).unwrap().origin, Origin::DownloadedUnverified);
        let offline = |_: &str, _: &mut dyn FnMut(u64, Option<u64>)| -> io::Result<Vec<u8>> { Err(io::Error::other("нет сети")) };
        assert_eq!(vet_downloaded(found, Some(LIST_48.as_bytes()), &offline).unwrap().origin, Origin::DownloadedUnverified);
    }

    #[test]
    fn a_refused_file_does_not_hide_a_genuine_one() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Papka-fake.exe"), b"MZ fake").unwrap();
        fs::write(dir.path().join("Papka.exe"), b"MZ genuine").unwrap();
        let version_of = |path: &Path| Some(if path.ends_with("Papka-fake.exe") { "9.9.9" } else { "0.1.48" }.to_string());
        let candidates = downloaded_candidates(dir.path(), "0.1.46", &version_of);
        assert_eq!(candidates.iter().map(|(_, version)| version.as_str()).collect::<Vec<_>>(), ["9.9.9", "0.1.48"]);

        let none = HashSet::new();
        let found = downloaded_offer(dir.path(), "0.1.46", &version_of, Some(LIST_48.as_bytes()), &published(b"MZ genuine"), &none);
        let offer = found.offer.unwrap().unwrap();
        assert_eq!((offer.version.as_str(), offer.origin), ("0.1.48", Origin::DownloadedVerified));
        assert_eq!(found.refused.len(), 1, "the invented version is remembered");

        fs::remove_file(dir.path().join("Papka.exe")).unwrap();
        let refused = downloaded_offer(dir.path(), "0.1.46", &version_of, Some(LIST_48.as_bytes()), &published(b"MZ genuine"), &none);
        assert!(refused.offer.unwrap().is_err(), "only the invented version is left");
        assert!(downloaded_offer(dir.path(), "9.9.9", &version_of, None, &published(b""), &none).offer.is_none());

        // Once refused, the file is not looked at again until it changes.
        let known: HashSet<FileStamp> = refused.refused.into_iter().collect();
        assert!(fresh_candidates(dir.path(), "0.1.46", &version_of, &known).is_empty());
        assert!(downloaded_offer(dir.path(), "0.1.46", &version_of, None, &published(b""), &known).offer.is_none());
        fs::write(dir.path().join("Papka-fake.exe"), b"MZ fake, downloaded again").unwrap();
        assert_eq!(fresh_candidates(dir.path(), "0.1.46", &version_of, &known).len(), 1);
        // Offline the file cannot be refused: nothing is remembered.
        let offline = downloaded_offer(dir.path(), "0.1.46", &version_of, None, &published(b""), &none);
        assert!(offline.refused.is_empty());
    }

    #[test]
    fn install_swaps_files_and_rollback_swaps_back() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("Papka.exe");
        fs::write(&exe, b"MZ old").unwrap();
        let new = b"MZ new".to_vec();
        assert!(install(&exe, &new, &"0".repeat(64)).is_err());
        assert!(install(&exe, b"not an exe", &sha256_hex(b"not an exe")).is_err());
        assert_eq!(fs::read(&exe).unwrap(), b"MZ old");
        install(&exe, &new, &sha256_hex(&new).to_uppercase()).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"MZ new");
        assert_eq!(fs::read(previous_path(&exe)).unwrap(), b"MZ old");
        rollback(&exe).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"MZ old");
        assert_eq!(fs::read(previous_path(&exe)).unwrap(), b"MZ new");
        assert!(!staged_path(&exe).exists());
    }
}
