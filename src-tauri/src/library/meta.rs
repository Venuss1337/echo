//! Tag and property extraction with Lofty from an already-authorized handle.
//!
//! The reader never receives a path, so it can only read the bytes the scanner
//! chose to open. Parsing runs with Lofty's best-attempt mode, a small
//! per-tag-item allocation limit and cleaned, bounded output strings. A file
//! whose container cannot be identified is an error, not a track.

use super::{
    artwork::MAX_PICTURE_BYTES,
    text::{clean, key},
};
use lofty::{
    config::{GlobalOptions, ParseOptions, ParsingMode, apply_global_options},
    file::{AudioFile, FileType, TaggedFileExt},
    picture::PictureType,
    probe::Probe,
    tag::{Accessor, ItemKey},
};
use std::io::{BufRead, Seek};

/// Bump to make the next scan re-read every file after extraction changes.
/// 2: folder-scoped album keys became absolute, so equal relative folders in
/// two roots no longer share an album.
pub(crate) const METADATA_VERSION: i64 = 2;
const MAX_ARTISTS: usize = 8;
/// Longer than any real recording; protects arithmetic and display.
const MAX_DURATION_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// Lofty rejects any single tag item above this many bytes.
const ITEM_ALLOCATION_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Debug, Default, Clone)]
pub(crate) struct TrackMeta {
    pub title: String,
    pub artists: Vec<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<u32>,
    pub genre: String,
    pub duration_ms: u64,
    pub codec: String,
    /// Original embedded picture bytes, at most [`MAX_PICTURE_BYTES`].
    pub picture: Option<Vec<u8>>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadError {
    /// The container was not recognized or its structure was unusable.
    Unrecognized,
    /// Reading the handle failed; a later scan may succeed.
    Io,
}

/// Lofty's allocation limit is per thread. Call once on every worker.
pub(crate) fn configure_thread() {
    apply_global_options(GlobalOptions::new().allocation_limit(ITEM_ALLOCATION_LIMIT));
}

fn bounded(value: Option<u32>, max: u32) -> Option<u32> {
    value.filter(|number| (1..=max).contains(number))
}

fn codec(file_type: FileType) -> &'static str {
    match file_type {
        FileType::Mpeg => "MP3",
        FileType::Flac => "FLAC",
        FileType::Wav => "WAV",
        FileType::Vorbis => "Ogg Vorbis",
        FileType::Opus => "Opus",
        FileType::Mp4 => "M4A",
        FileType::Aac => "AAC",
        _ => "Audio",
    }
}

/// Reads metadata. `fallback_title` is the cleaned file stem used when the
/// title tag is absent or empty.
pub(crate) fn read<R: BufRead + Seek>(
    reader: R,
    fallback_title: &str,
) -> Result<TrackMeta, ReadError> {
    let options = ParseOptions::new()
        .parsing_mode(ParsingMode::BestAttempt)
        .read_properties(true)
        .read_tags(true)
        .read_cover_art(true);
    let probe = Probe::new(reader)
        .options(options)
        .guess_file_type()
        .map_err(|_| ReadError::Io)?;
    if probe.file_type().is_none() {
        return Err(ReadError::Unrecognized);
    }
    let file = probe.read().map_err(|error| {
        // Lofty hides its error kind. An I/O cause anywhere in the chain means
        // the bytes could not be read, which a later scan may retry.
        let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(&error);
        while let Some(current) = cause {
            if current.downcast_ref::<std::io::Error>().is_some() {
                return ReadError::Io;
            }
            cause = current.source();
        }
        ReadError::Unrecognized
    })?;
    let tag = file.primary_tag().or_else(|| file.first_tag());

    let mut meta = TrackMeta {
        codec: codec(file.file_type()).to_owned(),
        duration_ms: u64::try_from(file.properties().duration().as_millis())
            .unwrap_or(u64::MAX)
            .min(MAX_DURATION_MS),
        ..TrackMeta::default()
    };
    if let Some(tag) = tag {
        meta.title = tag.title().map(|v| clean(&v)).unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        let values = tag
            .get_strings(ItemKey::TrackArtist)
            .map(str::to_owned)
            .chain(tag.artist().map(|v| v.into_owned()));
        for value in values {
            let value = clean(&value);
            if !value.is_empty() && seen.insert(key(&value)) {
                meta.artists.push(value);
            }
            if meta.artists.len() >= MAX_ARTISTS {
                break;
            }
        }
        meta.album_artist = tag
            .get_string(ItemKey::AlbumArtist)
            .map(clean)
            .filter(|v| !v.is_empty());
        meta.album = tag.album().map(|v| clean(&v)).filter(|v| !v.is_empty());
        meta.track_no = bounded(tag.track(), 9999);
        meta.disc_no = bounded(tag.disk(), 999);
        meta.year = tag
            .date()
            .map(|date| u32::from(date.year))
            .filter(|year| (1000..=2999).contains(year));
        meta.genre = tag
            .genre()
            .map(|v| clean(&v).chars().take(128).collect())
            .unwrap_or_default();
        let picture = tag
            .pictures()
            .iter()
            .find(|picture| picture.pic_type() == PictureType::CoverFront)
            .or_else(|| tag.pictures().first());
        meta.picture = picture
            .map(|picture| picture.data())
            .filter(|data| !data.is_empty() && data.len() <= MAX_PICTURE_BYTES)
            .map(<[u8]>::to_vec);
    }
    if meta.title.is_empty() {
        meta.title = if fallback_title.is_empty() {
            "Untitled".to_owned()
        } else {
            fallback_title.to_owned()
        };
    }
    Ok(meta)
}

/// Test helpers that write real tagged audio files with Lofty. The files hold
/// a short run of PCM silence so the whole decode path is exercised without
/// shipping any copyrighted audio.
#[cfg(test)]
pub(crate) mod fixtures {
    use lofty::{
        config::WriteOptions,
        picture::{MimeType, Picture, PictureType},
        prelude::*,
        tag::{Tag, TagType},
    };
    use std::path::Path;

    #[derive(Default, Clone)]
    pub(crate) struct Spec<'a> {
        pub title: Option<&'a str>,
        pub artist: Option<&'a str>,
        pub album_artist: Option<&'a str>,
        pub album: Option<&'a str>,
        pub track: Option<u32>,
        pub disc: Option<u32>,
        pub year: Option<u32>,
        pub genre: Option<&'a str>,
        pub picture: Option<Vec<u8>>,
    }

    /// 8 kHz mono 8-bit silence, 250 ms.
    pub(crate) fn silent_wav() -> Vec<u8> {
        silent_wav_of(2000)
    }

    /// The same format with `samples` samples (8000 per second).
    pub(crate) fn silent_wav_of(samples: usize) -> Vec<u8> {
        let data = vec![0x80_u8; samples];
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&8000_u32.to_le_bytes());
        wav.extend_from_slice(&8000_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&8_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        wav
    }

    pub(crate) fn write(path: &Path, spec: &Spec) {
        write_with(path, spec, &silent_wav());
    }

    pub(crate) fn write_with(path: &Path, spec: &Spec, audio: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, audio).unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        if let Some(value) = spec.title {
            tag.set_title(value.to_owned());
        }
        if let Some(value) = spec.artist {
            tag.set_artist(value.to_owned());
        }
        if let Some(value) = spec.album_artist {
            tag.insert_text(ItemKey::AlbumArtist, value.to_owned());
        }
        if let Some(value) = spec.album {
            tag.set_album(value.to_owned());
        }
        if let Some(value) = spec.track {
            tag.set_track(value);
        }
        if let Some(value) = spec.disc {
            tag.set_disk(value);
        }
        if let Some(value) = spec.year {
            tag.insert_text(ItemKey::RecordingDate, value.to_string());
        }
        if let Some(value) = spec.genre {
            tag.set_genre(value.to_owned());
        }
        if let Some(bytes) = &spec.picture {
            tag.push_picture(
                Picture::unchecked(bytes.clone())
                    .pic_type(PictureType::CoverFront)
                    .mime_type(MimeType::Png)
                    .build(),
            );
        }
        tag.save_to_path(path, WriteOptions::default()).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::{fixtures::*, *};
    use std::{fs::File, io::BufReader};

    fn read_path(path: &std::path::Path) -> Result<TrackMeta, ReadError> {
        configure_thread();
        read(BufReader::new(File::open(path).unwrap()), "fallback")
    }

    #[test]
    fn extracts_every_supported_field_from_a_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        write(
            &path,
            &Spec {
                title: Some("  Echo \u{202e}Chamber "),
                artist: Some("Nils Frahm"),
                album_artist: Some("Various Artists"),
                album: Some("Spaces"),
                track: Some(7),
                disc: Some(2),
                year: Some(2013),
                genre: Some("Ambient"),
                picture: Some(super::super::artwork::fixtures::png(8, 8, 1)),
            },
        );
        let meta = read_path(&path).unwrap();
        assert_eq!(meta.title, "Echo Chamber");
        assert_eq!(meta.artists, ["Nils Frahm"]);
        assert_eq!(meta.album_artist.as_deref(), Some("Various Artists"));
        assert_eq!(meta.album.as_deref(), Some("Spaces"));
        assert_eq!(
            (meta.track_no, meta.disc_no, meta.year),
            (Some(7), Some(2), Some(2013))
        );
        assert_eq!(meta.genre, "Ambient");
        assert_eq!(meta.codec, "WAV");
        assert_eq!(meta.duration_ms, 250);
        assert!(meta.picture.is_some());
    }

    #[test]
    fn untagged_files_fall_back_to_the_supplied_title() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bare.wav");
        std::fs::write(&path, silent_wav()).unwrap();
        let meta = read_path(&path).unwrap();
        assert_eq!(meta.title, "fallback");
        assert!(meta.artists.is_empty() && meta.album.is_none() && meta.picture.is_none());
    }

    #[test]
    fn implausible_numbers_and_empty_strings_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("odd.wav");
        write(
            &path,
            &Spec {
                title: Some("   "),
                artist: Some("\u{0}\u{0}"),
                album: Some(""),
                track: Some(0),
                year: Some(99_999),
                ..Spec::default()
            },
        );
        let meta = read_path(&path).unwrap();
        assert_eq!(meta.title, "fallback");
        assert!(meta.artists.is_empty() && meta.album.is_none());
        assert_eq!((meta.track_no, meta.year), (None, None));
    }

    #[test]
    fn corrupt_and_truncated_files_are_reported_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.mp3");
        std::fs::write(&junk, b"definitely not audio").unwrap();
        assert_eq!(read_path(&junk).err(), Some(ReadError::Unrecognized));
        let empty = dir.path().join("empty.flac");
        std::fs::write(&empty, b"").unwrap();
        assert!(read_path(&empty).is_err());
        let truncated = dir.path().join("cut.wav");
        let mut bytes = silent_wav();
        bytes.truncate(30);
        std::fs::write(&truncated, bytes).unwrap();
        // Either outcome is acceptable; panicking or hanging is not.
        let _ = read_path(&truncated);
    }

    #[test]
    fn an_enormous_tag_value_is_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.wav");
        let huge = "x".repeat(2_000_000);
        write(
            &path,
            &Spec {
                title: Some(&huge),
                album: Some(&huge),
                ..Spec::default()
            },
        );
        let meta = read_path(&path).unwrap();
        assert!(meta.title.chars().count() <= 512);
        assert!(meta.album.unwrap().chars().count() <= 512);
    }
}
