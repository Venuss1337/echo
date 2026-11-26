//! Writes a small demonstration library for screenshots: eight fictional
//! albums of short synthesized tones with procedurally drawn covers. Every
//! byte is generated here, so the result can be published with ECHO under its
//! own license. Development only; nothing here ships.
//!
//!     cargo run --locked --manifest-path src-tauri/Cargo.toml --example showcase-fixtures -- <dir>

use lofty::{
    config::WriteOptions,
    picture::{MimeType, Picture, PictureType},
    prelude::*,
    tag::{Tag, TagType},
};
use std::f32::consts::TAU;
use std::path::{Path, PathBuf};

/// A few seconds of a soft two-note chord, so playback has something to do.
fn tone(root: f32) -> Vec<u8> {
    const RATE: u32 = 22_050;
    let samples: Vec<u8> = (0..RATE * 6)
        .flat_map(|n| {
            let t = n as f32 / RATE as f32;
            let envelope = (t * 2.0).min(1.0) * (6.0 - t).min(1.0);
            let value = (t * root * TAU).sin() + 0.5 * (t * root * 1.5 * TAU).sin();
            ((value * 700.0 * envelope) as i16).to_le_bytes()
        })
        .collect();
    let mut wav = Vec::with_capacity(samples.len() + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    wav.extend_from_slice(&samples);
    wav
}

type Rgb = [f32; 3];

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// A cheap deterministic hash for film grain, so covers are not flat.
fn grain(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263) ^ seed;
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xff) as f32 / 255.0 - 0.5
}

/// Each design maps normalized coordinates to a colour.
fn cover(design: usize) -> Vec<u8> {
    let size = 900_u32;
    let image = image::RgbImage::from_fn(size, size, |px, py| {
        let (x, y) = (px as f32 / size as f32, py as f32 / size as f32);
        let colour = match design {
            // A low sun over layered hills.
            0 => {
                let sky = mix([62.0, 22.0, 40.0], [226.0, 120.0, 96.0], y * 1.3);
                let sun = ((x - 0.5).powi(2) + (y - 0.42).powi(2)).sqrt();
                let lit = if sun < 0.2 {
                    mix([252.0, 196.0, 150.0], sky, (sun / 0.2).powi(4))
                } else {
                    sky
                };
                let hill = 0.68 + 0.05 * (x * 9.0).sin() + 0.03 * (x * 23.0).cos();
                if y > hill {
                    mix([48.0, 20.0, 38.0], [20.0, 10.0, 22.0], (y - hill) * 3.0)
                } else {
                    lit
                }
            }
            // Teal water in concentric swells.
            1 => {
                let r = ((x - 0.3).powi(2) + (y - 0.7).powi(2)).sqrt();
                let wave = (r * 38.0).sin() * 0.5 + 0.5;
                mix([10.0, 52.0, 66.0], [118.0, 206.0, 214.0], wave * (1.1 - r))
            }
            // Overlapping pale moons on paper.
            2 => {
                let mut c: Rgb = [236.0, 226.0, 208.0];
                for (cx, cy, rad, tint) in [
                    (0.36, 0.40, 0.24, [214.0, 120.0, 96.0]),
                    (0.60, 0.52, 0.22, [96.0, 120.0, 168.0]),
                    (0.48, 0.70, 0.14, [222.0, 182.0, 92.0]),
                ] {
                    if ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() < rad {
                        c = mix(c, tint, 0.62);
                    }
                }
                c
            }
            // Night transit: diagonal light streaks on deep blue.
            3 => {
                let base = mix([8.0, 12.0, 40.0], [36.0, 30.0, 88.0], y);
                let streak = ((x * 0.8 + y) * 18.0).sin().max(0.0).powi(24);
                mix(base, [255.0, 196.0, 120.0], streak * (1.0 - y * 0.6))
            }
            // A meadow in morning haze.
            4 => {
                let haze = mix([214.0, 226.0, 196.0], [92.0, 128.0, 72.0], y.powf(1.6));
                let blades = ((x * 140.0).sin() * 0.5 + 0.5) * (y - 0.55).max(0.0) * 1.4;
                mix(haze, [40.0, 70.0, 36.0], blades)
            }
            // Violet static in soft horizontal bands.
            5 => {
                let band = ((y * 26.0).sin() * 0.5 + 0.5) * 0.35;
                mix(
                    [46.0, 18.0, 70.0],
                    [196.0, 120.0, 214.0],
                    0.25 + band + x * 0.25,
                )
            }
            // Copper rings.
            6 => {
                let r = ((x - 0.5).powi(2) + (y - 0.5).powi(2)).sqrt();
                let ring = (r * 52.0).sin() * 0.5 + 0.5;
                mix(
                    [70.0, 30.0, 16.0],
                    [228.0, 134.0, 72.0],
                    ring * (1.0 - r * 1.2),
                )
            }
            // A cool room of stacked blocks.
            _ => {
                let block = ((x * 4.0).floor() + (y * 3.0).floor() * 2.0) % 5.0 / 5.0;
                mix(
                    [40.0, 52.0, 66.0],
                    [168.0, 186.0, 200.0],
                    block * 0.7 + y * 0.2,
                )
            }
        };
        let noise = grain(px, py, design as u32) * 14.0;
        image::Rgb(colour.map(|channel| (channel + noise).clamp(0.0, 255.0) as u8))
    });
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    png.into_inner()
}

const ALBUMS: [(&str, &str, [&str; 5]); 8] = [
    (
        "Low Meridian",
        "Mira Vale",
        [
            "Last Light",
            "Afterglow",
            "Western Road",
            "Ember",
            "Slow Return",
        ],
    ),
    (
        "Glass Harbour",
        "Aven Drift",
        [
            "Tidewater",
            "Salt Lines",
            "Low Bridge",
            "Undertow",
            "Harbour Lights",
        ],
    ),
    (
        "Paper Moons",
        "Iona Bloom",
        ["Pale Hours", "Folded", "Two Moons", "Margins", "Quiet Room"],
    ),
    (
        "Night Transit",
        "Silver Index",
        [
            "Platform 9",
            "Overpass",
            "Neon Rain",
            "Last Train",
            "City Sleeps",
        ],
    ),
    (
        "Field Notes",
        "Ellis Fen",
        [
            "Morning Haze",
            "Long Grass",
            "Wind Study",
            "Dew",
            "Open Field",
        ],
    ),
    (
        "Velvet Static",
        "Ostra",
        ["Signal", "Soft Noise", "Carrier", "Violet", "Fade Out"],
    ),
    (
        "Copper Hours",
        "The Lanterns",
        ["Kindle", "Copper Hours", "Ring", "Warm Current", "Ashes"],
    ),
    (
        "Northern Room",
        "Kai Moreno",
        ["North", "Stacks", "Cold Light", "Blueprint", "Still"],
    ),
];

fn album(root: &Path, index: usize) {
    let (title, artist, songs) = ALBUMS[index];
    let directory = root.join(artist).join(title);
    std::fs::create_dir_all(&directory).unwrap();
    let art = cover(index);
    for (number, song) in songs.iter().enumerate() {
        let track = number as u32 + 1;
        let path = directory.join(format!("{track:02} {song}.wav"));
        std::fs::write(
            &path,
            tone(160.0 + index as f32 * 18.0 + track as f32 * 24.0),
        )
        .unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title((*song).to_owned());
        tag.set_artist(artist.to_owned());
        tag.insert_text(ItemKey::AlbumArtist, artist.to_owned());
        tag.set_album(title.to_owned());
        tag.set_track(track);
        tag.insert_text(ItemKey::RecordingDate, format!("{}", 2019 + index % 6));
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
    for index in 0..ALBUMS.len() {
        album(&root, index);
    }
}
