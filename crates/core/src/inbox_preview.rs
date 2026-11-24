//! Small local previews of inbox screenshots. Decode off the UI thread;
//! the original is read only for a cold cache entry, never by GPUI's img.
use std::{fs, hash::{Hash, Hasher}, io::{self, Read}, path::{Path, PathBuf}, time::SystemTime};

pub const EDGE: u32 = 280;
const MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub source: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl Key {
    pub fn read(path: &Path) -> io::Result<Self> {
        let meta = fs::metadata(path)?;
        if !meta.is_file() { return Err(io::Error::other("снимок не является файлом")); }
        Ok(Self { source: path.to_path_buf(), size: meta.len(), modified: meta.modified().ok() })
    }
}

pub fn make(key: &Key, cache: &Path) -> io::Result<PathBuf> {
    crate::guard::catch("миниатюра входящих", || make_inner(key, cache))?
}

fn make_inner(key: &Key, cache: &Path) -> io::Result<PathBuf> {
    if key.size > MAX_BYTES { return Err(io::Error::other("снимок слишком велик для миниатюры")); }
    // The inbox already read this source stamp. A warm cache needs no
    // further round trips to the screenshot's network share.
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hash);
    EDGE.hash(&mut hash);
    let target = cache.join(format!("{:016x}.png", hash.finish()));
    if crate::thumbnail::validate_cached(&target, EDGE).is_ok() { return Ok(target); }
    if Key::read(&key.source)? != *key { return Err(io::Error::other("снимок изменился")); }
    crate::folder_copy::reject_link(&key.source)?;
    let mut bytes = Vec::new();
    fs::File::open(&key.source)?.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES { return Err(io::Error::other("снимок слишком велик для миниатюры")); }
    let mut reader = image::ImageReader::new(io::Cursor::new(bytes)).with_guessed_format().map_err(io::Error::other)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let picture = reader.decode().map_err(io::Error::other)?.thumbnail(EDGE, EDGE);
    if Key::read(&key.source)? != *key { return Err(io::Error::other("снимок изменился")); }
    fs::create_dir_all(cache)?;
    let temporary = tempfile::NamedTempFile::new_in(cache)?;
    picture.save_with_format(temporary.path(), image::ImageFormat::Png).map_err(io::Error::other)?;
    crate::thumbnail::validate_cached(temporary.path(), EDGE)?;
    temporary.persist(&target).map_err(|error| error.error)?;
    // Cleanup failure must not hide a complete, usable preview.
    let _ = crate::thumbnail::prune_cache(cache, &target);
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_is_supported_and_a_broken_cache_is_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("screen.jpg");
        let cache = dir.path().join("cache");
        image::RgbImage::from_pixel(600, 300, image::Rgb([30, 70, 90])).save(&source).unwrap();
        let key = Key::read(&source).unwrap();
        let preview = make(&key, &cache).unwrap();
        fs::write(&preview, b"broken cache").unwrap();
        assert_eq!(make(&key, &cache).unwrap(), preview);
        assert_eq!(image::image_dimensions(&preview).unwrap(), (280, 140));
    }

    #[test]
    fn oversized_source_is_refused_before_allocating_or_making_cache_files() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("huge.png");
        fs::File::create(&source).unwrap().set_len(MAX_BYTES + 1).unwrap();
        let cache = dir.path().join("cache");
        assert!(make(&Key::read(&source).unwrap(), &cache).is_err());
        assert!(!cache.exists());
    }

    #[test]
    fn large_screenshot_is_small_and_warm_cache_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("screen.png");
        let cache = dir.path().join("cache");
        image::RgbaImage::from_pixel(3840, 2160, image::Rgba([20, 50, 90, 255])).save(&source).unwrap();
        let key = Key::read(&source).unwrap();
        let start = std::time::Instant::now();
        let preview = make(&key, &cache).unwrap();
        let cold = start.elapsed();
        let picture = image::open(&preview).unwrap();
        assert_eq!((picture.width(), picture.height()), (280, 158));
        assert_eq!(image::image_dimensions(&source).unwrap(), (3840, 2160));
        let stamp = fs::metadata(&preview).unwrap().modified().unwrap();
        let start = std::time::Instant::now();
        assert_eq!(make(&key, &cache).unwrap(), preview);
        eprintln!("4K PNG preview: cold {:?}, warm {:?}", cold, start.elapsed());
        assert_eq!(fs::metadata(&preview).unwrap().modified().unwrap(), stamp);
        assert!(fs::metadata(&preview).unwrap().len() < 1024 * 1024);
    }

    #[test]
    fn changed_and_broken_images_are_not_published_under_an_old_key() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("screen.png");
        let cache = dir.path().join("cache");
        image::RgbaImage::new(20, 10).save(&source).unwrap();
        let old = Key::read(&source).unwrap();
        let first = make(&old, &cache).unwrap();
        image::RgbaImage::new(100, 30).save(&source).unwrap();
        // A cached old revision is still that revision, never new pixels
        // published under the old identity. A stale cold task is refused.
        assert_eq!(make(&old, &cache).unwrap(), first);
        fs::remove_file(&first).unwrap();
        assert!(make(&old, &cache).is_err());
        let second = make(&Key::read(&source).unwrap(), &cache).unwrap();
        assert_ne!(first, second);
        fs::write(&source, b"not an image").unwrap();
        assert!(make(&Key::read(&source).unwrap(), &cache).is_err());
        assert_eq!(fs::read_dir(cache).unwrap().count(), 1);
    }
}
