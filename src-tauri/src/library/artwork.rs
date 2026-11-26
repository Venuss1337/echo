//! Bounded decoding and content-addressed thumbnail cache for embedded covers.
//!
//! Embedded pictures are attacker-controlled bytes. A decompression bomb is the
//! main risk, so dimensions are read from the header and checked *before* any
//! pixel buffer is allocated, the decoder also runs under an allocation limit,
//! and a process-wide gate allows a single decode at a time. Peak memory is
//! therefore one image of at most [`MAX_PIXELS`], regardless of scan concurrency.

use image::{ImageFormat, ImageReader, Limits, codecs::jpeg::JpegEncoder};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Largest embedded picture considered at all.
pub(crate) const MAX_PICTURE_BYTES: usize = 16 * 1024 * 1024;
/// Largest decoded picture area: 16 megapixels is 48 MiB as RGB.
pub(crate) const MAX_PIXELS: u64 = 16_000_000;
const MAX_SIDE: u32 = 8192;
/// Longest edge of a cached thumbnail. Enough for a 2x list row and a grid card.
pub(crate) const THUMB_EDGE: u32 = 320;
/// Longest edge of the variant used by full-size sleeves.
pub(crate) const LARGE_EDGE: u32 = 1024;
pub(crate) const MAX_LARGE_BYTES: u64 = 2 * 1024 * 1024;
/// Large variants are regenerable, so the cache keeps only the newest.
const LARGE_CACHE_BYTES: u64 = 192 * 1024 * 1024;
/// Cached thumbnails are far below this; the cap bounds an IPC response.
pub(crate) const MAX_THUMB_BYTES: u64 = 512 * 1024;

static DECODE_GATE: Mutex<()> = Mutex::new(());

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ArtworkError {
    TooLarge,
    Unsupported,
    Malformed,
    Oversized,
}

pub(crate) struct Thumbnail {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Lowercase hex SHA-256 of the original embedded bytes.
pub(crate) fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

pub(crate) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Decodes a JPEG or PNG within the memory bounds and returns a small JPEG.
pub(crate) fn make_thumbnail(bytes: &[u8]) -> Result<Thumbnail, ArtworkError> {
    make_variant(bytes, THUMB_EDGE)
}

/// Never enlarges: a source smaller than `edge` keeps its own size.
pub(crate) fn make_variant(bytes: &[u8], edge: u32) -> Result<Thumbnail, ArtworkError> {
    make_bounded_variant(bytes, edge, MAX_SIDE, MAX_PIXELS)
}

/// Uses the shared raster decoder with a smaller pixel budget for remote artwork.
#[cfg(windows)]
pub(crate) fn make_station_thumbnail(bytes: &[u8]) -> Result<Thumbnail, ArtworkError> {
    make_bounded_variant(bytes, THUMB_EDGE, 2048, 2_000_000)
}

fn make_bounded_variant(
    bytes: &[u8],
    edge: u32,
    max_side: u32,
    max_pixels: u64,
) -> Result<Thumbnail, ArtworkError> {
    if bytes.len() > MAX_PICTURE_BYTES {
        return Err(ArtworkError::TooLarge);
    }
    let open = || {
        ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| ArtworkError::Malformed)
    };
    let probe = open()?;
    if !matches!(probe.format(), Some(ImageFormat::Jpeg | ImageFormat::Png)) {
        return Err(ArtworkError::Unsupported);
    }
    let (width, height) = probe
        .into_dimensions()
        .map_err(|_| ArtworkError::Malformed)?;
    if width == 0
        || height == 0
        || width > max_side
        || height > max_side
        || u64::from(width) * u64::from(height) > max_pixels
    {
        return Err(ArtworkError::Oversized);
    }
    // A poisoned gate only means another decode panicked; the guard protects no data.
    let _gate = DECODE_GATE.lock().unwrap_or_else(|p| p.into_inner());
    let mut reader = open()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(max_side);
    limits.max_image_height = Some(max_side);
    limits.max_alloc = Some(max_pixels * 10);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| ArtworkError::Malformed)?;
    let thumb = if width.max(height) <= edge {
        decoded.to_rgb8()
    } else {
        decoded.thumbnail(edge, edge).to_rgb8()
    };
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 86)
        .encode_image(&thumb)
        .map_err(|_| ArtworkError::Malformed)?;
    Ok(Thumbnail {
        jpeg,
        width: thumb.width(),
        height: thumb.height(),
    })
}

/// `<dir>/<first two hex>/<hash>.jpg`. Callers must pass a validated hash so a
/// database value can never name a path outside the cache.
pub(crate) fn cache_path(dir: &Path, hash: &str) -> Option<PathBuf> {
    valid_hash(hash).then(|| dir.join(&hash[..2]).join(format!("{hash}.jpg")))
}

/// Writes through a temporary name and renames, so a reader or a crash never
/// observes a partial image. Existing identical content is left in place.
pub(crate) fn store(dir: &Path, hash: &str, thumbnail: &Thumbnail) -> io::Result<()> {
    let path = cache_path(dir, hash).ok_or(io::ErrorKind::InvalidInput)?;
    if path.is_file() {
        return Ok(());
    }
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!("{hash}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(&thumbnail.jpeg)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
        // A concurrent writer of the same content may have won the rename.
        if path.is_file() {
            return Ok(());
        }
    }
    result
}

/// Reads a cached thumbnail, never more than [`MAX_THUMB_BYTES`].
pub(crate) fn load(dir: &Path, hash: &str) -> io::Result<Vec<u8>> {
    read_bounded(
        &cache_path(dir, hash).ok_or(io::ErrorKind::InvalidInput)?,
        MAX_THUMB_BYTES,
    )
}

fn read_bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}

pub(crate) fn remove(dir: &Path, hash: &str) {
    for root in [dir.to_path_buf(), large_dir(dir)] {
        if let Some(path) = cache_path(&root, hash) {
            let _ = fs::remove_file(path);
        }
    }
}

pub(crate) fn large_dir(dir: &Path) -> PathBuf {
    dir.join("large")
}

pub(crate) fn load_large(dir: &Path, hash: &str) -> io::Result<Vec<u8>> {
    read_bounded(
        &cache_path(&large_dir(dir), hash).ok_or(io::ErrorKind::InvalidInput)?,
        MAX_LARGE_BYTES,
    )
}

/// Stores a large variant and then drops the oldest ones above the budget.
pub(crate) fn store_large(dir: &Path, hash: &str, image: &Thumbnail) -> io::Result<()> {
    let root = large_dir(dir);
    store(&root, hash, image)?;
    let mut files = Vec::new();
    for shard in fs::read_dir(&root)?.flatten() {
        for file in fs::read_dir(shard.path()).into_iter().flatten().flatten() {
            if let Ok(metadata) = file.metadata() {
                files.push((metadata.modified().ok(), metadata.len(), file.path()));
            }
        }
    }
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    files.sort();
    let keep = cache_path(&root, hash);
    for (_, size, path) in files {
        if total <= LARGE_CACHE_BYTES {
            break;
        }
        if Some(&path) != keep.as_ref() && fs::remove_file(&path).is_ok() {
            total -= size;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod fixtures {
    use image::{ImageBuffer, ImageEncoder, Rgb};

    /// A deterministic gradient so different seeds hash differently.
    pub(crate) fn png(width: u32, height: u32, seed: u8) -> Vec<u8> {
        let buffer = ImageBuffer::from_fn(width, height, |x, y| {
            Rgb([
                (x % 251) as u8 ^ seed,
                (y % 241) as u8,
                seed.wrapping_mul(7),
            ])
        });
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(
                buffer.as_raw(),
                width,
                height,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_bounded_deterministic_thumbnail() {
        let source = fixtures::png(1000, 500, 3);
        let first = make_thumbnail(&source).unwrap();
        let second = make_thumbnail(&source).unwrap();
        assert_eq!((first.width, first.height), (320, 160));
        assert_eq!(first.jpeg, second.jpeg);
        assert!(first.jpeg.len() < MAX_THUMB_BYTES as usize);
        assert_eq!(&first.jpeg[..2], &[0xff, 0xd8]);
    }

    #[test]
    fn rejects_malformed_unsupported_and_oversized_pictures() {
        assert_eq!(
            make_thumbnail(b"not an image at all").err(),
            Some(ArtworkError::Unsupported)
        );
        assert_eq!(
            make_thumbnail(b"GIF89a\x01\x00\x01\x00\x00\x00\x00;").err(),
            Some(ArtworkError::Unsupported)
        );
        let mut truncated = fixtures::png(64, 64, 1);
        truncated.truncate(truncated.len() / 2);
        assert_eq!(
            make_thumbnail(&truncated).err(),
            Some(ArtworkError::Malformed)
        );
        // A solid 9000x1 strip is tiny on disk but exceeds the side limit.
        let strip = fixtures::png(9000, 1, 0);
        assert_eq!(make_thumbnail(&strip).err(), Some(ArtworkError::Oversized));
        // 5000x5000 compresses to a small file yet needs 25 megapixels.
        let bomb = fixtures::png(5000, 5000, 0);
        assert_eq!(make_thumbnail(&bomb).err(), Some(ArtworkError::Oversized));
        let huge = vec![0_u8; MAX_PICTURE_BYTES + 1];
        assert_eq!(make_thumbnail(&huge).err(), Some(ArtworkError::TooLarge));
    }

    #[test]
    fn cache_is_content_addressed_atomic_and_path_safe() {
        let dir = tempfile::tempdir().unwrap();
        let thumb = make_thumbnail(&fixtures::png(40, 40, 9)).unwrap();
        let hash = content_hash(b"original bytes");
        store(dir.path(), &hash, &thumb).unwrap();
        store(dir.path(), &hash, &thumb).unwrap();
        assert_eq!(load(dir.path(), &hash).unwrap(), thumb.jpeg);
        assert!(cache_path(dir.path(), "../escape").is_none());
        assert!(cache_path(dir.path(), &"G".repeat(64)).is_none());
        assert!(load(dir.path(), "..\\..\\x").is_err());
        let leftovers: Vec<_> = fs::read_dir(dir.path().join(&hash[..2]))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1);
        remove(dir.path(), &hash);
        assert!(load(dir.path(), &hash).is_err());
    }
}
