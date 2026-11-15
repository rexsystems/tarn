//! Clickless envelopes, shaped crossfades, and gapless handoff.
//!
//! A single long-lived rodio `Source` sits on the sink. Commands on the
//! audio thread swap or overlap decoders; the callback only mixes samples.
//!
//! Crossfades can use per-side curves and lengths, an optional filter
//! sweep, a beat-aligned start, and varispeed tempo matching of the
//! incoming track (see `transition.rs`). With the default settings all of
//! that is inert and the output matches the classic equal-power blend.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rodio::Source;

use crate::decode::{AudioSource, Pull};
use crate::transition::{
    frames_to_next_beat, sweep_hz, Beat, Biquad, EnvBuilder, FrameRes, Vari, XfShape, SILENCE,
};

/// Seconds of the outgoing track kept for live tempo analysis.
const TAP_SECS: f32 = 12.0;

/// Everything a single crossfade needs beyond the incoming decoder.
#[derive(Clone, Debug)]
pub struct XfPlan {
    pub shape: XfShape,
    /// Playback rate of the incoming track during the overlap (1 = native).
    pub in_rate: f64,
    /// Outgoing beat grid (period, phase) in the live tap's frame clock;
    /// the overlap waits for the next beat when set.
    pub out_grid: Option<(f64, f64)>,
    /// Seconds for the new track to glide back to rate 1 after the overlap.
    pub return_secs: f32,
    /// Frames already skipped at the head of the incoming decoder
    /// (silence trim / beat alignment) so the position clock stays right.
    pub skipped_frames: u64,
    /// Shorten the overlap when the outgoing track has already gone quiet.
    pub smart: bool,
}

impl XfPlan {
    pub fn simple(secs: f32) -> Self {
        Self {
            shape: XfShape::equal_power(secs),
            in_rate: 1.0,
            out_grid: None,
            return_secs: 8.0,
            skipped_frames: 0,
            smart: false,
        }
    }
}
use crate::eq::Eq;

/// Fade used for play / pause / resume / seek / skip (~150-300 ms).
pub const CLICK_FADE: f32 = 0.22;

pub struct Mixer {
    inner: Arc<Mutex<Inner>>,
    samples: Arc<AtomicU64>,
    sample_rate: Arc<AtomicU32>,
    channels: Arc<AtomicU32>,
    ended: Arc<AtomicBool>,
    phase: Arc<AtomicU8>,
    buffering: Arc<AtomicBool>,
    recover: Arc<AtomicBool>,
    has_src: Arc<AtomicBool>,
    /// Milliseconds the current track has been continuously silent.
    quiet_ms: Arc<AtomicU32>,
}

/// Cheap handle used off the audio callback (analysis threads).
#[derive(Clone)]
pub struct MixTap {
    inner: Arc<Mutex<Inner>>,
}

impl MixTap {
    /// Tempo/phase of what the outgoing track has played recently, with the
    /// phase expressed in the tap's absolute frame clock.
    pub fn outgoing_beat(&self) -> Option<Beat> {
        let (full, low, start, sr) = {
            let g = self.inner.lock();
            let t = g.tap.as_ref()?;
            (
                t.full.iter().copied().collect::<Vec<_>>(),
                t.low.iter().copied().collect::<Vec<_>>(),
                t.window_start(),
                g.sr,
            )
        };
        let mut b = crate::transition::analyze(&full, &low, sr)?;
        b.phase += start as f64;
        Some(b)
    }

    pub fn set_analysis(&self, on: bool) {
        let mut g = self.inner.lock();
        if on && g.tap.is_none() {
            let sr = g.sr;
            g.tap = Some(EnvBuilder::new(sr, TAP_SECS));
        } else if !on {
            g.tap = None;
        }
    }
}

struct Inner {
    current: Option<AudioSource>,
    incoming: Option<AudioSource>,
    /// 0 = silent, 1 = full. Applied after the equal-power crossfade mix.
    env: f32,
    env_tgt: f32,
    env_step: f32,
    paused: bool,
    /// Crossfade length and position in frames (not interleaved samples).
    xf_len: u32,
    xf_pos: u32,
    sr: u32,
    ch: u16,
    /// True once the outgoing decoder ran out and nothing followed.
    finished: bool,
    /// Held frame so every channel shares the same envelope/xf value.
    frame: Vec<i16>,
    frame_i: usize,
    /// Crossfade shape (curves, lengths, filter).
    shape: XfShape,
    /// Output frames to keep playing the outgoing track alone before the
    /// overlap starts (beat alignment).
    xf_wait: u32,
    cur_vari: Vari,
    inc_vari: Vari,
    return_secs: f32,
    lp_out: Vec<Biquad>,
    hp_in: Vec<Biquad>,
    /// Live envelope of the current track for tempo analysis (None = off).
    tap: Option<EnvBuilder>,
    quiet_run: u32,
    clock_acc: f64,
    eq: Eq,
}

impl Mixer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        samples: Arc<AtomicU64>,
        sample_rate: Arc<AtomicU32>,
        channels: Arc<AtomicU32>,
        ended: Arc<AtomicBool>,
        phase: Arc<AtomicU8>,
        buffering: Arc<AtomicBool>,
        recover: Arc<AtomicBool>,
        has_src: Arc<AtomicBool>,
        quiet_ms: Arc<AtomicU32>,
    ) -> Self {
        let sr = 44100;
        let ch = 2u16;
        sample_rate.store(sr, Ordering::Relaxed);
        channels.store(ch as u32, Ordering::Relaxed);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                current: None,
                incoming: None,
                env: 0.0,
                env_tgt: 0.0,
                env_step: 0.0,
                paused: false,
                xf_len: 0,
                xf_pos: 0,
                sr,
                ch,
                finished: false,
                frame: Vec::new(),
                frame_i: 0,
                shape: XfShape::equal_power(0.0),
                xf_wait: 0,
                cur_vari: Vari::default(),
                inc_vari: Vari::default(),
                return_secs: 8.0,
                lp_out: Vec::new(),
                hp_in: Vec::new(),
                tap: None,
                quiet_run: 0,
                clock_acc: 0.0,
                eq: Eq::new(sr),
            })),
            samples,
            sample_rate,
            channels,
            ended,
            phase,
            buffering,
            recover,
            has_src,
            quiet_ms,
        }
    }

    pub fn tap(&self) -> MixTap {
        MixTap {
            inner: self.inner.clone(),
        }
    }

    pub fn source(&self) -> MixerSource {
        MixerSource {
            inner: self.inner.clone(),
            samples: self.samples.clone(),
            ended: self.ended.clone(),
            phase: self.phase.clone(),
            buffering: self.buffering.clone(),
            recover: self.recover.clone(),
            quiet_ms: self.quiet_ms.clone(),
        }
    }

    /// Load a decoder at the given position and stay silent, paused.
    /// Used when restoring a session so Play is instant.
    pub fn arm(&self, src: AudioSource) {
        let sr = src.sample_rate().max(1);
        let ch = src.channels().max(1);
        self.sample_rate.store(sr, Ordering::Relaxed);
        self.channels.store(ch as u32, Ordering::Relaxed);
        self.samples.store(0, Ordering::SeqCst);
        self.ended.store(false, Ordering::Relaxed);
        let mut g = self.inner.lock();
        g.sr = sr;
        g.ch = ch;
        g.eq.set_sample_rate(sr);
        g.current = Some(src);
        g.incoming = None;
        g.xf_len = 0;
        g.xf_pos = 0;
        g.new_current();
        g.finished = false;
        g.paused = true;
        g.env = 0.0;
        g.env_tgt = 0.0;
        g.env_step = 0.0;
        self.phase.store(2, Ordering::Relaxed);
        self.has_src.store(true, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
        self.recover.store(false, Ordering::Relaxed);
    }

    pub fn play(&self, src: AudioSource, fade_in: f32) {
        let sr = src.sample_rate().max(1);
        let ch = src.channels().max(1);
        self.sample_rate.store(sr, Ordering::Relaxed);
        self.channels.store(ch as u32, Ordering::Relaxed);
        self.samples.store(0, Ordering::SeqCst);
        self.ended.store(false, Ordering::Relaxed);
        let mut g = self.inner.lock();
        g.sr = sr;
        g.ch = ch;
        g.eq.set_sample_rate(sr);
        g.current = Some(src);
        g.incoming = None;
        g.xf_len = 0;
        g.xf_pos = 0;
        g.new_current();
        g.finished = false;
        g.paused = false;
        g.env = 0.0;
        g.set_fade(1.0, fade_in);
        self.phase.store(1, Ordering::Relaxed);
        self.has_src.store(true, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
        self.recover.store(false, Ordering::Relaxed);
    }

    /// Equal-power overlap of the current decoder with `src`.
    pub fn crossfade(&self, src: AudioSource, secs: f32) {
        self.crossfade_plan(src, XfPlan::simple(secs));
    }

    /// Shaped overlap of the current decoder with `src`.
    pub fn crossfade_plan(&self, src: AudioSource, plan: XfPlan) {
        let sr = src.sample_rate().max(1);
        let ch = src.channels().max(1);
        let mut g = self.inner.lock();
        // If sample rates differ, fall back to a short cut - rodio won't
        // resample a mid-stream rate change cleanly.
        if g.current.is_none() || g.sr != sr || g.ch != ch {
            drop(g);
            self.play(src, CLICK_FADE);
            // play() zeros the clock; re-apply silence/beat trim so the
            // progress bar matches the audio that already skipped ahead.
            self.apply_skipped_frames(plan.skipped_frames);
            return;
        }
        let mut secs = plan.shape.secs.clamp(0.05, 20.0);
        // Smart start: the outgoing track already went quiet - don't make
        // the listener sit through a long fade-in from silence.
        if plan.smart && g.quiet_run as f32 > 0.5 * g.sr as f32 {
            secs = secs.min(1.0);
        }
        g.incoming = Some(src);
        g.xf_len = (secs * g.sr as f32).round().max(1.0) as u32;
        g.xf_pos = 0;
        g.shape = plan.shape.clone();
        g.shape.secs = secs;
        g.return_secs = plan.return_secs;
        g.inc_vari = Vari::with_rate(plan.in_rate);
        g.lp_out = vec![Biquad::default(); ch as usize];
        g.hp_in = vec![Biquad::default(); ch as usize];
        g.xf_wait = 0;
        if let (Some((period, phase)), Some(tap)) = (plan.out_grid, g.tap.as_ref()) {
            let now = tap.frames as f64;
            let wait = frames_to_next_beat(now, phase, period);
            // Never hold the overlap back by more than a beat (or 1 s).
            if wait <= period && wait <= g.sr as f64 {
                let rate = g.cur_vari.rate.max(0.5);
                g.xf_wait = (wait / rate).round() as u32;
            }
        }
        g.finished = false;
        g.paused = false;
        if g.env < 0.5 {
            g.set_fade(1.0, CLICK_FADE);
        }
        g.clock_acc = 0.0;
        self.ended.store(false, Ordering::Relaxed);
        self.phase.store(1, Ordering::Relaxed);
        self.apply_skipped_frames(plan.skipped_frames);
        self.has_src.store(true, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
    }

    /// Seed the position clock after smart-start / beat-align trim.
    pub fn apply_skipped_frames(&self, frames: u64) {
        let ch = self.channels.load(Ordering::Relaxed).max(1) as u64;
        self.samples
            .store(frames.saturating_mul(ch), Ordering::SeqCst);
    }

    /// Short fade-out of the current track, then fade-in of `src` (skip).
    pub fn cut(&self, src: AudioSource, fade: f32) {
        // Open the incoming source first; the fade-out happens in the
        // mixer and we swap when quiet. For a tight skip we do a brief
        // env dip then swap - implemented as a very short crossfade so
        // both streams stay aligned with equal-power (no pop).
        let fade = fade.clamp(0.05, 0.5);
        self.crossfade(src, fade);
    }

    pub fn pause(&self) {
        let mut g = self.inner.lock();
        g.set_fade(0.0, CLICK_FADE);
        g.paused = true;
        self.phase.store(2, Ordering::Relaxed);
    }

    pub fn resume(&self) -> bool {
        let mut g = self.inner.lock();
        if g.current.is_none() {
            return false;
        }
        g.paused = false;
        g.set_fade(1.0, CLICK_FADE);
        self.phase.store(1, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
        true
    }

    #[allow(dead_code)]
    pub fn has_current(&self) -> bool {
        self.has_src.load(Ordering::Relaxed) && self.inner.lock().current.is_some()
    }

    pub fn stop(&self) {
        let mut g = self.inner.lock();
        g.set_fade(0.0, CLICK_FADE * 0.7);
        g.incoming = None;
        g.xf_len = 0;
        g.paused = true;
        g.current = None;
        g.new_current();
        g.env = 0.0;
        self.phase.store(0, Ordering::Relaxed);
        self.has_src.store(false, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
    }

    /// Drop the playing decoder without touching phase - used while the
    /// player UI is already in loading for the next track.
    pub fn hush(&self) {
        let mut g = self.inner.lock();
        g.set_fade(0.0, CLICK_FADE * 0.7);
        g.incoming = None;
        g.xf_len = 0;
        g.paused = true;
        g.current = None;
        g.new_current();
        g.env = 0.0;
        self.has_src.store(false, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
    }

    pub fn seek_swap(&self, src: AudioSource) {
        let sr = src.sample_rate().max(1);
        let ch = src.channels().max(1);
        self.sample_rate.store(sr, Ordering::Relaxed);
        self.channels.store(ch as u32, Ordering::Relaxed);
        let mut g = self.inner.lock();
        g.sr = sr;
        g.ch = ch;
        g.eq.set_sample_rate(sr);
        g.current = Some(src);
        g.incoming = None;
        g.xf_len = 0;
        g.new_current();
        g.finished = false;
        g.paused = false;
        g.env = 0.0;
        g.set_fade(1.0, CLICK_FADE);
        self.phase.store(1, Ordering::Relaxed);
        self.has_src.store(true, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
        self.recover.store(false, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.inner.lock().paused
    }

    pub fn env_quiet(&self) -> bool {
        self.inner.lock().env < 0.02
    }

    pub fn fading_out(&self) {
        self.inner.lock().set_fade(0.0, CLICK_FADE);
    }

    pub fn set_eq(&self, gains: [f32; crate::eq::BANDS], enabled: bool) {
        let mut g = self.inner.lock();
        g.eq.set(gains, enabled);
    }


    /// Queue `src` to start the instant the current decoder ends (no overlap,
    /// no silence). Used when crossfade is off, and within an album.
    pub fn queue_gapless(&self, src: AudioSource) {
        let sr = src.sample_rate().max(1);
        let ch = src.channels().max(1);
        let mut g = self.inner.lock();
        if g.current.is_none() || g.sr != sr || g.ch != ch {
            drop(g);
            self.play(src, 0.04);
            return;
        }
        g.incoming = Some(src);
        g.xf_len = 0;
        g.xf_pos = 0;
        g.xf_wait = 0;
        g.inc_vari = Vari::default();
        g.finished = false;
        g.paused = false;
        self.ended.store(false, Ordering::Relaxed);
        self.phase.store(1, Ordering::Relaxed);
        self.has_src.store(true, Ordering::Relaxed);
    }
}

impl Inner {
    /// Reset per-track state when `current` is replaced outright.
    fn new_current(&mut self) {
        self.xf_wait = 0;
        self.cur_vari = Vari::default();
        self.inc_vari = Vari::default();
        self.quiet_run = 0;
        self.clock_acc = 0.0;
        if let Some(t) = self.tap.as_mut() {
            t.reset();
        }
    }

    /// The incoming decoder becomes current (end of overlap or gapless).
    fn promote_incoming(&mut self) {
        self.current = self.incoming.take();
        self.cur_vari = std::mem::take(&mut self.inc_vari);
        if self.cur_vari.active() {
            let (secs, sr) = (self.return_secs as f64, self.sr);
            self.cur_vari.ramp_to(1.0, secs, sr);
        }
        self.xf_wait = 0;
        self.quiet_run = 0;
        self.clock_acc = 0.0;
        if let Some(t) = self.tap.as_mut() {
            t.reset();
        }
    }

    fn set_fade(&mut self, tgt: f32, secs: f32) {
        self.env_tgt = tgt.clamp(0.0, 1.0);
        let frames = (secs.max(0.04) * self.sr as f32).max(1.0);
        let dist = (self.env_tgt - self.env).abs().max(0.001);
        self.env_step = dist / frames;
    }

    fn step_env(&mut self) {
        if (self.env - self.env_tgt).abs() <= self.env_step {
            self.env = self.env_tgt;
        } else if self.env < self.env_tgt {
            self.env += self.env_step;
        } else {
            self.env -= self.env_step;
        }
    }
}

pub struct MixerSource {
    inner: Arc<Mutex<Inner>>,
    samples: Arc<AtomicU64>,
    ended: Arc<AtomicBool>,
    phase: Arc<AtomicU8>,
    buffering: Arc<AtomicBool>,
    recover: Arc<AtomicBool>,
    quiet_ms: Arc<AtomicU32>,
}

/// Read one interleaved frame straight from a decoder.
pub fn pull_raw(src: &mut AudioSource, out: &mut [i16]) -> FrameRes {
    for s in out.iter_mut() {
        match src.pull() {
            Pull::Sample(v) => *s = v,
            Pull::Buffering => return FrameRes::Buffering,
            Pull::Ended => return FrameRes::Ended,
            Pull::Stall(e) => return FrameRes::Stall(e),
        }
    }
    FrameRes::Ok
}

impl MixerSource {
    fn pull_frame(&mut self) -> Option<i16> {
        let mut guard = self.inner.lock();
        let g = &mut *guard;
        if g.frame_i < g.frame.len() {
            let v = g.frame[g.frame_i];
            g.frame_i += 1;
            return Some(v);
        }
        let ch = g.ch.max(1) as usize;
        let mut a = vec![0i16; ch];
        let mut b = vec![0i16; ch];
        let mut a_alive = false;
        let mut b_alive = false;

        let frozen = g.paused && g.env <= 0.02 && g.incoming.is_none();
        let mut a_buf = false;
        let mut a_stall: Option<String> = None;
        let mut b_buf = false;
        // Beat alignment: the outgoing track plays alone until its next beat.
        let waiting = g.xf_len > 0 && g.xf_wait > 0 && g.incoming.is_some();
        if !frozen {
            if let Some(src) = g.current.as_mut() {
                a_alive = true;
                match g.cur_vari.pull(&mut a, |f| pull_raw(src, f)) {
                    FrameRes::Ok => {}
                    FrameRes::Buffering => a_buf = true,
                    FrameRes::Ended => a_alive = false,
                    FrameRes::Stall(e) => {
                        a_stall = Some(e);
                        a_alive = false;
                    }
                }
                if a_alive && !a_buf {
                    if let Some(t) = g.tap.as_mut() {
                        t.push_frame(&a);
                    }
                    let peak = a.iter().map(|&v| (v as i32).abs()).max().unwrap_or(0);
                    if peak < SILENCE {
                        g.quiet_run = g.quiet_run.saturating_add(1);
                    } else {
                        g.quiet_run = 0;
                    }
                    if g.quiet_run.is_multiple_of(512) {
                        let ms = (g.quiet_run as u64 * 1000 / g.sr.max(1) as u64) as u32;
                        self.quiet_ms.store(ms, Ordering::Relaxed);
                    }
                }
            }
            // Only consume the incoming decoder during a crossfade. For
            // gapless, it stays queued and starts the instant current ends.
            if g.xf_len > 0 && (!waiting || !a_alive) {
                if let Some(src) = g.incoming.as_mut() {
                    b_alive = true;
                    match g.inc_vari.pull(&mut b, |f| pull_raw(src, f)) {
                        FrameRes::Ok => {}
                        FrameRes::Buffering => b_buf = true,
                        FrameRes::Ended | FrameRes::Stall(_) => b_alive = false,
                    }
                }
            }
        }
        if waiting {
            if a_alive {
                g.xf_wait -= 1;
            } else {
                g.xf_wait = 0;
            }
        }
        // Which deck drives the position clock this frame, and how fast.
        let mut clock_rate = g.cur_vari.rate;

        // Crossfade handoff.
        let mut mixed = a.clone();
        if g.xf_len > 0 && waiting && a_alive {
            // Still lined up for the beat: outgoing alone, clock held.
            clock_rate = 0.0;
        } else if g.xf_len > 0 && (b_alive || g.incoming.is_some()) {
            let t = (g.xf_pos as f32 / g.xf_len as f32).clamp(0.0, 1.0);
            // Default shape is equal-power: cos/sin of π/2 keeps perceived
            // loudness constant.
            let (go, gi) = g.shape.gains(t);
            let filter = g.shape.filter;
            if filter.lowpass_out() || filter.highpass_in() {
                let sr = g.sr as f32;
                if g.xf_pos.is_multiple_of(32) {
                    let to = g.shape.out_progress(t);
                    let ti = g.shape.in_progress(t);
                    let lp = sweep_hz(18_000.0, 280.0, to);
                    let hp = sweep_hz(900.0, 20.0, ti);
                    for f in g.lp_out.iter_mut() {
                        f.set_lowpass(lp, sr);
                    }
                    for f in g.hp_in.iter_mut() {
                        f.set_highpass(hp, sr);
                    }
                }
                for i in 0..ch.min(g.lp_out.len()) {
                    if filter.lowpass_out() {
                        a[i] = g.lp_out[i]
                            .process(a[i] as f32)
                            .round()
                            .clamp(i16::MIN as f32, i16::MAX as f32)
                            as i16;
                    }
                    if filter.highpass_in() {
                        b[i] = g.hp_in[i]
                            .process(b[i] as f32)
                            .round()
                            .clamp(i16::MIN as f32, i16::MAX as f32)
                            as i16;
                    }
                }
            }
            for i in 0..ch {
                let va = a[i] as f32 * go;
                let vb = b[i] as f32 * gi;
                mixed[i] = (va + vb).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            }
            clock_rate = g.inc_vari.rate;
            g.xf_pos = g.xf_pos.saturating_add(1);
            if g.xf_pos >= g.xf_len || !a_alive {
                g.promote_incoming();
                g.xf_len = 0;
                g.xf_pos = 0;
                a_alive = b_alive || g.current.is_some();
            }
        } else if !a_alive {
            if g.incoming.is_some() {
                // Gapless: incoming takes over with no overlap and no silence.
                g.promote_incoming();
                mixed = b;
                a_alive = b_alive;
            } else if a_stall.is_some() {
                // Mid-play 403/stall: drop the dead decoder, ask for a
                // re-resolve at the current position. Never auto-advance.
                let _ = g.current.take();
                g.finished = false;
                if let Some(msg) = a_stall.as_ref() {
                    eprintln!(
                        "playback stall: {msg}; re-resolving, not advancing"
                    );
                }
                self.recover.store(true, Ordering::SeqCst);
                self.buffering.store(true, Ordering::Relaxed);
                self.phase.store(4, Ordering::Relaxed);
            } else if a_buf {
                // Still the same track; just waiting on the downloader.
                a_alive = true;
            } else if !frozen && g.current.take().is_some() {
                g.finished = true;
            }
        }

        if a_buf || b_buf {
            if !self.buffering.swap(true, Ordering::Relaxed) {
                eprintln!("underrun: pausing clock, waiting on downloader");
            }
            let ph = self.phase.load(Ordering::Relaxed);
            if ph == 1 {
                self.phase.store(4, Ordering::Relaxed);
            }
        } else if a_alive && self.buffering.swap(false, Ordering::Relaxed) {
            if self.phase.load(Ordering::Relaxed) == 4 {
                self.phase.store(1, Ordering::Relaxed);
            }
            eprintln!("underrun: resumed, clock running");
        }

        if g.finished && g.incoming.is_none() && g.current.is_none() && a_stall.is_none() {
            self.ended.store(true, Ordering::Relaxed);
            self.phase.store(0, Ordering::Relaxed);
            self.buffering.store(false, Ordering::Relaxed);
            g.frame = vec![0; ch];
            g.frame_i = 1;
            return Some(0);
        }

        g.step_env();
        // Cosine ease on the click envelope so pause/resume is also smooth.
        let gain = (g.env * std::f32::consts::FRAC_PI_2).sin();
        for s in mixed.iter_mut() {
            *s = (*s as f32 * gain).round() as i16;
        }
        g.eq.process_frame(&mut mixed, ch);
        // The mixer owns the position clock. Do not increment while
        // buffering or paused - that was a source of "time runs, silence".
        // AudioSource used to also increment, doubling the clock (and
        // firing blend/end ~2× early, which skipped ahead).
        // Varispeed advances the clock by the source frames consumed, so
        // at the normal rate of 1 this adds exactly one frame per frame.
        if a_alive && !frozen && !a_buf {
            g.clock_acc += clock_rate;
            let whole = g.clock_acc.floor();
            if whole >= 1.0 {
                g.clock_acc -= whole;
                self.samples
                    .fetch_add(ch as u64 * whole as u64, Ordering::Relaxed);
            }
        }
        g.frame = mixed;
        g.frame_i = 1;
        g.frame.first().copied()
    }
}

impl Iterator for MixerSource {
    type Item = i16;
    fn next(&mut self) -> Option<Self::Item> {
        // Always yield: the sink stays attached for the life of the thread.
        Some(self.pull_frame().unwrap_or(0))
    }
}

impl Source for MixerSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.inner.lock().ch.max(1)
    }
    fn sample_rate(&self) -> u32 {
        self.inner.lock().sr.max(1)
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
