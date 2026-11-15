//! Transition DSP: crossfade curves, filter sweeps, varispeed, and beat
//! analysis used for BPM matching.
//!
//! Everything here is plain math with no audio-device or decoder
//! dependencies so it can be unit-tested directly. The mixer
//! (`mix.rs`) and the audio thread (`player.rs`) wire it together.

use std::collections::VecDeque;

/// Shape of one side of a crossfade (gain as a function of 0..1 progress).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Curve {
    Linear,
    /// sin/cos quarter-wave: constant perceived loudness. The classic default.
    #[default]
    EqualPower,
    /// Fast rise, slow finish (sounds like the track "arrives" early).
    Logarithmic,
    /// Slow rise, fast finish (keeps the other track in front longer).
    Exponential,
    /// Smooth ease-in/ease-out (raised cosine).
    Smooth,
}

impl Curve {
    pub fn parse(s: &str) -> Self {
        match s {
            "linear" => Curve::Linear,
            "logarithmic" | "log" => Curve::Logarithmic,
            "exponential" | "exp" => Curve::Exponential,
            "s_curve" | "scurve" | "s-curve" => Curve::Smooth,
            _ => Curve::EqualPower,
        }
    }

    /// Fade-out gain for progress `x` in 0..=1 (1 = full, 0 = silent).
    /// Mirror of [`Curve::gain`]; equal-power uses cos directly so the
    /// default blend is sample-identical to the original mixer.
    pub fn fade_out(self, x: f32) -> f32 {
        match self {
            Curve::EqualPower => (x.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).cos(),
            c => c.gain(1.0 - x),
        }
    }

    /// Fade-in gain for progress `x` in 0..=1 (0 = silent, 1 = full).
    pub fn gain(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Curve::Linear => x,
            Curve::EqualPower => (x * std::f32::consts::FRAC_PI_2).sin(),
            Curve::Logarithmic => (1.0 + 9.0 * x).log10(),
            Curve::Exponential => (10f32.powf(x) - 1.0) / 9.0,
            Curve::Smooth => 0.5 - 0.5 * (x * std::f32::consts::PI).cos(),
        }
    }
}

/// Optional filter sweep applied during a crossfade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FilterSweep {
    #[default]
    Off,
    /// Low-pass the outgoing track closed as it fades.
    LowPass,
    /// High-pass the incoming track open as it rises (bass swap feel).
    HighPass,
    /// Both: DJ-style "bass swap".
    Sweep,
}

impl FilterSweep {
    pub fn parse(s: &str) -> Self {
        match s {
            "lowpass" => FilterSweep::LowPass,
            "highpass" => FilterSweep::HighPass,
            "sweep" | "both" => FilterSweep::Sweep,
            _ => FilterSweep::Off,
        }
    }
    pub fn lowpass_out(self) -> bool {
        matches!(self, FilterSweep::LowPass | FilterSweep::Sweep)
    }
    pub fn highpass_in(self) -> bool {
        matches!(self, FilterSweep::HighPass | FilterSweep::Sweep)
    }
}

/// Everything the mixer needs to shape one crossfade.
#[derive(Clone, Debug, PartialEq)]
pub struct XfShape {
    pub secs: f32,
    pub curve_out: Curve,
    pub curve_in: Curve,
    /// Fraction (0.1..=1) of the window the outgoing track spends fading;
    /// the fade sits at the *end* of the window.
    pub out_len: f32,
    /// Fraction (0.1..=1) of the window the incoming track spends rising;
    /// the rise sits at the *start* of the window.
    pub in_len: f32,
    pub filter: FilterSweep,
}

impl XfShape {
    /// The historical behaviour: equal-power both ways, full overlap.
    pub fn equal_power(secs: f32) -> Self {
        Self {
            secs,
            curve_out: Curve::EqualPower,
            curve_in: Curve::EqualPower,
            out_len: 1.0,
            in_len: 1.0,
            filter: FilterSweep::Off,
        }
    }

    /// (outgoing gain, incoming gain) at window progress `t` in 0..=1.
    pub fn gains(&self, t: f32) -> (f32, f32) {
        let t = t.clamp(0.0, 1.0);
        let out_len = self.out_len.clamp(0.05, 1.0);
        let in_len = self.in_len.clamp(0.05, 1.0);
        let t_out = ((t - (1.0 - out_len)) / out_len).clamp(0.0, 1.0);
        let t_in = (t / in_len).clamp(0.0, 1.0);
        (self.curve_out.fade_out(t_out), self.curve_in.gain(t_in))
    }

    /// Progress of the outgoing fade alone (for the low-pass sweep).
    pub fn out_progress(&self, t: f32) -> f32 {
        let out_len = self.out_len.clamp(0.05, 1.0);
        ((t - (1.0 - out_len)) / out_len).clamp(0.0, 1.0)
    }

    /// Progress of the incoming rise alone (for the high-pass sweep).
    pub fn in_progress(&self, t: f32) -> f32 {
        (t / self.in_len.clamp(0.05, 1.0)).clamp(0.0, 1.0)
    }
}

/// Full crossfade configuration as set from the settings page.
#[derive(Clone, Debug, PartialEq)]
pub struct XfConfig {
    pub shape: XfShape,
    /// Trim trailing silence of the outgoing and leading silence of the
    /// incoming track.
    pub smart: bool,
    pub bpm_match: bool,
    /// Largest tempo change allowed, in percent (e.g. 6.0 = ±6 %).
    pub bpm_max_pct: f32,
    /// Start the overlap on a beat of the outgoing track, and start the
    /// incoming track on its first beat.
    pub beat_align: bool,
    /// After the fade, glide the new track back to its own tempo over
    /// this many seconds.
    pub bpm_return_secs: f32,
}

impl XfConfig {
    pub fn simple(secs: f32) -> Self {
        Self {
            shape: XfShape::equal_power(secs),
            smart: false,
            bpm_match: false,
            bpm_max_pct: 6.0,
            beat_align: true,
            bpm_return_secs: 8.0,
        }
    }
    pub fn needs_analysis(&self) -> bool {
        self.smart || self.bpm_match
    }
}

/// RBJ biquad (DF-I), one per channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    pub fn set_lowpass(&mut self, fc: f32, sr: f32) {
        self.set(fc, sr, false);
    }
    pub fn set_highpass(&mut self, fc: f32, sr: f32) {
        self.set(fc, sr, true);
    }
    fn set(&mut self, fc: f32, sr: f32, high: bool) {
        let fc = fc.clamp(10.0, sr * 0.45);
        let w0 = 2.0 * std::f32::consts::PI * fc / sr;
        let (s, c) = w0.sin_cos();
        let q = std::f32::consts::FRAC_1_SQRT_2;
        let alpha = s / (2.0 * q);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = if high {
            ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0)
        } else {
            ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0)
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = -2.0 * c / a0;
        self.a2 = (1.0 - alpha) / a0;
    }
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Exponential sweep between two frequencies.
pub fn sweep_hz(from: f32, to: f32, t: f32) -> f32 {
    from * (to / from).powf(t.clamp(0.0, 1.0))
}

// ---------------------------------------------------------------------------
// Beat analysis
// ---------------------------------------------------------------------------

/// Frames per envelope hop.
pub const HOP: usize = 256;

/// Peak level below which a frame counts as silence (~ -48 dBFS).
pub const SILENCE: i32 = 130;

/// Builds a two-band energy envelope (full band + low band) one frame at
/// a time. Used both live on the outgoing track and offline on the
/// incoming one.
#[derive(Clone, Debug)]
pub struct EnvBuilder {
    lp: f32,
    lp_a: f32,
    acc_full: f32,
    acc_low: f32,
    n: usize,
    pub full: VecDeque<f32>,
    pub low: VecDeque<f32>,
    cap: usize,
    /// Frames consumed so far.
    pub frames: u64,
}

impl EnvBuilder {
    pub fn new(sr: u32, max_secs: f32) -> Self {
        let sr = sr.max(1) as f32;
        // One-pole low-pass ≈ 150 Hz isolates kick/bass energy.
        let lp_a = 1.0 - (-2.0 * std::f32::consts::PI * 150.0 / sr).exp();
        let cap = ((max_secs * sr) / HOP as f32).ceil().max(1.0) as usize;
        Self {
            lp: 0.0,
            lp_a,
            acc_full: 0.0,
            acc_low: 0.0,
            n: 0,
            full: VecDeque::with_capacity(cap),
            low: VecDeque::with_capacity(cap),
            cap,
            frames: 0,
        }
    }

    pub fn reset(&mut self) {
        self.lp = 0.0;
        self.acc_full = 0.0;
        self.acc_low = 0.0;
        self.n = 0;
        self.full.clear();
        self.low.clear();
        self.frames = 0;
    }

    pub fn push_frame(&mut self, frame: &[i16]) {
        let ch = frame.len().max(1) as f32;
        let m = frame.iter().map(|&v| v as f32).sum::<f32>() / ch / 32768.0;
        self.lp += self.lp_a * (m - self.lp);
        self.acc_full += m * m;
        self.acc_low += self.lp * self.lp;
        self.n += 1;
        self.frames += 1;
        if self.n >= HOP {
            if self.full.len() >= self.cap {
                self.full.pop_front();
                self.low.pop_front();
            }
            self.full.push_back(self.acc_full / HOP as f32);
            self.low.push_back(self.acc_low / HOP as f32);
            self.acc_full = 0.0;
            self.acc_low = 0.0;
            self.n = 0;
        }
    }

    /// Frame index (in `frames` units) where the first stored hop begins.
    pub fn window_start(&self) -> u64 {
        let complete = self.frames - self.n as u64;
        complete.saturating_sub((self.full.len() * HOP) as u64)
    }
}

/// Detected beat grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beat {
    pub bpm: f64,
    /// Beat period in frames.
    pub period: f64,
    /// Frame offset of the first beat from the start of the window.
    pub phase: f64,
    /// Comb strength relative to the average onset (≥ ~1.5 is usable).
    pub confidence: f64,
}

fn onset(full: &[f32], low: &[f32]) -> Vec<f32> {
    let n = full.len().min(low.len());
    let mut o = vec![0f32; n];
    const EPS: f32 = 1e-9;
    for i in 1..n {
        let dl = (low[i] + EPS).ln() - (low[i - 1] + EPS).ln();
        let df = (full[i] + EPS).ln() - (full[i - 1] + EPS).ln();
        o[i] = dl.max(0.0) + 0.5 * df.max(0.0);
    }
    // Remove the local mean so sustained loud parts don't dominate.
    let w = 16usize;
    let mut out = vec![0f32; n];
    let mut sum = 0f32;
    for i in 0..n {
        sum += o[i];
        if i >= w {
            sum -= o[i - w];
        }
        let cnt = (i + 1).min(w) as f32;
        out[i] = (o[i] - sum / cnt).max(0.0);
    }
    out
}

fn comb(o: &[f32], period: f64, phase: f64) -> f64 {
    let mut s = 0f64;
    let mut k = 0usize;
    let mut x = phase;
    let n = o.len() as f64;
    while x < n - 0.5 {
        let i = x.round() as usize;
        // Tolerate ±1 hop of jitter.
        let lo = i.saturating_sub(1);
        let hi = (i + 1).min(o.len() - 1);
        let v = o[lo..=hi].iter().cloned().fold(0f32, f32::max);
        s += v as f64;
        k += 1;
        x += period;
    }
    if k == 0 {
        0.0
    } else {
        s / k as f64
    }
}

/// Estimate tempo and beat phase from an envelope. `sr` is the sample
/// rate the envelope was built at. Needs ~5 s of audio.
pub fn analyze(full: &[f32], low: &[f32], sr: u32) -> Option<Beat> {
    let o = onset(full, low);
    let hop_secs = HOP as f64 / sr.max(1) as f64;
    if (o.len() as f64) * hop_secs < 5.0 {
        return None;
    }
    let mean = o.iter().map(|&v| v as f64).sum::<f64>() / o.len() as f64;
    if mean <= 1e-6 {
        return None;
    }
    let lag_of = |bpm: f64| 60.0 / (bpm * hop_secs);
    let lmin = lag_of(190.0).floor().max(2.0) as usize;
    let lmax = (lag_of(65.0).ceil() as usize).min(o.len() / 2);
    if lmax <= lmin + 2 {
        return None;
    }
    // Coarse: weighted autocorrelation, preferring ~120 BPM.
    let ac = |l: usize| -> f64 {
        let mut s = 0f64;
        for i in l..o.len() {
            s += o[i] as f64 * o[i - l] as f64;
        }
        s / (o.len() - l) as f64
    };
    let mut best_l = 0usize;
    let mut best = f64::MIN;
    for l in lmin..=lmax {
        let bpm = 60.0 / (l as f64 * hop_secs);
        let w = (-0.5 * ((bpm / 120.0).log2() / 0.9).powi(2)).exp();
        let mut s = ac(l);
        if 2 * l < o.len() {
            s += 0.5 * ac(2 * l);
        }
        let s = s * w;
        if s > best {
            best = s;
            best_l = l;
        }
    }
    if best_l == 0 {
        return None;
    }
    // Fine: fractional period + phase by comb filtering.
    let mut bp = best_l as f64;
    let mut bph = 0f64;
    let mut bs = f64::MIN;
    let mut p = best_l as f64 - 1.5;
    while p <= best_l as f64 + 1.5 {
        if p >= 2.0 {
            let mut ph = 0f64;
            while ph < p {
                let s = comb(&o, p, ph);
                if s > bs {
                    bs = s;
                    bp = p;
                    bph = ph;
                }
                ph += 0.5;
            }
        }
        p += 0.02;
    }
    let confidence = bs / mean;
    let period = bp * HOP as f64;
    // Onsets lag the true transient by about half a hop.
    let phase = (bph * HOP as f64).max(0.0);
    Some(Beat {
        bpm: 60.0 * sr as f64 / period,
        period,
        phase,
        confidence,
    })
}

/// Minimum comb confidence before a detected tempo is trusted.
pub const MIN_CONFIDENCE: f64 = 1.6;

/// Rate to play the incoming track at so its tempo meets the outgoing one,
/// considering half/double-time. Returns `(rate, effective_incoming_bpm)`
/// or `None` if the needed change exceeds `max_pct`.
pub fn match_rate(bpm_out: f64, bpm_in: f64, max_pct: f64) -> Option<(f64, f64)> {
    if bpm_out <= 0.0 || bpm_in <= 0.0 || max_pct <= 0.0 {
        return None;
    }
    let mut best: Option<(f64, f64)> = None;
    for eff in [bpm_in, bpm_in * 2.0, bpm_in / 2.0] {
        let r = bpm_out / eff;
        if best.is_none_or(|(br, _)| (r - 1.0).abs() < (br - 1.0).abs()) {
            best = Some((r, eff));
        }
    }
    let (r, eff) = best?;
    if (r - 1.0).abs() * 100.0 > max_pct + 1e-9 {
        return None;
    }
    Some((r, eff))
}

/// Frames to wait from `now` (in the same frame clock as the grid) until
/// the next beat at `phase + k * period`.
pub fn frames_to_next_beat(now: f64, phase: f64, period: f64) -> f64 {
    if period <= 0.0 {
        return 0.0;
    }
    let k = ((now - phase) / period).ceil();
    let next = phase + k * period;
    (next - now).max(0.0)
}

// ---------------------------------------------------------------------------
// Varispeed
// ---------------------------------------------------------------------------

/// Tempo (and pitch) change by resampling with 4-point Catmull-Rom
/// interpolation. When the rate is exactly 1 and no ramp is pending this
/// is bypassed entirely, so normal playback is bit-identical.
#[derive(Debug)]
pub struct Vari {
    pub rate: f64,
    tgt: f64,
    step: f64,
    frac: f64,
    hist: VecDeque<Vec<i16>>,
    primed: bool,
    flush: VecDeque<Vec<i16>>,
    ended: bool,
}

impl Default for Vari {
    fn default() -> Self {
        Self::with_rate(1.0)
    }
}

/// Result of pulling one frame from a source.
#[derive(Debug)]
pub enum FrameRes {
    Ok,
    Buffering,
    Ended,
    Stall(String),
}

impl Vari {
    pub fn with_rate(rate: f64) -> Self {
        Self {
            rate,
            tgt: rate,
            step: 0.0,
            frac: 0.0,
            hist: VecDeque::with_capacity(5),
            primed: false,
            flush: VecDeque::new(),
            ended: false,
        }
    }

    /// Glide to `tgt` over `secs` seconds of output at `sr`.
    pub fn ramp_to(&mut self, tgt: f64, secs: f64, sr: u32) {
        self.tgt = tgt;
        let frames = (secs.max(0.05) * sr.max(1) as f64).max(1.0);
        self.step = (tgt - self.rate).abs() / frames;
    }

    pub fn active(&self) -> bool {
        self.primed || !self.flush.is_empty() || self.rate != 1.0 || self.tgt != 1.0
    }

    /// Pull one output frame into `out`, using `raw` to read source frames.
    pub fn pull<F>(&mut self, out: &mut [i16], mut raw: F) -> FrameRes
    where
        F: FnMut(&mut [i16]) -> FrameRes,
    {
        if let Some(f) = self.flush.pop_front() {
            out.copy_from_slice(&f);
            return FrameRes::Ok;
        }
        if !self.active() {
            return raw(out);
        }
        let ch = out.len();
        if !self.primed {
            let mut f = vec![0i16; ch];
            match raw(&mut f) {
                FrameRes::Ok => {}
                r => return r,
            }
            self.hist.clear();
            self.hist.push_back(f.clone());
            self.hist.push_back(f);
            self.primed = true;
            self.ended = false;
            self.frac = 0.0;
        }
        while self.hist.len() < 4 && !self.ended {
            let mut f = vec![0i16; ch];
            match raw(&mut f) {
                FrameRes::Ok => self.hist.push_back(f),
                FrameRes::Buffering => return FrameRes::Buffering,
                FrameRes::Ended => self.ended = true,
                FrameRes::Stall(e) => {
                    self.primed = false;
                    self.hist.clear();
                    return FrameRes::Stall(e);
                }
            }
        }
        if self.hist.len() < 2 {
            self.primed = false;
            self.hist.clear();
            return FrameRes::Ended;
        }
        let at = |i: usize| -> &Vec<i16> { &self.hist[i.min(self.hist.len() - 1)] };
        let t = self.frac as f32;
        for (c, o) in out.iter_mut().enumerate() {
            let x0 = at(0)[c] as f32;
            let x1 = at(1)[c] as f32;
            let x2 = at(2)[c] as f32;
            let x3 = at(3)[c] as f32;
            let y = x1
                + 0.5
                    * t
                    * (x2 - x0
                        + t * (2.0 * x0 - 5.0 * x1 + 4.0 * x2 - x3
                            + t * (3.0 * (x1 - x2) + x3 - x0)));
            *o = y.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
        // Advance the rate ramp.
        if self.rate != self.tgt {
            if (self.tgt - self.rate).abs() <= self.step {
                self.rate = self.tgt;
            } else if self.rate < self.tgt {
                self.rate += self.step;
            } else {
                self.rate -= self.step;
            }
        }
        // Back at unity: creep forward a hair until the read head lands on
        // a source frame, then hand back to the direct path.
        let settling = self.rate == 1.0 && self.tgt == 1.0;
        let adv = if settling { 1.002 } else { self.rate };
        self.frac += adv;
        let mut wrapped = false;
        while self.frac >= 1.0 {
            self.frac -= 1.0;
            if self.hist.len() > 1 {
                self.hist.pop_front();
            }
            wrapped = true;
        }
        if settling && wrapped && self.frac < 0.0025 && !self.ended {
            // Output position is now exactly hist[1]; replay it and the
            // frames already read, then bypass.
            self.hist.pop_front();
            self.flush = std::mem::take(&mut self.hist);
            self.primed = false;
            self.frac = 0.0;
        }
        FrameRes::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_hit_endpoints() {
        for c in [
            Curve::Linear,
            Curve::EqualPower,
            Curve::Logarithmic,
            Curve::Exponential,
            Curve::Smooth,
        ] {
            assert!(c.gain(0.0).abs() < 1e-5, "{c:?} at 0");
            assert!((c.gain(1.0) - 1.0).abs() < 1e-5, "{c:?} at 1");
            // Monotonic.
            let mut prev = -1.0;
            for i in 0..=100 {
                let g = c.gain(i as f32 / 100.0);
                assert!(g >= prev - 1e-6, "{c:?} not monotonic");
                prev = g;
            }
        }
    }

    #[test]
    fn default_shape_matches_legacy_equal_power() {
        let s = XfShape::equal_power(4.0);
        for i in 0..=50 {
            let t = i as f32 / 50.0;
            let (go, gi) = s.gains(t);
            assert!((go - (t * std::f32::consts::FRAC_PI_2).cos()).abs() < 1e-5);
            assert!((gi - (t * std::f32::consts::FRAC_PI_2).sin()).abs() < 1e-5);
        }
    }

    #[test]
    fn partial_lengths_hold_then_move() {
        let s = XfShape {
            out_len: 0.5,
            in_len: 0.5,
            ..XfShape::equal_power(4.0)
        };
        let (go, gi) = s.gains(0.25);
        assert!((go - 1.0).abs() < 1e-5, "outgoing holds until its half");
        assert!(gi > 0.5);
        let (go, gi) = s.gains(0.75);
        assert!(go < 1.0);
        assert!((gi - 1.0).abs() < 1e-5, "incoming is full after its half");
    }

    #[test]
    fn parse_names() {
        assert_eq!(Curve::parse("s_curve"), Curve::Smooth);
        assert_eq!(Curve::parse("nonsense"), Curve::EqualPower);
        assert_eq!(FilterSweep::parse("sweep"), FilterSweep::Sweep);
        assert_eq!(FilterSweep::parse("off"), FilterSweep::Off);
    }

    fn click_track(bpm: f64, sr: u32, secs: f64, offset: f64) -> Vec<i16> {
        // Mono decaying 60 Hz "kick" bursts plus a little hiss.
        let n = (secs * sr as f64) as usize;
        let period = 60.0 / bpm * sr as f64;
        let mut out = vec![0i16; n];
        let mut seed = 12345u32;
        for (i, s) in out.iter_mut().enumerate() {
            let t = i as f64;
            let since = (t - offset * sr as f64).rem_euclid(period);
            let env = if t >= offset * sr as f64 {
                (-since / (0.03 * sr as f64)).exp()
            } else {
                0.0
            };
            let kick = (2.0 * std::f64::consts::PI * 60.0 * since / sr as f64).sin() * env;
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = ((seed >> 16) as f64 / 65536.0 - 0.5) * 0.01;
            *s = ((kick * 0.8 + noise) * 30000.0) as i16;
        }
        out
    }

    fn env_of(samples: &[i16], sr: u32) -> EnvBuilder {
        let mut e = EnvBuilder::new(sr, 12.0);
        for &s in samples {
            e.push_frame(&[s, s]);
        }
        e
    }

    #[test]
    fn detects_tempo_of_click_track() {
        let sr = 44100;
        for bpm in [96.0, 120.0, 128.0, 140.0, 174.0] {
            let e = env_of(&click_track(bpm, sr, 11.0, 0.0), sr);
            let full: Vec<f32> = e.full.iter().cloned().collect();
            let low: Vec<f32> = e.low.iter().cloned().collect();
            let b = analyze(&full, &low, sr).expect("beat");
            let err = (b.bpm - bpm)
                .abs()
                .min((b.bpm * 2.0 - bpm).abs())
                .min((b.bpm / 2.0 - bpm).abs());
            assert!(err < 1.0, "want {bpm}, got {:.2}", b.bpm);
            assert!(
                b.confidence >= MIN_CONFIDENCE,
                "confidence {:.2} at {bpm}",
                b.confidence
            );
        }
    }

    #[test]
    fn detects_beat_phase() {
        let sr = 44100;
        let offset = 0.21; // first kick at 210 ms
        let e = env_of(&click_track(120.0, sr, 11.0, offset), sr);
        let full: Vec<f32> = e.full.iter().cloned().collect();
        let low: Vec<f32> = e.low.iter().cloned().collect();
        let b = analyze(&full, &low, sr).expect("beat");
        let want = offset * sr as f64;
        let d = (b.phase - want).rem_euclid(b.period);
        let d = d.min(b.period - d);
        assert!(
            d < 0.03 * sr as f64,
            "phase off by {:.1} ms",
            d / sr as f64 * 1000.0
        );
    }

    #[test]
    fn silence_has_no_tempo() {
        let e = env_of(&vec![0i16; 44100 * 8], 44100);
        let full: Vec<f32> = e.full.iter().cloned().collect();
        let low: Vec<f32> = e.low.iter().cloned().collect();
        assert!(analyze(&full, &low, 44100).is_none());
    }

    #[test]
    fn match_rate_limits_and_folds() {
        let (r, _) = match_rate(128.0, 124.0, 6.0).unwrap();
        assert!((r - 128.0 / 124.0).abs() < 1e-9);
        assert!(match_rate(128.0, 100.0, 6.0).is_none());
        // Half time folds.
        let (r, eff) = match_rate(140.0, 71.0, 6.0).unwrap();
        assert!((eff - 142.0).abs() < 1e-9 && (r - 140.0 / 142.0).abs() < 1e-9);
    }

    #[test]
    fn next_beat_wait() {
        assert!((frames_to_next_beat(100.0, 10.0, 50.0) - 10.0).abs() < 1e-9);
        assert!((frames_to_next_beat(110.0, 10.0, 50.0)).abs() < 1e-9);
        assert!((frames_to_next_beat(0.0, 30.0, 50.0) - 30.0).abs() < 1e-9);
    }

    fn ramp_source(n: usize) -> impl FnMut(&mut [i16]) -> FrameRes {
        let mut i = 0usize;
        move |f: &mut [i16]| {
            if i >= n {
                return FrameRes::Ended;
            }
            for s in f.iter_mut() {
                *s = i as i16;
            }
            i += 1;
            FrameRes::Ok
        }
    }

    #[test]
    fn vari_unity_is_bypass() {
        let mut v = Vari::default();
        let mut src = ramp_source(10);
        let mut out = [0i16; 2];
        for i in 0..10 {
            assert!(matches!(v.pull(&mut out, &mut src), FrameRes::Ok));
            assert_eq!(out, [i as i16, i as i16]);
        }
        assert!(matches!(v.pull(&mut out, &mut src), FrameRes::Ended));
    }

    #[test]
    fn vari_speeds_up_and_returns_to_bypass() {
        let mut v = Vari::with_rate(1.05);
        let mut src = ramp_source(20_000);
        let mut out = [0i16; 1];
        let mut last = -1i32;
        let mut produced = 0usize;
        // 1000 frames at 1.05 → ~1050 source frames consumed.
        for _ in 0..1000 {
            assert!(matches!(v.pull(&mut out, &mut src), FrameRes::Ok));
            assert!(out[0] as i32 >= last, "monotonic read head");
            last = out[0] as i32;
            produced += 1;
        }
        assert!((last - 1049).abs() <= 3, "read head at {last}");
        v.ramp_to(1.0, 0.01, 44100);
        // Glide back, then it must drop to the bypass path seamlessly.
        let mut bypassed = false;
        for _ in 0..5000 {
            assert!(matches!(v.pull(&mut out, &mut src), FrameRes::Ok));
            assert!(out[0] as i32 >= last);
            if out[0] as i32 - last > 2 {
                panic!("jumped from {last} to {}", out[0]);
            }
            last = out[0] as i32;
            produced += 1;
            if !v.active() {
                bypassed = true;
            }
        }
        assert!(bypassed, "should return to bypass after settling");
        assert!(produced > 0);
        // Once bypassed, frames are consecutive.
        let mut prev = last;
        for _ in 0..50 {
            v.pull(&mut out, &mut src);
            assert_eq!(out[0] as i32, prev + 1);
            prev = out[0] as i32;
        }
    }

    #[test]
    fn biquad_lowpass_attenuates_highs() {
        let sr = 44100.0;
        let mut f = Biquad::default();
        f.set_lowpass(300.0, sr);
        let mut peak = 0f32;
        for i in 0..4410 {
            let x = (2.0 * std::f32::consts::PI * 8000.0 * i as f32 / sr).sin();
            let y = f.process(x);
            if i > 1000 {
                peak = peak.max(y.abs());
            }
        }
        assert!(peak < 0.05, "8 kHz through 300 Hz LP: {peak}");
    }
}
