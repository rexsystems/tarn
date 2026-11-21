//! AutoMix: picks where and how long to blend into the next song, the way a
//! DJ would, from the outgoing track's cached analysis.
//!
//! * The blend ends where the music ends (trailing silence is skipped).
//! * A quiet outro is where the mix happens: the blend starts when the
//!   outro starts (up to 16 s), so the next song rises over it.
//! * With a clear beat, the length is whole bars (8 bars for faster songs,
//!   4 bars for slower ones) and the start lands on a bar line.
//! * Without a usable beat (ballads, spoken intros) it stays a short,
//!   smooth fade.
//!
//! Tempo matching, beat alignment of the incoming track, keylock and the
//! bass swap are done by the regular transition code (see `transition.rs`);
//! AutoMix only turns them on and chooses the timing.

use crate::analysis::TrackAnalysis;
use crate::transition::{Curve, FilterSweep, XfConfig, XfShape};

/// Confidence above which the outgoing beat grid is trusted for bar
/// lengths and phrase alignment (`MIN_CONFIDENCE` is the floor for any use).
const SURE_BEAT: f64 = 2.2;
const MIN_SECS: f64 = 3.0;
const MAX_SECS: f64 = 16.0;
/// Fade used when nothing is known about the outgoing track.
pub const FALLBACK_SECS: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixPoint {
    /// Track time (seconds) when the blend should start.
    pub start_s: f64,
    /// Length of the blend.
    pub secs: f32,
}

/// When and how long to blend out of a track of `duration` seconds.
pub fn mix_point(a: Option<&TrackAnalysis>, duration: f64) -> MixPoint {
    let fallback = |end: f64| {
        let secs = (FALLBACK_SECS as f64).min(end * 0.3).max(0.5);
        MixPoint { start_s: (end - secs).max(0.0), secs: secs as f32 }
    };
    let Some(a) = a else {
        return fallback(duration);
    };
    if duration <= 1.0 {
        return fallback(duration);
    }
    // Where the music really ends. Ignore implausible values (analysis of
    // a different encode, or a track that is silent for minutes).
    let mut end = a.music_end_s as f64;
    if !end.is_finite() || end <= duration - 45.0 || end > duration + 0.5 {
        end = duration;
    }
    end = end.min(duration);

    let grid = a.tail.filter(|g| g.bpm.is_finite() && g.bpm > 40.0 && g.bpm < 220.0);
    let sure = grid.filter(|g| g.confidence >= SURE_BEAT);

    // Default length from the beat.
    let mut secs = match (sure, grid) {
        (Some(g), _) => {
            let beats = if g.bpm >= 110.0 { 32.0 } else { 16.0 };
            (beats * g.period_s()).clamp(6.0, MAX_SECS)
        }
        (None, Some(_)) => 8.0,
        (None, None) => FALLBACK_SECS as f64,
    };
    let mut start = end - secs;

    // Mix over a quiet outro when there is one.
    if let Some(outro) = outro_start(&a.wave, duration, end) {
        let len = end - outro;
        if len >= 4.0 {
            start = outro.max(end - MAX_SECS);
            secs = end - start;
        }
    }

    // Start on a bar line (rounding to the nearest one, never past the end).
    if let Some(g) = sure {
        let bar = 4.0 * g.period_s();
        if bar > 0.5 {
            let k = ((start - g.phase_s) / bar).round();
            let snapped = g.phase_s + k * bar;
            if end - snapped >= MIN_SECS && end - snapped <= MAX_SECS + bar {
                start = snapped;
                secs = end - start;
            }
        }
    }

    // Never more than a third of a short song.
    let cap = (duration * 0.34).max(MIN_SECS);
    if secs > cap {
        secs = cap;
        start = end - secs;
    }
    secs = secs.clamp(MIN_SECS.min(end), MAX_SECS + 4.0);
    MixPoint { start_s: start.max(0.0), secs: secs as f32 }
}

/// Start of a quiet ending (fade-out / outro) from the waveform overview:
/// the earliest point in the last 40 s after which the level stays well
/// under the body of the song. None when the song ends at full level.
fn outro_start(wave: &[u8], duration: f64, end: f64) -> Option<f64> {
    let n = wave.len();
    if n < 40 || duration <= 30.0 {
        return None;
    }
    let bin_s = duration / n as f64;
    let at = |t: f64| ((t / bin_s).floor() as usize).min(n - 1);
    // Typical loudness of the body (middle of the song).
    let mut body: Vec<u8> = wave[n / 10..n * 85 / 100].to_vec();
    body.sort_unstable();
    let median = body[body.len() / 2] as f64;
    if median < 30.0 {
        return None;
    }
    let quiet = median * 0.55;
    let last = at((end - 0.5).max(0.0));
    let first = at((end - 40.0).max(0.0));
    if (wave[last] as f64) >= quiet {
        return None;
    }
    let mut i = last;
    while i > first && (wave[i - 1] as f64) < quiet {
        i -= 1;
    }
    let t = i as f64 * bin_s;
    (end - t >= 2.0).then_some(t)
}

/// Transition settings AutoMix plays with: tempo matched with keylock,
/// beats aligned, silence trimmed, smooth curves and a bass swap.
pub fn config(base: &XfConfig) -> XfConfig {
    XfConfig {
        shape: XfShape {
            secs: base.shape.secs,
            curve_out: Curve::Smooth,
            curve_in: Curve::Smooth,
            out_len: 1.0,
            in_len: 1.0,
            filter: FilterSweep::Sweep,
        },
        smart: true,
        bpm_match: true,
        bpm_max_pct: base.bpm_max_pct.max(8.0),
        beat_align: true,
        bpm_return_secs: 12.0,
        keylock: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Grid;

    fn track(duration: f32, end: f32, tail: Option<Grid>, wave: Vec<u8>) -> TrackAnalysis {
        TrackAnalysis {
            v: 2,
            lufs: -9.0,
            peak: 0.9,
            duration_s: duration,
            lead_silence_s: 0.0,
            music_end_s: end,
            head: None,
            tail,
            at: 0,
            wave,
            key: None,
        }
    }

    #[test]
    fn no_analysis_is_a_short_fade_at_the_end() {
        let p = mix_point(None, 200.0);
        assert!((p.start_s + p.secs as f64 - 200.0).abs() < 1e-6);
        assert!(p.secs <= FALLBACK_SECS);
    }

    #[test]
    fn dance_track_mixes_eight_bars_on_a_bar_line() {
        let g = Grid { bpm: 128.0, phase_s: 0.2, confidence: 4.0 };
        let a = track(240.0, 238.0, Some(g), vec![200; 240]);
        let p = mix_point(Some(&a), 240.0);
        let bar = 4.0 * 60.0 / 128.0;
        // Ends at the music end, on a bar line, roughly 8 bars long.
        assert!((p.start_s + p.secs as f64 - 238.0).abs() < 0.01);
        let k = (p.start_s - 0.2) / bar;
        assert!((k - k.round()).abs() < 1e-6, "start {} not on a bar", p.start_s);
        assert!(p.secs >= 12.0 && p.secs <= 18.0, "{}", p.secs);
    }

    #[test]
    fn quiet_outro_is_where_the_mix_happens() {
        // Loud body, last ~12 s fade.
        let mut wave = vec![220u8; 240];
        for w in wave.iter_mut().skip(228) {
            *w = 60;
        }
        let a = track(240.0, 240.0, None, wave);
        let p = mix_point(Some(&a), 240.0);
        assert!((p.start_s - 228.0).abs() < 1.5, "start {}", p.start_s);
        assert!((p.secs - 12.0).abs() < 1.5);
    }

    #[test]
    fn trailing_silence_is_skipped() {
        let a = track(200.0, 185.0, None, vec![200; 240]);
        let p = mix_point(Some(&a), 200.0);
        assert!((p.start_s + p.secs as f64 - 185.0).abs() < 0.01);
    }

    #[test]
    fn short_song_is_never_mostly_blend() {
        let g = Grid { bpm: 90.0, phase_s: 0.0, confidence: 4.0 };
        let a = track(30.0, 30.0, Some(g), vec![200; 240]);
        let p = mix_point(Some(&a), 30.0);
        assert!(p.secs as f64 <= 30.0 * 0.34 + 0.01);
    }
}
