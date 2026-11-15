//! Five-band peaking equalizer (RBJ cookbook) applied in the mixer.

use std::f32::consts::PI;

pub const BANDS: usize = 5;
/// Center frequencies for the graphic EQ bands.
pub const FREQS: [f32; BANDS] = [60.0, 230.0, 910.0, 3600.0, 14_000.0];
pub const Q: f32 = 1.0;

#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn flat() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn peaking(sr: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        if !sr.is_finite() || sr < 1000.0 || !freq.is_finite() || freq <= 0.0 {
            return Self::flat();
        }
        let freq = freq.min(sr * 0.45);
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sr;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q.max(0.1));
        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos;
        let a2 = 1.0 - alpha / a;
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

pub struct Eq {
    enabled: bool,
    gains: [f32; BANDS],
    left: [Biquad; BANDS],
    right: [Biquad; BANDS],
    sr: u32,
}

impl Eq {
    pub fn new(sr: u32) -> Self {
        let mut eq = Self {
            enabled: false,
            gains: [0.0; BANDS],
            left: [Biquad::flat(); BANDS],
            right: [Biquad::flat(); BANDS],
            sr: sr.max(1),
        };
        eq.rebuild();
        eq
    }

    pub fn set(&mut self, gains: [f32; BANDS], enabled: bool) {
        let mut dirty = self.enabled != enabled;
        for i in 0..BANDS {
            let g = gains[i].clamp(-12.0, 12.0);
            if (self.gains[i] - g).abs() > 0.01 {
                dirty = true;
            }
            self.gains[i] = g;
        }
        self.enabled = enabled;
        if dirty {
            self.rebuild();
        }
    }

    pub fn set_sample_rate(&mut self, sr: u32) {
        let sr = sr.max(1);
        if self.sr != sr {
            self.sr = sr;
            self.rebuild();
        }
    }

    fn rebuild(&mut self) {
        let sr = self.sr as f32;
        for i in 0..BANDS {
            let g = if self.enabled { self.gains[i] } else { 0.0 };
            // Keep filter history when only gain changes so we don't click.
            let z1l = self.left[i].z1;
            let z2l = self.left[i].z2;
            let z1r = self.right[i].z1;
            let z2r = self.right[i].z2;
            self.left[i] = Biquad::peaking(sr, FREQS[i], Q, g);
            self.right[i] = Biquad::peaking(sr, FREQS[i], Q, g);
            self.left[i].z1 = z1l;
            self.left[i].z2 = z2l;
            self.right[i].z1 = z1r;
            self.right[i].z2 = z2r;
        }
    }

    pub fn process_frame(&mut self, frame: &mut [i16], ch: usize) {
        if !self.enabled || frame.is_empty() {
            return;
        }
        if self.gains.iter().all(|g| g.abs() < 0.05) {
            return;
        }
        let ch = ch.max(1).min(frame.len());
        for c in 0..ch {
            let mut x = frame[c] as f32;
            let band = if c == 0 {
                &mut self.left
            } else {
                &mut self.right
            };
            for b in band.iter_mut() {
                x = b.process(x);
            }
            frame[c] = x.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }
}
