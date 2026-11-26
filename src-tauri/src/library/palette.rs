//! Three art-directed colors from a cached thumbnail.
//!
//! `ink` is a deep field colored by the cover's dominant hue, `glow` a richer
//! but still dark companion for lighting, and `accent` a light signal color.
//! Contrast is enforced here, once, so no component can produce unreadable
//! text: the interface's light text reads at 7:1 on `ink` and 4.5:1 on `glow`,
//! and `accent` reads at 4.5:1 on both the workspace and `ink`.

use crate::contract::Palette;

type Rgb = [f32; 3];
const TEXT: Rgb = [0.957, 0.937, 0.925];
const TRANSPORT: Rgb = [0.125, 0.114, 0.141];
const SIGNAL: Rgb = [0.933, 0.706, 0.761];

fn luminance(color: Rgb) -> f32 {
    let [r, g, b] = color.map(|v| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn hsl([r, g, b]: Rgb) -> Rgb {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let (light, range) = ((max + min) / 2.0, max - min);
    if range < 1e-4 {
        return [0.0, 0.0, light];
    }
    let saturation = range / (1.0 - (2.0 * light - 1.0).abs()).max(1e-4);
    let hue = if max == r {
        ((g - b) / range).rem_euclid(6.0)
    } else if max == g {
        (b - r) / range + 2.0
    } else {
        (r - g) / range + 4.0
    };
    [hue * 60.0, saturation.min(1.0), light]
}

fn rgb([hue, saturation, light]: Rgb) -> Rgb {
    let chroma = (1.0 - (2.0 * light - 1.0).abs()) * saturation;
    let part = hue.rem_euclid(360.0) / 60.0;
    let second = chroma * (1.0 - (part % 2.0 - 1.0).abs());
    let base = light - chroma / 2.0;
    let [r, g, b] = match part as u32 {
        0 => [chroma, second, 0.0],
        1 => [second, chroma, 0.0],
        2 => [0.0, chroma, second],
        3 => [0.0, second, chroma],
        4 => [second, 0.0, chroma],
        _ => [chroma, 0.0, second],
    };
    [r + base, g + base, b + base]
}

/// Walks lightness until the predicate holds, or stops at the end of range.
fn adjust(mut color: Rgb, step: f32, ok: impl Fn(Rgb) -> bool) -> Rgb {
    while !ok(quantized(color)) && (0.02..=0.96).contains(&(color[2] + step)) {
        color[2] += step;
    }
    quantized(color)
}

/// The color actually shipped, so checks see the same rounding as the page.
fn quantized(color: Rgb) -> Rgb {
    rgb(color).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0)
}

fn hex(color: Rgb) -> String {
    let [r, g, b] = color.map(|v| (v * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Decodes a trusted-size cached JPEG and extracts its palette.
pub(crate) fn extract(jpeg: &[u8]) -> Option<Palette> {
    let image = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).ok()?;
    let small = image.thumbnail(56, 56).to_rgb8();
    Some(from_pixels(small.pixels().map(|p| p.0)))
}

fn from_pixels(pixels: impl Iterator<Item = [u8; 3]>) -> Palette {
    #[derive(Clone, Copy, Default)]
    struct Bucket {
        weight: f32,
        x: f32,
        y: f32,
        saturation: f32,
        light: f32,
    }
    // Twenty-four hue families. Averaging hue on the circle keeps reds intact.
    let mut buckets = [Bucket::default(); 24];
    let (mut total, mut colored) = (0.0_f32, 0.0_f32);
    for pixel in pixels {
        let [hue, saturation, light] = hsl(pixel.map(|v| f32::from(v) / 255.0));
        total += 1.0;
        // Near-black, near-white and gray pixels say little about the hue.
        let weight = saturation * (1.0 - (2.0 * light - 1.0).abs());
        colored += weight;
        let bucket = &mut buckets[(hue / 15.0) as usize % 24];
        let (sin, cos) = hue.to_radians().sin_cos();
        bucket.weight += weight;
        bucket.x += cos * weight;
        bucket.y += sin * weight;
        bucket.saturation += saturation * weight;
        bucket.light += light * weight;
    }
    // A monochrome cover keeps the house charcoal instead of inventing a hue.
    if total == 0.0 || colored / total < 0.035 {
        return finish([330.0, 0.06], [330.0, 0.08], hsl(SIGNAL));
    }
    let describe = |b: Bucket| {
        let hue = b.y.atan2(b.x).to_degrees().rem_euclid(360.0);
        [hue, b.saturation / b.weight, b.light / b.weight]
    };
    let dominant = buckets
        .iter()
        .copied()
        .max_by(|a, b| a.weight.total_cmp(&b.weight))
        .map_or([330.0, 0.1, 0.5], describe);
    // The companion must be a clearly different family with real presence.
    let distance = |a: f32, b: f32| {
        let d = (a - b).abs() % 360.0;
        d.min(360.0 - d)
    };
    let companion = buckets
        .iter()
        .copied()
        .filter(|b| b.weight > colored * 0.08)
        .map(|b| (b.weight, describe(b)))
        .filter(|(_, c)| distance(c[0], dominant[0]) > 28.0)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(dominant, |(_, c)| c);
    finish(
        [dominant[0], (dominant[1] * 0.62).clamp(0.10, 0.46)],
        [companion[0], (companion[1] * 0.8).clamp(0.14, 0.58)],
        [dominant[0], (dominant[1] * 0.9).clamp(0.30, 0.78), 0.78],
    )
}

fn finish(ink: [f32; 2], glow: [f32; 2], accent: Rgb) -> Palette {
    let ink = adjust([ink[0], ink[1], 0.20], -0.005, |c| contrast(TEXT, c) >= 9.0);
    let glow = adjust([glow[0], glow[1], 0.36], -0.005, |c| {
        contrast(TEXT, c) >= 5.0
    });
    let accent = adjust(accent, 0.005, |c| {
        contrast(c, TRANSPORT) >= 6.0 && contrast(c, ink) >= 5.0
    });
    Palette {
        ink: hex(ink),
        glow: hex(glow),
        accent: hex(accent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(color: &str) -> Rgb {
        let v = u32::from_str_radix(&color[1..], 16).unwrap();
        [(v >> 16) & 255, (v >> 8) & 255, v & 255].map(|c| c as f32 / 255.0)
    }

    #[test]
    fn every_hue_and_extreme_cover_yields_readable_colors() {
        let mut covers: Vec<Vec<[u8; 3]>> = vec![
            vec![[0, 0, 0]; 64],
            vec![[255, 255, 255]; 64],
            vec![[128, 128, 128]; 64],
            vec![],
        ];
        for hue in (0..360).step_by(5) {
            for light in [0.15, 0.5, 0.85] {
                let c = rgb([hue as f32, 1.0, light]).map(|v| (v * 255.0) as u8);
                covers.push(vec![c; 64]);
                covers.push([vec![c; 40], vec![[10, 10, 10]; 24]].concat());
            }
        }
        for pixels in covers {
            let p = from_pixels(pixels.into_iter());
            let (ink, glow, accent) = (parse(&p.ink), parse(&p.glow), parse(&p.accent));
            assert!(contrast(TEXT, ink) >= 7.0, "{p:?}");
            assert!(contrast(TEXT, glow) >= 4.5, "{p:?}");
            assert!(contrast(accent, TRANSPORT) >= 4.5, "{p:?}");
            assert!(contrast(accent, ink) >= 4.5, "{p:?}");
        }
    }

    #[test]
    fn palettes_follow_the_cover_and_are_deterministic() {
        let orange = [[230, 110, 20]; 50];
        let blue = [[20, 90, 220]; 50];
        let a = from_pixels(orange.iter().copied());
        assert_eq!(a, from_pixels(orange.iter().copied()));
        let hue = |p: &Palette| hsl(parse(&p.accent))[0];
        assert!((10.0..50.0).contains(&hue(&a)), "{a:?}");
        let b = from_pixels(blue.into_iter());
        assert!((190.0..240.0).contains(&hue(&b)), "{b:?}");
    }

    #[test]
    fn decodes_a_real_thumbnail_and_rejects_garbage() {
        let png = super::super::artwork::fixtures::png(64, 64, 9);
        let thumb = super::super::artwork::make_thumbnail(&png).unwrap();
        assert!(extract(&thumb.jpeg).is_some());
        assert!(extract(b"not a jpeg").is_none());
    }
}
