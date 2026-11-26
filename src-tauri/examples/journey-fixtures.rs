//! Writes the original tagged music used by the desktop journey test: two
//! albums of twelve synthesized tones with drawn covers. Development only.

use lofty::{
    config::WriteOptions,
    picture::{MimeType, Picture, PictureType},
    prelude::*,
    tag::{Tag, TagType},
};
use std::path::{Path, PathBuf};

/// Sixteen seconds of a quiet sine, so half of it passes the history threshold.
fn tone(frequency: f32) -> Vec<u8> {
    const RATE: u32 = 22_050;
    let samples: Vec<u8> = (0..RATE * 16)
        .flat_map(|n| {
            let phase = n as f32 / RATE as f32 * frequency * std::f32::consts::TAU;
            ((phase.sin() * 900.0) as i16).to_le_bytes()
        })
        .collect();
    let mut wav = Vec::with_capacity(samples.len() + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    for half in [1_u16, 1] {
        wav.extend_from_slice(&half.to_le_bytes());
    }
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    for half in [2_u16, 16] {
        wav.extend_from_slice(&half.to_le_bytes());
    }
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    wav.extend_from_slice(&samples);
    wav
}

fn cover(warm: bool) -> Vec<u8> {
    let size = 900_u32;
    let image = image::RgbImage::from_fn(size, size, |x, y| {
        let (u, v) = (x as f32 / size as f32, y as f32 / size as f32);
        let ring = (((u - 0.5).powi(2) + (v - 0.42).powi(2)).sqrt() * 46.0).sin();
        let light = (1.0 - v * 0.75 + ring * 0.16).clamp(0.0, 1.0);
        let base = if warm {
            [232.0, 112.0, 58.0]
        } else {
            [52.0, 148.0, 176.0]
        };
        image::Rgb(base.map(|channel| (channel * light) as u8))
    });
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    png.into_inner()
}

fn album(root: &Path, title: &str, artist: &str, song: &str, warm: bool) {
    let directory = root.join(artist).join(title);
    std::fs::create_dir_all(&directory).unwrap();
    let art = cover(warm);
    for track in 1..=12_u32 {
        let path = directory.join(format!("{track:02}.wav"));
        std::fs::write(&path, tone(140.0 + track as f32 * 22.0)).unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title(format!("{song} {track:02}"));
        tag.set_artist(artist.to_owned());
        tag.insert_text(ItemKey::AlbumArtist, artist.to_owned());
        tag.set_album(title.to_owned());
        tag.set_track(track);
        tag.insert_text(ItemKey::RecordingDate, "2024".to_owned());
        tag.push_picture(
            Picture::unchecked(art.clone())
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
        );
        tag.save_to_path(&path, WriteOptions::default()).unwrap();
    }
}

fn main() {
    let root = PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    album(
        &root,
        "Ember Field 音",
        "Generated Ensemble",
        "Cinder",
        true,
    );
    album(&root, "Salt Water", "Test Harbour", "Tide", false);
}
