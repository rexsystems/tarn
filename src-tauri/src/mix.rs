//! Clickless envelopes, equal-power crossfades, and gapless handoff.
//!
//! A single long-lived rodio `Source` sits on the sink. Commands on the
//! audio thread swap or overlap decoders; the callback only mixes samples.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rodio::Source;

use crate::decode::{AudioSource, Pull};

/// Fade used for play / pause / resume / seek / skip (~150–300 ms).
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
            })),
            samples,
            sample_rate,
            channels,
            ended,
            phase,
            buffering,
            recover,
            has_src,
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
        g.current = Some(src);
        g.incoming = None;
        g.xf_len = 0;
        g.xf_pos = 0;
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
        g.current = Some(src);
        g.incoming = None;
        g.xf_len = 0;
        g.xf_pos = 0;
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
        let sr = src.sample_rate().max(1);
        let ch = src.channels().max(1);
        let mut g = self.inner.lock();
        // If sample rates differ, fall back to a short cut — rodio won't
        // resample a mid-stream rate change cleanly.
        if g.current.is_none() || g.sr != sr || g.ch != ch {
            drop(g);
            self.play(src, CLICK_FADE);
            return;
        }
        let secs = secs.clamp(0.05, 12.0);
        g.incoming = Some(src);
        g.xf_len = (secs * g.sr as f32).round().max(1.0) as u32;
        g.xf_pos = 0;
        g.finished = false;
        g.paused = false;
        if g.env < 0.5 {
            g.set_fade(1.0, CLICK_FADE);
        }
        self.ended.store(false, Ordering::Relaxed);
        self.phase.store(1, Ordering::Relaxed);
        self.samples.store(0, Ordering::SeqCst);
        self.has_src.store(true, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
    }

    /// Short fade-out of the current track, then fade-in of `src` (skip).
    pub fn cut(&self, src: AudioSource, fade: f32) {
        // Open the incoming source first; the fade-out happens in the
        // mixer and we swap when quiet. For a tight skip we do a brief
        // env dip then swap — implemented as a very short crossfade so
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
        g.env = 0.0;
        self.phase.store(0, Ordering::Relaxed);
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
        g.current = Some(src);
        g.incoming = None;
        g.xf_len = 0;
        g.finished = false;
        g.paused = false;
        g.env = 0.0;
        g.set_fade(1.0, CLICK_FADE);
        self.phase.store(1, Ordering::Relaxed);
        self.has_src.store(true, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
        self.recover.store(false, Ordering::Relaxed);
    }

    pub fn env_quiet(&self) -> bool {
        self.inner.lock().env < 0.02
    }

    pub fn fading_out(&self) {
        self.inner.lock().set_fade(0.0, CLICK_FADE);
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
        g.finished = false;
        g.paused = false;
        self.ended.store(false, Ordering::Relaxed);
        self.phase.store(1, Ordering::Relaxed);
        self.has_src.store(true, Ordering::Relaxed);
    }
}

impl Inner {
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
}

impl MixerSource {
    fn pull_frame(&mut self) -> Option<i16> {
        let mut g = self.inner.lock();
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
        if !frozen {
            if let Some(src) = g.current.as_mut() {
                a_alive = true;
                for s in a.iter_mut() {
                    match src.pull() {
                        Pull::Sample(v) => *s = v,
                        Pull::Buffering => {
                            a_buf = true;
                            break;
                        }
                        Pull::Ended => {
                            a_alive = false;
                            break;
                        }
                        Pull::Stall(e) => {
                            a_stall = Some(e);
                            a_alive = false;
                            break;
                        }
                    }
                }
            }
            // Only consume the incoming decoder during a crossfade. For
            // gapless, it stays queued and starts the instant current ends.
            if g.xf_len > 0 {
                if let Some(src) = g.incoming.as_mut() {
                    b_alive = true;
                    for s in b.iter_mut() {
                        match src.pull() {
                            Pull::Sample(v) => *s = v,
                            Pull::Buffering => {
                                b_buf = true;
                                break;
                            }
                            Pull::Ended => {
                                b_alive = false;
                                break;
                            }
                            Pull::Stall(_) => {
                                b_alive = false;
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Crossfade handoff.
        let mut mixed = a.clone();
        if g.xf_len > 0 && (b_alive || g.incoming.is_some()) {
            let t = (g.xf_pos as f32 / g.xf_len as f32).clamp(0.0, 1.0);
            // Equal-power: cos/sin of π/2 keeps perceived loudness constant.
            let go = (t * std::f32::consts::FRAC_PI_2).cos();
            let gi = (t * std::f32::consts::FRAC_PI_2).sin();
            for i in 0..ch {
                let va = a[i] as f32 * go;
                let vb = b[i] as f32 * gi;
                mixed[i] = (va + vb).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            }
            g.xf_pos = g.xf_pos.saturating_add(1);
            if g.xf_pos >= g.xf_len || !a_alive {
                g.current = g.incoming.take();
                g.xf_len = 0;
                g.xf_pos = 0;
                a_alive = b_alive || g.current.is_some();
            }
        } else if !a_alive {
            if g.incoming.is_some() {
                // Gapless: incoming takes over with no overlap and no silence.
                g.current = g.incoming.take();
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
        // The mixer owns the position clock. Do not increment while
        // buffering or paused — that was a source of "time runs, silence".
        // AudioSource used to also increment, doubling the clock (and
        // firing blend/end ~2× early, which skipped ahead).
        if a_alive && !frozen && !a_buf {
            self.samples
                .fetch_add(ch as u64, Ordering::Relaxed);
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
