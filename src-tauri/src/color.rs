use image::imageops::FilterType;
use image::GenericImageView;

/// Pick a saturated, mid-lightness colour from cover art for the UI accent.
pub fn accent_from_bytes(bytes: &[u8]) -> Option<[u8; 3]> {
    let img = image::load_from_memory(bytes).ok()?;
    let small = img.resize(48, 48, FilterType::Triangle);
    let mut best: Option<(f32, [u8; 3])> = None;
    let mut buckets: [[f32; 5]; 24] = [[0.0; 5]; 24]; // weight, r, g, b, n

    for (_x, _y, px) in small.pixels() {
        let [r, g, b, a] = px.0;
        if a < 200 {
            continue;
        }
        let (h, s, l) = rgb_to_hsl(r, g, b);
        if s < 0.18 || l < 0.18 || l > 0.82 {
            continue;
        }
        let score = s * (1.0 - (l - 0.48).abs() * 1.4);
        let rgb = [r, g, b];
        if best.map(|(sc, _)| score > sc).unwrap_or(true) {
            best = Some((score, rgb));
        }
        let bucket = ((h / 360.0) * 24.0).floor() as usize % 24;
        let w = score;
        buckets[bucket][0] += w;
        buckets[bucket][1] += r as f32 * w;
        buckets[bucket][2] += g as f32 * w;
        buckets[bucket][3] += b as f32 * w;
        buckets[bucket][4] += 1.0;
    }

    let mut top: Option<(f32, [u8; 3])> = None;
    for b in buckets {
        if b[0] <= 0.0 {
            continue;
        }
        let rgb = [
            (b[1] / b[0]).round() as u8,
            (b[2] / b[0]).round() as u8,
            (b[3] / b[0]).round() as u8,
        ];
        if top.map(|(w, _)| b[0] > w).unwrap_or(true) {
            top = Some((b[0], rgb));
        }
    }
    top.map(|(_, c)| lift(c)).or_else(|| best.map(|(_, c)| lift(c)))
}

/// Nudge the colour so it stays readable as a progress-bar accent on a dark UI.
fn lift(c: [u8; 3]) -> [u8; 3] {
    let (h, s, l) = rgb_to_hsl(c[0], c[1], c[2]);
    let s = s.clamp(0.45, 0.85);
    let l = l.clamp(0.48, 0.68);
    hsl_to_rgb(h, s, l)
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d) % 6.0
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    let h = if h < 0.0 { h + 360.0 } else { h };
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    [
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    ]
}
