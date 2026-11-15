use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use rodio::{OutputStream, Sink};

use crate::models::{PlayerState, Track};
use crate::transition::XfConfig;
use crate::ytm::Stream;

/// Commands sent to the audio thread.
enum Cmd {
    Play(Arc<crate::decode::GrowingBody>, f64),
    Arm(Arc<crate::decode::GrowingBody>, f64),
    Crossfade(Arc<crate::decode::GrowingBody>, f64, Box<XfConfig>),
    /// Analysis for a crossfade finished off-thread; `u64` is the command
    /// generation it was started for (stale results are dropped).
    XfReady(Box<crate::decode::AudioSource>, Box<crate::mix::XfPlan>, u64),
    Gapless(Arc<crate::decode::GrowingBody>, f64),
    Cut(Arc<crate::decode::GrowingBody>, f64),
    Pause,
    Resume,
    Stop,
    /// Drop the current decoder without flipping UI phase (keeps loading).
    Hush,
    Seek(f64),
    Volume(f32),
    Eq([f32; crate::eq::BANDS], bool),
}

/// How to start a new decoder relative to whatever is already playing.
#[derive(Clone, Copy, Debug)]
pub enum PlayStyle {
    /// Fade in from silence (first track, or a hard start).
    Fresh,
    /// Decode and sit paused at `start` (session restore).
    Armed,
    /// Short equal-power dip when skipping.
    Cut,
    /// Shaped overlap of `f32` seconds (curves/BPM from the transition
    /// settings).
    Crossfade(f32),
    /// Queue the decoder to start the instant the current one ends.
    Gapless,
}

#[derive(Clone)]
pub struct Player {
    tx: std::sync::mpsc::Sender<Cmd>,
    state: Arc<Mutex<PlayerState>>,
    /// Playback position in samples (f32 interleaved) from the start of the
    /// current source, plus the seek origin stored in `state.position`.
    samples: Arc<AtomicU64>,
    sample_rate: Arc<AtomicU32>,
    channels: Arc<AtomicU32>,
    ended: Arc<AtomicBool>,
    /// 0 = idle, 1 = playing, 2 = paused, 3 = loading
    phase: Arc<AtomicU8>,
    current_audio: Arc<Mutex<Option<Arc<crate::decode::GrowingBody>>>>,
    xf_on: Arc<AtomicBool>,
    xf_ms: Arc<AtomicU32>,
    xf_manual: Arc<AtomicBool>,
    album_cont: Arc<AtomicBool>,
    /// Curves, filter, smart start, and BPM matching settings.
    xf_cfg: Arc<Mutex<XfConfig>>,
    /// How long the current track has been silent (smart start).
    quiet_ms: Arc<AtomicU32>,
    blending: Arc<AtomicBool>,
    /// If the user hits play while a session is still arming.
    want_play: Arc<AtomicBool>,
    buffering: Arc<AtomicBool>,
    recover: Arc<AtomicBool>,
    has_src: Arc<AtomicBool>,
    recovering: Arc<AtomicBool>,
    last_ended: Arc<Mutex<Option<String>>>,
    last_recover: Arc<Mutex<Option<Instant>>>,
    /// Set by adopt_track after a successful blend; take_ended must not
    /// also advance (that restarted the same song or skipped ahead).
    blend_adopted: Arc<AtomicBool>,
    /// Bumped on every prepare/play so a stale crossfade from the previous
    /// queue cannot adopt into the new one.
    play_gen: Arc<AtomicU64>,
    /// Next track to show in the UI once the mixer actually starts it.
    /// Avoids "new title, old audio" while crossfade analysis is in flight.
    pending_adopt: Arc<Mutex<Option<(Track, usize, u64)>>>,
    /// Original playlist order while shuffle is on (for restore + Up next).
    linear_queue: Arc<Mutex<Option<Vec<Track>>>>,
}

impl Player {
    pub fn start() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
        let state = Arc::new(Mutex::new(PlayerState::default()));
        let samples = Arc::new(AtomicU64::new(0));
        let sample_rate = Arc::new(AtomicU32::new(44100));
        let channels = Arc::new(AtomicU32::new(2));
        let ended = Arc::new(AtomicBool::new(false));
        let phase = Arc::new(AtomicU8::new(0));
        let current_audio = Arc::new(Mutex::new(None));
        let xf_on = Arc::new(AtomicBool::new(true));
        let xf_ms = Arc::new(AtomicU32::new(4000));
        let xf_manual = Arc::new(AtomicBool::new(true));
        let album_cont = Arc::new(AtomicBool::new(true));
        let xf_cfg = Arc::new(Mutex::new(XfConfig::simple(4.0)));
        let quiet_ms = Arc::new(AtomicU32::new(0));
        let blending = Arc::new(AtomicBool::new(false));
        let want_play = Arc::new(AtomicBool::new(false));
        let buffering = Arc::new(AtomicBool::new(false));
        let recover = Arc::new(AtomicBool::new(false));
        let has_src = Arc::new(AtomicBool::new(false));
        let recovering = Arc::new(AtomicBool::new(false));
        let last_ended = Arc::new(Mutex::new(None));
        let last_recover = Arc::new(Mutex::new(None));
        let blend_adopted = Arc::new(AtomicBool::new(false));
        let play_gen = Arc::new(AtomicU64::new(0));
        let pending_adopt = Arc::new(Mutex::new(None));
        let linear_queue = Arc::new(Mutex::new(None));

        let samples_t = samples.clone();
        let rate_t = sample_rate.clone();
        let ch_t = channels.clone();
        let ended_t = ended.clone();
        let phase_t = phase.clone();
        let audio_t = current_audio.clone();
        let state_t = state.clone();
        let buffering_t = buffering.clone();
        let recover_t = recover.clone();
        let has_src_t = has_src.clone();
        let tx_t = tx.clone();
        let cfg_t = xf_cfg.clone();
        let quiet_t = quiet_ms.clone();
        let pending_t = pending_adopt.clone();
        let play_gen_t = play_gen.clone();
        let blend_adopted_t = blend_adopted.clone();
        let quiet_ms_t = quiet_ms.clone();
        let blending_t = blending.clone();
        let last_ended_t = last_ended.clone();

        std::thread::Builder::new()
            .name("tarn-audio".into())
            .spawn(move || {
                audio_thread(
                    rx,
                    samples_t,
                    rate_t,
                    ch_t,
                    ended_t,
                    phase_t,
                    audio_t,
                    state_t,
                    buffering_t,
                    recover_t,
                    has_src_t,
                    AudioExtras {
                        tx: tx_t,
                        cfg: cfg_t,
                        quiet_ms: quiet_t,
                        pending_adopt: pending_t,
                        play_gen: play_gen_t,
                        blend_adopted: blend_adopted_t,
                        quiet_clear: quiet_ms_t,
                        blending: blending_t,
                        last_ended: last_ended_t,
                    },
                )
            })
            .expect("audio thread");

        Self {
            tx,
            state,
            samples,
            sample_rate,
            channels,
            ended,
            phase,
            current_audio,
            xf_on,
            xf_ms,
            xf_manual,
            album_cont,
            xf_cfg,
            quiet_ms,
            blending,
            want_play,
            buffering,
            recover,
            has_src,
            recovering,
            last_ended,
            last_recover,
            blend_adopted,
            play_gen,
            pending_adopt,
            linear_queue,
        }
    }

    fn same_track_set(a: &[Track], b: &[Track]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        let mut aa: Vec<&str> = a.iter().map(|t| t.id.as_str()).collect();
        let mut bb: Vec<&str> = b.iter().map(|t| t.id.as_str()).collect();
        aa.sort_unstable();
        bb.sort_unstable();
        aa == bb
    }

    /// Put the current track first, shuffle the rest. Updates `index` to 0.
    fn shuffle_queue_view(queue: &mut Vec<Track>, index: &mut usize) {
        use rand::seq::SliceRandom;
        if queue.len() <= 1 {
            *index = 0;
            return;
        }
        let cur = (*index).min(queue.len() - 1);
        let current = queue[cur].clone();
        let mut rest: Vec<Track> = queue
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != cur)
            .map(|(_, t)| t.clone())
            .collect();
        rest.shuffle(&mut rand::thread_rng());
        *queue = std::iter::once(current).chain(rest).collect();
        *index = 0;
    }

    pub fn play_gen(&self) -> u64 {
        self.play_gen.load(Ordering::SeqCst)
    }

    pub fn snapshot(&self) -> PlayerState {
        let mut s = self.state.lock().clone();
        let rate = self.sample_rate.load(Ordering::Relaxed).max(1) as f64
            * self.channels.load(Ordering::Relaxed).max(1) as f64;
        let extra = self.samples.load(Ordering::Relaxed) as f64 / rate;
        let ph = self.phase.load(Ordering::Relaxed);
        // `position` in state is the seek origin; add consumed samples.
        // Phase 4 (buffering) keeps the last position: the mixer pauses the
        // clock, so `extra` does not run ahead of the audio.
        if ph != 0 {
            s.position = (s.position + extra).min(if s.duration > 0.0 {
                s.duration
            } else {
                f64::MAX
            });
        }
        s.playing = ph == 1 || ph == 4;
        s.loading = ph == 3;
        let body = self.current_audio.lock().clone();
        // Only the mixer/phase own the UI buffering bit. OR-ing the body's
        // transient wait flag left the play button spinning while audio was
        // already flowing (e.g. after a seek wait_len that never cleared it).
        s.buffering = ph == 4 || self.buffering.load(Ordering::Relaxed);
        s.buffered = match body {
            Some(b) => b.buffered_secs(s.duration),
            None => if s.duration > 0.0 && !s.loading { s.duration } else { 0.0 },
        };
        s
    }

    pub fn set_loading(&self, track: Track, queue: Vec<Track>, index: usize) {
        self.prepare(track, queue, index, 0.0, true);
    }

    pub fn prepare(
        &self,
        track: Track,
        queue: Vec<Track>,
        index: usize,
        position: f64,
        loading: bool,
    ) {
        // Invalidate any in-flight end-of-track blend from the previous queue.
        self.play_gen.fetch_add(1, Ordering::SeqCst);
        *self.pending_adopt.lock() = None;
        self.ended.store(false, Ordering::Relaxed);
        self.recover.store(false, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
        self.blending.store(false, Ordering::Relaxed);
        self.blend_adopted.store(false, Ordering::SeqCst);
        *self.last_ended.lock() = None;
        self.quiet_ms.store(0, Ordering::Relaxed);
        self.phase
            .store(if loading { 3 } else { 2 }, Ordering::Relaxed);
        self.want_play.store(false, Ordering::Relaxed);
        let mut track = track;
        track.ensure_cover();
        let mut queue = queue;
        for t in &mut queue {
            t.ensure_cover();
        }
        let mut s = self.state.lock();
        if let Some(d) = track.duration {
            if d > 0 {
                s.duration = d as f64;
            }
        }
        s.track = Some(track);
        s.loading = loading;
        s.playing = false;
        s.position = position.max(0.0);
        s.error = None;
        if s.shuffle {
            let mut lin = self.linear_queue.lock();
            if lin
                .as_ref()
                .is_some_and(|orig| Self::same_track_set(orig, &queue))
            {
                // Same playlist (e.g. click in Up next) - keep linear, keep order.
                s.queue = queue;
                s.index = index;
            } else {
                // New playlist while shuffle is on: remember linear, show shuffled.
                *lin = Some(queue.clone());
                let mut q = queue;
                let mut i = index;
                Self::shuffle_queue_view(&mut q, &mut i);
                s.queue = q;
                s.index = i;
            }
        } else {
            *self.linear_queue.lock() = None;
            s.queue = queue;
            s.index = index;
        }
    }



    pub fn set_accent(&self, rgb: Option<[u8; 3]>) {
        self.state.lock().accent = rgb;
    }

    pub fn set_error(&self, msg: String) {
        self.phase.store(0, Ordering::Relaxed);
        let mut s = self.state.lock();
        s.loading = false;
        s.playing = false;
        s.error = Some(msg);
    }

    pub fn set_duration(&self, d: f64) {
        let mut s = self.state.lock();
        if d > 0.0 {
            s.duration = d;
            if let Some(b) = self.current_audio.lock().as_ref() {
                b.set_duration(d);
            }
        }
    }

    pub fn play_stream(
        &self,
        _stream: Stream,
        audio: Arc<crate::decode::GrowingBody>,
        start: f64,
        style: PlayStyle,
    ) -> Result<()> {
        {
            let dur = self.state.lock().duration;
            if dur > 0.0 {
                audio.set_duration(dur);
            }
        }
        *self.current_audio.lock() = Some(audio.clone());
        let blendish = matches!(
            style,
            PlayStyle::Crossfade(_) | PlayStyle::Gapless | PlayStyle::Cut
        );
        // Natural end-blend queues adopt first; keep the outgoing clock so
        // the UI doesn't show the next title at 0:00 while old audio plays.
        // Manual next/prepare already swapped the track - reset as usual.
        let keep_clock = self.pending_adopt.lock().is_some();
        if !keep_clock {
            self.samples.store(0, Ordering::Relaxed);
            let mut s = self.state.lock();
            s.position = start;
        }
        self.ended.store(false, Ordering::Relaxed);
        if !blendish {
            *self.last_ended.lock() = None;
        }
        self.recovering.store(false, Ordering::Relaxed);
        self.blending.store(blendish, Ordering::Relaxed);
        let cmd = match style {
            PlayStyle::Fresh => Cmd::Play(audio, start),
            PlayStyle::Armed => {
                if self.want_play.load(Ordering::Relaxed) {
                    Cmd::Play(audio, start)
                } else {
                    Cmd::Arm(audio, start)
                }
            }
            PlayStyle::Cut => Cmd::Cut(audio, start),
            PlayStyle::Crossfade(s) => {
                let mut cfg = self.xf_cfg.lock().clone();
                cfg.shape.secs = s;
                Cmd::Crossfade(audio, start, Box::new(cfg))
            }
            PlayStyle::Gapless => Cmd::Gapless(audio, start),
        };
        self.tx
            .send(cmd)
            .map_err(|_| anyhow!("audio thread died"))?;
        // The blend itself is underway; clear the "due" latch so the next
        // track can be armed later. User-skip blending is already applied.
        if matches!(style, PlayStyle::Cut | PlayStyle::Fresh | PlayStyle::Armed) {
            self.blending.store(false, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn set_crossfade(&self, on: bool, seconds: f64, album_continuity: bool, manual: bool) {
        self.xf_on.store(on, Ordering::Relaxed);
        let ms = (seconds.clamp(0.0, 20.0) * 1000.0).round() as u32;
        self.xf_ms.store(ms, Ordering::Relaxed);
        self.album_cont.store(album_continuity, Ordering::Relaxed);
        self.xf_manual.store(manual, Ordering::Relaxed);
        self.xf_cfg.lock().shape.secs = ms as f32 / 1000.0;
    }

    /// Curves, filter sweep, smart start, and BPM matching.
    pub fn set_transition(&self, cfg: XfConfig) {
        let mut cur = self.xf_cfg.lock();
        let secs = cur.shape.secs;
        *cur = cfg;
        // Duration is owned by set_crossfade.
        cur.shape.secs = secs;
    }

    pub fn crossfade_secs(&self) -> f32 {
        self.xf_ms.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// `manual` is Next/Prev/clicking another track. Natural end-of-track
    /// handoffs pass `false` and only need the main crossfade toggle.
    pub fn should_crossfade(&self, from: Option<&Track>, to: &Track, manual: bool) -> bool {
        if !self.xf_on.load(Ordering::Relaxed) {
            return false;
        }
        if manual && !self.xf_manual.load(Ordering::Relaxed) {
            return false;
        }
        if self.crossfade_secs() < 0.05 {
            return false;
        }
        if self.album_cont.load(Ordering::Relaxed) && albums_match(from, to) {
            return false;
        }
        true
    }

    /// If we're close enough to the end, latch a blend and return the next
    /// track so the caller can prefetch a decoder and hand it back.
    pub fn blend_due(&self) -> Option<(Track, usize, PlayStyle)> {
        if self.blending.load(Ordering::Relaxed) {
            return None;
        }
        if self.phase.load(Ordering::Relaxed) != 1 {
            return None;
        }
        let s = self.snapshot();
        if !s.playing || s.duration <= 1.0 {
            return None;
        }
        let (track, index) = match self.next_choice(false) {
            NextChoice::Play(i) => s.queue.get(i).cloned().map(|t| (t, i))?,
            _ => return None,
        };
        if s.track.as_ref().map(|t| t.id.as_str()) == Some(track.id.as_str()) && s.repeat != "one" {
            return None;
        }
        let xf = self.should_crossfade(s.track.as_ref(), &track, false);
        let (smart, bpm) = {
            let c = self.xf_cfg.lock();
            (c.smart, c.bpm_match)
        };
        // Analysis runs off-thread and beat alignment may wait up to a
        // beat, so start a little earlier when those are on.
        let extra = if bpm {
            1.2
        } else if smart {
            0.4
        } else {
            0.0
        };
        let lead = if xf {
            self.crossfade_secs() as f64 + 0.2 + extra
        } else {
            0.35
        };
        // Smart start: the outgoing track has trailed off into silence
        // near its end - move on instead of playing dead air.
        let quiet_tail = xf
            && smart
            && s.duration - s.position < 15.0
            && self.quiet_ms.load(Ordering::Relaxed) >= 2000;
        if !quiet_tail && s.position + 0.04 < (s.duration - lead).max(0.0) {
            return None;
        }
        self.blending.store(true, Ordering::Relaxed);
        let style = if xf {
            PlayStyle::Crossfade(self.crossfade_secs())
        } else {
            PlayStyle::Gapless
        };
        Some((track, index, style))
    }

    pub fn clear_blending(&self) {
        self.blending.store(false, Ordering::Relaxed);
        *self.pending_adopt.lock() = None;
    }

    /// Show `track` in the UI only after the mixer has started it.
    pub fn queue_adopt(&self, track: Track, index: usize) {
        let gen = self.play_gen.load(Ordering::SeqCst);
        *self.pending_adopt.lock() = Some((track, index, gen));
    }

    pub fn adopt_track(&self, track: Track, index: usize) {
        self.ended.store(false, Ordering::Relaxed);
        // Remember the outgoing track so a late Ended from its decoder
        // is not treated as "this (new) track finished".
        let prev = self
            .state
            .lock()
            .track
            .as_ref()
            .map(|t| t.id.clone())
            .unwrap_or_default();
        if !prev.is_empty() {
            *self.last_ended.lock() = Some(prev);
        }
        self.blend_adopted.store(true, Ordering::SeqCst);
        self.quiet_ms.store(0, Ordering::Relaxed);
        let mut track = track;
        track.ensure_cover();
        let mut s = self.state.lock();
        // Keep any better cover already on the queue entry.
        if track.cover.is_none() {
            if let Some(q) = s.queue.get(index).and_then(|t| t.cover.clone()) {
                track.cover = Some(q);
            }
        } else if let Some(slot) = s.queue.get_mut(index) {
            if slot.cover.as_deref().is_none_or(|u| u.contains("/mixart")) {
                slot.cover = track.cover.clone();
            }
        }
        s.track = Some(track);
        s.index = index;
        s.loading = false;
        s.playing = true;
        s.position = 0.0;
        s.error = None;
        self.blending.store(false, Ordering::Relaxed);
    }

    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }
    pub fn resume(&self) {
        if !self.has_src.load(Ordering::Relaxed) {
            self.request_recover(true);
            return;
        }
        let _ = self.tx.send(Cmd::Resume);
    }
    pub fn toggle(&self) {
        match self.phase.load(Ordering::Relaxed) {
            1 | 4 => self.pause(),
            2 => self.resume(),
            3 => {
                // Still arming a restored track - play as soon as it's ready.
                let on = !self.want_play.load(Ordering::Relaxed);
                self.want_play.store(on, Ordering::Relaxed);
            }
            _ => {
                // Phase 0: the decoder is dead. Re-resolve at the current
                // position rather than no-op (Play used to do nothing).
                if self.state.lock().track.is_some() {
                    self.request_recover(true);
                }
            }
        }
    }

    pub fn request_recover(&self, play: bool) {
        if play {
            self.want_play.store(true, Ordering::Relaxed);
        }
        self.recover.store(true, Ordering::SeqCst);
        eprintln!(
            "recover: requested at {:.2}s (play={play})",
            self.snapshot().position
        );
    }

    /// One-shot: the emit loop re-resolves the current track at `position`.
    pub fn take_recover(&self) -> Option<(Track, Vec<Track>, usize, f64, bool)> {
        if !self.recover.swap(false, Ordering::SeqCst) {
            return None;
        }
        {
            let mut last = self.last_recover.lock();
            if let Some(t) = *last {
                if t.elapsed() < Duration::from_secs(2) {
                    eprintln!("recover: backing off (last attempt {:.1}s ago)", t.elapsed().as_secs_f32());
                    // Put the flag back so a later tick can retry.
                    self.recover.store(true, Ordering::SeqCst);
                    return None;
                }
            }
            *last = Some(Instant::now());
        }
        if self.recovering.swap(true, Ordering::SeqCst) {
            return None;
        }
        let s = self.snapshot();
        let track = s.track.clone()?;
        let play = self.want_play.load(Ordering::Relaxed) || s.playing || s.buffering;
        Some((track, s.queue, s.index, s.position, play))
    }

    pub fn recover_finished(&self) {
        self.recovering.store(false, Ordering::SeqCst);
    }

    pub fn is_buffering(&self) -> bool {
        self.buffering.load(Ordering::Relaxed) || self.phase.load(Ordering::Relaxed) == 4
    }
    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop);
    }

    /// Silence the mixer while `prepare` already owns the loading UI. Used on
    /// Cut/Fresh skips so the previous track does not keep playing under the
    /// new title while we resolve/download.
    pub fn hush(&self) {
        let _ = self.tx.send(Cmd::Hush);
    }
    pub fn seek(&self, secs: f64) {
        let secs = secs.max(0.0);
        let dur = self.state.lock().duration;
        let secs = if dur > 0.0 { secs.min(dur) } else { secs };
        self.samples.store(0, Ordering::Relaxed);
        self.state.lock().position = secs;
        let _ = self.tx.send(Cmd::Seek(secs));
    }
    pub fn set_volume(&self, v: f32) {
        let v = (v.clamp(0.0, 1.0) * 100.0).round() / 100.0;
        self.state.lock().volume = v;
        let _ = self.tx.send(Cmd::Volume(v));
    }

    pub fn set_eq(&self, gains: [f32; crate::eq::BANDS], enabled: bool) {
        let mut cleaned = [0.0f32; crate::eq::BANDS];
        for i in 0..crate::eq::BANDS {
            cleaned[i] = (gains[i].clamp(-12.0, 12.0) * 10.0).round() / 10.0;
        }
        let _ = self.tx.send(Cmd::Eq(cleaned, enabled));
    }
    pub fn take_ended(&self) -> bool {
        if self.blending.load(Ordering::Relaxed) {
            // Blend never adopted and the outgoing track finished - drop the
            // latch so natural advance (and Next) can recover instead of
            // sitting in silence with a stuck "next".
            if self.ended.swap(false, Ordering::Relaxed) {
                eprintln!("track-end during blend (no adopt); clearing blend latch");
                self.blending.store(false, Ordering::Relaxed);
                let id = self
                    .state
                    .lock()
                    .track
                    .as_ref()
                    .map(|t| t.id.clone())
                    .unwrap_or_default();
                let mut last = self.last_ended.lock();
                if !id.is_empty() && last.as_deref() == Some(id.as_str()) {
                    return false;
                }
                *last = Some(id.clone());
                eprintln!("track-end {id} (complete, blend recovered)");
                return true;
            }
            return false;
        }
        // Successful crossfade/gapless already adopted the next track.
        // The outgoing decoder ending must not start another advance
        // (that restarted the same song, or skipped one ahead).
        if self.blend_adopted.swap(false, Ordering::SeqCst) {
            self.ended.store(false, Ordering::Relaxed);
            return false;
        }
        if self.recovering.load(Ordering::Relaxed) || self.recover.load(Ordering::Relaxed) {
            self.ended.store(false, Ordering::Relaxed);
            return false;
        }
        if !self.ended.swap(false, Ordering::Relaxed) {
            return false;
        }
        let id = self
            .state
            .lock()
            .track
            .as_ref()
            .map(|t| t.id.clone())
            .unwrap_or_default();
        let mut last = self.last_ended.lock();
        if !id.is_empty() && last.as_deref() == Some(id.as_str()) {
            eprintln!("track-end ignored (already ended {id})");
            return false;
        }
        *last = Some(id.clone());
        eprintln!("track-end {id} (complete)");
        true
    }
    pub fn volume(&self) -> f32 {
        self.state.lock().volume
    }
    pub fn queue_and_index(&self) -> (Vec<Track>, usize) {
        let s = self.state.lock();
        (s.queue.clone(), s.index)
    }
    pub fn set_index(&self, i: usize) {
        self.state.lock().index = i;
    }

    pub fn set_shuffle(&self, on: bool) {
        let mut s = self.state.lock();
        if on == s.shuffle {
            return;
        }
        if on {
            let mut lin = self.linear_queue.lock();
            if lin.is_none() {
                *lin = Some(s.queue.clone());
            }
            let source = lin.as_ref().cloned().unwrap_or_else(|| s.queue.clone());
            let cur_id = s
                .track
                .as_ref()
                .map(|t| t.id.clone())
                .or_else(|| s.queue.get(s.index).map(|t| t.id.clone()));
            let cur_idx = cur_id
                .as_ref()
                .and_then(|id| source.iter().position(|t| &t.id == id))
                .unwrap_or(s.index.min(source.len().saturating_sub(1)));
            let mut q = source;
            let mut i = cur_idx;
            Self::shuffle_queue_view(&mut q, &mut i);
            s.queue = q;
            s.index = i;
            // Keep track pointer on the same song (now at 0).
            if let Some(t) = s.queue.first().cloned() {
                s.track = Some(t);
            }
            s.shuffle = true;
            crate::diag::log(format!(
                "shuffle on: queue={} current={}",
                s.queue.len(),
                s.queue.first().map(|t| t.id.as_str()).unwrap_or("-")
            ));
        } else {
            s.shuffle = false;
            if let Some(linear) = self.linear_queue.lock().take() {
                let cur_id = s
                    .track
                    .as_ref()
                    .map(|t| t.id.clone())
                    .or_else(|| s.queue.get(s.index).map(|t| t.id.clone()));
                s.queue = linear;
                if let Some(id) = cur_id {
                    s.index = s
                        .queue
                        .iter()
                        .position(|t| t.id == id)
                        .unwrap_or(0);
                }
                crate::diag::log(format!(
                    "shuffle off: restored linear, index={}",
                    s.index
                ));
            } else {
                crate::diag::log("shuffle off: no linear snapshot");
            }
        }
    }

    pub fn set_repeat(&self, mode: &str) {
        let mode = match mode {
            "all" | "one" => mode,
            _ => "off",
        };
        self.state.lock().repeat = mode.into();
    }

    pub fn cycle_repeat(&self) -> String {
        let mut s = self.state.lock();
        s.repeat = match s.repeat.as_str() {
            "off" => "all".into(),
            "all" => "one".into(),
            _ => "off".into(),
        };
        s.repeat.clone()
    }

    pub fn set_autoplay(&self, on: bool) {
        self.state.lock().autoplay = on;
    }

    pub fn set_rating(&self, liked: bool, disliked: bool) {
        let mut s = self.state.lock();
        s.liked = liked;
        s.disliked = disliked;
    }

    pub fn insert_next(&self, tracks: Vec<Track>) {
        let mut s = self.state.lock();
        let i = s.index.saturating_add(1).min(s.queue.len());
        let cur_id = s.queue.get(s.index).map(|t| t.id.clone());
        for (k, t) in tracks.iter().enumerate() {
            s.queue.insert(i + k, t.clone());
        }
        if let Some(lin) = self.linear_queue.lock().as_mut() {
            let at = cur_id
                .and_then(|id| lin.iter().position(|t| t.id == id))
                .map(|p| p + 1)
                .unwrap_or(lin.len());
            for (k, t) in tracks.into_iter().enumerate() {
                lin.insert(at + k, t);
            }
        }
    }

    pub fn append(&self, tracks: Vec<Track>) {
        let mut s = self.state.lock();
        s.queue.extend(tracks.iter().cloned());
        if let Some(lin) = self.linear_queue.lock().as_mut() {
            lin.extend(tracks);
        }
    }

    pub fn append_unique(&self, tracks: Vec<Track>) {
        let mut s = self.state.lock();
        let existing: std::collections::HashSet<_> =
            s.queue.iter().map(|t| t.id.clone()).collect();
        let mut added = Vec::new();
        for mut t in tracks {
            if !t.id.is_empty() && !existing.contains(&t.id) {
                t.ensure_cover();
                added.push(t.clone());
                s.queue.push(t);
            }
        }
        if let Some(lin) = self.linear_queue.lock().as_mut() {
            lin.extend(added);
        }
    }

    /// Swap the queue without interrupting the current source. Used when
    /// starting radio of the track that is already playing.
    pub fn replace_queue(&self, queue: Vec<Track>, index: usize) {
        let mut queue = queue;
        for t in &mut queue {
            t.ensure_cover();
        }
        let mut s = self.state.lock();
        if s.shuffle {
            *self.linear_queue.lock() = Some(queue.clone());
            let mut q = queue;
            let mut i = index;
            Self::shuffle_queue_view(&mut q, &mut i);
            s.queue = q;
            s.index = i;
        } else {
            *self.linear_queue.lock() = None;
            s.queue = queue;
            s.index = index;
        }
    }

    pub fn remove_at(&self, i: usize) {
        let mut s = self.state.lock();
        if i < s.queue.len() {
            let removed_id = s.queue[i].id.clone();
            s.queue.remove(i);
            if s.index > i {
                s.index -= 1;
            }
            if s.index >= s.queue.len() && !s.queue.is_empty() {
                s.index = s.queue.len() - 1;
            }
            if let Some(lin) = self.linear_queue.lock().as_mut() {
                lin.retain(|t| t.id != removed_id);
            }
        }
    }

    pub fn move_item(&self, from: usize, to: usize) {
        let mut s = self.state.lock();
        if from >= s.queue.len() || to >= s.queue.len() || from == to {
            return;
        }
        let t = s.queue.remove(from);
        s.queue.insert(to, t);
        if s.index == from {
            s.index = to;
        } else if from < s.index && to >= s.index {
            s.index -= 1;
        } else if from > s.index && to <= s.index {
            s.index += 1;
        }
    }

    /// What to do when the current track ends or the user hits next.
    pub fn next_choice(&self, user: bool) -> NextChoice {
        let s = self.state.lock();
        if s.queue.is_empty() {
            return NextChoice::Stop;
        }
        if !user && s.repeat == "one" {
            return NextChoice::Play(s.index);
        }
        // Shuffle reorders the queue itself so Up next matches playback;
        // advance is just sequential through that view.
        let n = s.index + 1;
        if n < s.queue.len() {
            NextChoice::Play(n)
        } else if s.repeat == "all" {
            NextChoice::Play(0)
        } else if s.autoplay {
            NextChoice::Autoplay
        } else if user {
            // Manual next on the last track with autoplay off: restart
            // current rather than wrapping to queue[0] (felt like old
            // playlist songs coming back).
            NextChoice::SeekZero
        } else {
            NextChoice::Stop
        }
    }

    pub fn prev_choice(&self) -> NextChoice {
        let snap = self.snapshot();
        if snap.position > 3.0 {
            return NextChoice::SeekZero;
        }
        let s = self.state.lock();
        if s.queue.is_empty() {
            return NextChoice::Stop;
        }
        if s.index == 0 {
            if s.repeat == "all" {
                NextChoice::Play(s.queue.len() - 1)
            } else {
                NextChoice::SeekZero
            }
        } else {
            NextChoice::Play(s.index - 1)
        }
    }
}

#[derive(Debug, Clone)]
pub enum NextChoice {
    Play(usize),
    Autoplay,
    SeekZero,
    Stop,
}

/// Extra handles the audio thread needs for analysed crossfades.
struct AudioExtras {
    tx: std::sync::mpsc::Sender<Cmd>,
    cfg: Arc<Mutex<XfConfig>>,
    quiet_ms: Arc<AtomicU32>,
    pending_adopt: Arc<Mutex<Option<(Track, usize, u64)>>>,
    play_gen: Arc<AtomicU64>,
    blend_adopted: Arc<AtomicBool>,
    quiet_clear: Arc<AtomicU32>,
    blending: Arc<AtomicBool>,
    last_ended: Arc<Mutex<Option<String>>>,
}

fn flush_pending_adopt(extras: &AudioExtras, state: &Mutex<PlayerState>) {
    let Some((mut track, index, gen)) = extras.pending_adopt.lock().take() else {
        return;
    };
    if extras.play_gen.load(Ordering::SeqCst) != gen {
        return;
    }
    track.ensure_cover();
    let mut s = state.lock();
    if track.cover.is_none() {
        if let Some(q) = s.queue.get(index).and_then(|t| t.cover.clone()) {
            track.cover = Some(q);
        }
    } else if let Some(slot) = s.queue.get_mut(index) {
        if slot.cover.as_deref().is_none_or(|u| u.contains("/mixart")) {
            slot.cover = track.cover.clone();
        }
    }
    let prev = s.track.as_ref().map(|t| t.id.clone()).unwrap_or_default();
    s.track = Some(track);
    s.index = index;
    s.loading = false;
    s.playing = true;
    s.position = 0.0;
    s.error = None;
    drop(s);
    if !prev.is_empty() {
        *extras.last_ended.lock() = Some(prev);
    }
    extras.blend_adopted.store(true, Ordering::SeqCst);
    extras.quiet_clear.store(0, Ordering::Relaxed);
    extras.blending.store(false, Ordering::Relaxed);
}

#[allow(clippy::too_many_arguments)]
fn audio_thread(
    rx: std::sync::mpsc::Receiver<Cmd>,
    samples: Arc<AtomicU64>,
    sample_rate: Arc<AtomicU32>,
    channels: Arc<AtomicU32>,
    ended: Arc<AtomicBool>,
    phase: Arc<AtomicU8>,
    current_audio: Arc<Mutex<Option<Arc<crate::decode::GrowingBody>>>>,
    state: Arc<Mutex<PlayerState>>,
    buffering: Arc<AtomicBool>,
    recover: Arc<AtomicBool>,
    has_src: Arc<AtomicBool>,
    extras: AudioExtras,
) {
    let (_stream, handle) = match OutputStream::try_default() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("audio output: {e}");
            // Keep the thread alive so commands don't fail the whole app.
            for _ in rx {
                // discard
            }
            return;
        }
    };
    let sink = Sink::try_new(&handle).expect("sink");
    sink.set_volume(state.lock().volume);
    let mix = crate::mix::Mixer::new(
        samples.clone(),
        sample_rate.clone(),
        channels.clone(),
        ended.clone(),
        phase.clone(),
        buffering.clone(),
        recover.clone(),
        has_src.clone(),
        extras.quiet_ms.clone(),
    );
    sink.append(mix.source());
    sink.play();
    let tap = mix.tap();
    // Bumped by every command that changes what should be playing, so a
    // slow crossfade analysis can't land on top of a newer action.
    let mut generation: u64 = 0;

    loop {
        let cmd = rx.recv_timeout(Duration::from_millis(80));
        // The live tempo tap only runs while BPM matching is on.
        let bpm_on = extras.cfg.lock().bpm_match;
        tap.set_analysis(bpm_on);
        if matches!(
            cmd,
            Ok(Cmd::Play(..)
                | Cmd::Arm(..)
                | Cmd::Crossfade(..)
                | Cmd::Gapless(..)
                | Cmd::Cut(..)
                | Cmd::Stop
                | Cmd::Hush
                | Cmd::Seek(_))
        ) {
            generation = generation.wrapping_add(1);
        }
        match cmd {
            Ok(Cmd::Play(audio, start)) => {
                match make_source(audio, start) {
                    Ok(src) => {
                        state.lock().position = start;
                        mix.play(src, crate::mix::CLICK_FADE);
                    }
                    Err(e) => {
                        eprintln!("decode: {e}");
                        state.lock().error = Some("Can't play this track".into());
                        phase.store(0, Ordering::Relaxed);
                    }
                }
            }
            Ok(Cmd::Arm(audio, start)) => {
                match make_source(audio.clone(), start) {
                    Ok(src) => {
                        let mut s = state.lock();
                        s.position = start;
                        s.loading = false;
                        s.playing = false;
                        drop(s);
                        mix.arm(src);
                    }
                    Err(e) => {
                        eprintln!("arm: {e}");
                        if !audio.is_complete() {
                            // Still downloading - retry via recover instead of
                            // sticking "Can't play" on a restore seek.
                            buffering.store(true, Ordering::Relaxed);
                            recover.store(true, Ordering::SeqCst);
                        } else {
                            state.lock().error = Some("Can't play this track".into());
                            phase.store(0, Ordering::Relaxed);
                        }
                    }
                }
            }
            Ok(Cmd::Crossfade(audio, start, cfg)) => {
                if cfg.needs_analysis() {
                    let tx = extras.tx.clone();
                    let tap = tap.clone();
                    let gen = generation;
                    let spawned = std::thread::Builder::new()
                        .name("tarn-xf-analysis".into())
                        .spawn(move || match prepare_transition(audio, start, &cfg, &tap) {
                            Ok((src, plan)) => {
                                let _ = tx.send(Cmd::XfReady(Box::new(src), Box::new(plan), gen));
                            }
                            Err(e) => eprintln!("crossfade: {e}"),
                        });
                    if let Err(e) = spawned {
                        eprintln!("crossfade: analysis thread: {e}");
                    }
                } else {
                    match make_source(audio, start) {
                        Ok(src) => {
                            mix.crossfade_plan(src, plain_plan(&cfg));
                            flush_pending_adopt(&extras, &state);
                        }
                        Err(e) => eprintln!("crossfade: {e}"),
                    }
                }
            }
            Ok(Cmd::XfReady(src, plan, gen)) => {
                if gen != generation {
                    eprintln!("crossfade: analysis superseded, dropping");
                    *extras.pending_adopt.lock() = None;
                } else if mix.is_paused() {
                    // Paused while analysing: swap silently, stay paused.
                    let skipped = plan.skipped_frames;
                    mix.arm(*src);
                    // arm() zeros the clock; keep smart-start / beat trim.
                    mix.apply_skipped_frames(skipped);
                    flush_pending_adopt(&extras, &state);
                } else {
                    mix.crossfade_plan(*src, *plan);
                    flush_pending_adopt(&extras, &state);
                }
            }
            Ok(Cmd::Gapless(audio, start)) => {
                match make_source(audio, start) {
                    Ok(src) => {
                        mix.queue_gapless(src);
                        flush_pending_adopt(&extras, &state);
                    }
                    Err(e) => eprintln!("gapless: {e}"),
                }
            }
            Ok(Cmd::Cut(audio, start)) => {
                match make_source(audio, start) {
                    Ok(src) => {
                        mix.cut(src, crate::mix::CLICK_FADE);
                        flush_pending_adopt(&extras, &state);
                    }
                    Err(e) => eprintln!("cut: {e}"),
                }
            }
            Ok(Cmd::Pause) => mix.pause(),
            Ok(Cmd::Resume) => {
                if !mix.resume() {
                    eprintln!("resume: no decoder; requesting re-resolve");
                    recover.store(true, Ordering::SeqCst);
                }
            }
            Ok(Cmd::Stop) => mix.stop(),
            Ok(Cmd::Hush) => mix.hush(),
            Ok(Cmd::Seek(secs)) => {
                // Fade out so the cut isn't a pop, then swap the decoder.
                mix.fading_out();
                let t0 = std::time::Instant::now();
                while !mix.env_quiet() && t0.elapsed() < Duration::from_millis(280) {
                    std::thread::sleep(Duration::from_millis(4));
                }
                let audio = current_audio.lock().clone();
                if let Some(audio) = audio {
                    let needed = audio
                        .bytes_for_secs(secs)
                        .saturating_add(crate::decode::READY_BYTES);
                    audio.request_urgent(needed as u64);
                    if audio.len() < needed && !audio.is_done() {
                        eprintln!(
                            "seek: fetching {needed} bytes for {secs:.2}s (have {})",
                            audio.len()
                        );
                        buffering.store(true, Ordering::Relaxed);
                        phase.store(4, Ordering::Relaxed);
                        let _ = audio.wait_len(needed, Duration::from_secs(30));
                        audio.set_buffering(false);
                    }
                    match make_source(audio.clone(), secs) {
                        Ok(src) => {
                            samples.store(0, Ordering::SeqCst);
                            state.lock().position = secs;
                            mix.seek_swap(src);
                            buffering.store(false, Ordering::Relaxed);
                        }
                        Err(e) => {
                            eprintln!("seek: {e}");
                            if !audio.is_complete() {
                                recover.store(true, Ordering::SeqCst);
                            }
                        }
                    }
                }
            }
            Ok(Cmd::Volume(v)) => sink.set_volume(v),
            Ok(Cmd::Eq(gains, on)) => mix.set_eq(gains, on),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn albums_match(from: Option<&Track>, to: &Track) -> bool {
    let Some(a) = from else {
        return false;
    };
    if let (Some(id1), Some(id2)) = (a.album_id.as_ref(), to.album_id.as_ref()) {
        if !id1.is_empty() && id1 == id2 {
            return true;
        }
    }
    if let (Some(n1), Some(n2)) = (a.album.as_ref(), to.album.as_ref()) {
        if !n1.is_empty() && n1.eq_ignore_ascii_case(n2) {
            let share = a.artists.iter().any(|x| to.artists.iter().any(|y| x.eq_ignore_ascii_case(y)));
            if share || a.artists.is_empty() || to.artists.is_empty() {
                return true;
            }
        }
    }
    false
}

/// Decode a few samples so callers can verify the in-process pipeline
/// without opening an audio device.
pub fn decode_head(audio: &[u8], n: usize) -> Result<(u32, u16, usize)> {
    crate::decode::decode_head(audio, n)
}

fn make_source(
    audio: Arc<crate::decode::GrowingBody>,
    start: f64,
) -> Result<crate::decode::AudioSource> {
    crate::decode::open(audio, start)
}

fn plain_plan(cfg: &XfConfig) -> crate::mix::XfPlan {
    crate::mix::XfPlan {
        shape: cfg.shape.clone(),
        return_secs: cfg.bpm_return_secs,
        smart: cfg.smart,
        ..crate::mix::XfPlan::simple(cfg.shape.secs)
    }
}

/// Open the incoming decoder and work out how to blend into it: trim its
/// leading silence, detect both tempos, pick a playback rate within the
/// allowed change, and line up beats. Runs off the audio thread.
fn prepare_transition(
    audio: Arc<crate::decode::GrowingBody>,
    start: f64,
    cfg: &XfConfig,
    tap: &crate::mix::MixTap,
) -> Result<(crate::decode::AudioSource, crate::mix::XfPlan)> {
    use crate::transition::{analyze, match_rate, EnvBuilder, FrameRes, MIN_CONFIDENCE, SILENCE};
    use rodio::Source;
    let t0 = Instant::now();
    let mut src = make_source(audio.clone(), start)?;
    let sr = src.sample_rate().max(1);
    let ch = src.channels().max(1) as usize;
    let mut plan = plain_plan(cfg);

    // Look at the head of the incoming track with a second decoder so the
    // playback one stays untouched. Only from the top: a mid-track start
    // would make the decoder wait for the whole download.
    let mut env = EnvBuilder::new(sr, 12.0);
    let mut lead_silence = 0u64;
    if start <= 0.05 {
        if let Ok(mut probe) = make_source(audio.clone(), start) {
            let max_frames = 10 * sr as u64;
            let mut f = vec![0i16; ch];
            let mut heard = false;
            while env.frames < max_frames && t0.elapsed() < Duration::from_millis(1500) {
                if !matches!(crate::mix::pull_raw(&mut probe, &mut f), FrameRes::Ok) {
                    break;
                }
                if !heard {
                    let peak = f.iter().map(|&v| (v as i32).abs()).max().unwrap_or(0);
                    if peak >= SILENCE {
                        heard = true;
                    } else {
                        lead_silence += 1;
                    }
                }
                env.push_frame(&f);
            }
            if !heard {
                lead_silence = 0;
            }
        }
    }

    let mut skip = 0u64;
    if cfg.smart {
        skip = lead_silence.min(8 * sr as u64);
    }
    if cfg.bpm_match {
        let full: Vec<f32> = env.full.iter().copied().collect();
        let low: Vec<f32> = env.low.iter().copied().collect();
        let base = env.window_start() as f64;
        let inb = analyze(&full, &low, sr).filter(|b| b.confidence >= MIN_CONFIDENCE);
        let outb = tap.outgoing_beat().filter(|b| b.confidence >= MIN_CONFIDENCE);
        match (outb, inb) {
            (Some(o), Some(i)) => {
                if let Some((rate, eff)) = match_rate(o.bpm, i.bpm, cfg.bpm_max_pct as f64) {
                    plan.in_rate = rate;
                    eprintln!(
                        "bpm: out {:.1} / in {:.1} (as {:.1}) → incoming at {:.2}%",
                        o.bpm,
                        i.bpm,
                        eff,
                        (rate - 1.0) * 100.0
                    );
                    if cfg.beat_align {
                        let p_in = 60.0 * sr as f64 / eff;
                        let phase = base + i.phase;
                        let k = ((skip as f64 - phase) / p_in).ceil().max(0.0);
                        let first = phase + k * p_in;
                        if first >= skip as f64 && first - (skip as f64) <= p_in {
                            skip = first.round() as u64;
                        }
                        plan.out_grid = Some((o.period, o.phase));
                    }
                } else {
                    eprintln!(
                        "bpm: out {:.1} / in {:.1} needs more than ±{:.1}%, not matching",
                        o.bpm, i.bpm, cfg.bpm_max_pct
                    );
                }
            }
            _ => eprintln!("bpm: tempo unclear, crossfading without matching"),
        }
    }
    if skip > 0 {
        let skip_secs = skip as f64 / sr as f64;
        // Prefer a real seek so the decoder timeline matches the clock.
        // Frame-pulling also works, but a later play()/arm() that forgot
        // skipped_frames left the progress bar at 0 while audio was ahead.
        match make_source(audio.clone(), start + skip_secs) {
            Ok(s) => {
                src = s;
                plan.skipped_frames = skip;
                eprintln!(
                    "smart-start: skipped {skip_secs:.2}s of lead silence/beat align"
                );
            }
            Err(_) => {
                let mut f = vec![0i16; ch];
                let mut done = 0u64;
                while done < skip {
                    if !matches!(crate::mix::pull_raw(&mut src, &mut f), FrameRes::Ok) {
                        break;
                    }
                    done += 1;
                }
                plan.skipped_frames = done;
            }
        }
    }
    Ok((src, plan))
}


#[cfg(test)]
mod crossfade_tests {
    use super::*;
    use crate::models::Track;

    fn t(id: &str, title: &str) -> Track {
        Track {
            id: id.into(),
            title: title.into(),
            duration: Some(10),
            ..Default::default()
        }
    }

    fn queue(n: usize) -> Vec<Track> {
        (0..n)
            .map(|i| t(&format!("t{i}"), &format!("Track {i}")))
            .collect()
    }

    fn arm(p: &Player, q: Vec<Track>, index: usize, pos: f64, dur: f64) {
        p.prepare(q[index].clone(), q, index, pos, false);
        p.state.lock().duration = dur;
        p.state.lock().position = pos;
        p.phase.store(1, Ordering::Relaxed);
        p.samples.store(0, Ordering::Relaxed);
        p.sample_rate.store(48000, Ordering::Relaxed);
        p.channels.store(2, Ordering::Relaxed);
        p.blending.store(false, Ordering::Relaxed);
        p.blend_adopted.store(false, Ordering::Relaxed);
        p.ended.store(false, Ordering::Relaxed);
        *p.last_ended.lock() = None;
    }

    fn run_xf_across(p: &Player, q: Vec<Track>, repeat: &str, shuffle: bool) -> Vec<String> {
        p.set_crossfade(true, 2.0, false, true);
        p.set_repeat(repeat);
        p.set_shuffle(shuffle);
        let n = q.len();
        arm(p, q.clone(), 0, 8.2, 10.0);
        let mut order = vec![p.snapshot().track.unwrap().id.clone()];
        let steps = if repeat == "one" { 3 } else { n + 2 };
        for _ in 0..steps {
            let Some((track, index, _style)) = p.blend_due() else {
                // If blend didn't fire, simulate a natural end.
                p.ended.store(true, Ordering::Relaxed);
                if p.take_ended() {
                    match p.next_choice(false) {
                        NextChoice::Play(i) => {
                            let q = p.queue_and_index().0;
                            if let Some(t) = q.get(i).cloned() {
                                arm(p, q, i, 8.2, 10.0);
                                order.push(t.id);
                            }
                        }
                        _ => break,
                    }
                } else {
                    break;
                }
                continue;
            };
            // Adopt the blended-in track, then the outgoing decoder ends.
            let id = track.id.clone();
            p.adopt_track(track, index);
            p.ended.store(true, Ordering::Relaxed);
            let advanced = p.take_ended();
            assert!(
                !advanced,
                "take_ended must not advance after a successful blend (would replay or skip); repeat={repeat} shuffle={shuffle}"
            );
            order.push(id);
            // Sit near the end of the newly adopted track so the next blend fires.
            p.state.lock().position = 8.2;
            p.state.lock().duration = 10.0;
            p.phase.store(1, Ordering::Relaxed);
            p.samples.store(0, Ordering::Relaxed);
            p.blending.store(false, Ordering::Relaxed);
            // blend_adopted already cleared by take_ended
        }
        order
    }

    #[test]
    fn crossfade_repeat_off_plays_each_once_then_stops() {
        let p = Player::start();
        let order = run_xf_across(&p, queue(5), "off", false);
        assert_eq!(
            order,
            vec!["t0", "t1", "t2", "t3", "t4"],
            "repeat-off must advance 0..4 and not restart t0 or repeat a song"
        );
    }

    #[test]
    fn crossfade_repeat_all_wraps_to_first() {
        let p = Player::start();
        let order = run_xf_across(&p, queue(5), "all", false);
        assert_eq!(&order[..5], &["t0", "t1", "t2", "t3", "t4"][..]);
        assert_eq!(order.get(5).map(String::as_str), Some("t0"), "repeat-all wraps");
    }

    #[test]
    fn crossfade_repeat_one_stays_on_same() {
        let p = Player::start();
        let order = run_xf_across(&p, queue(5), "one", false);
        assert!(order.len() >= 2, "got {order:?}");
        assert!(
            order.iter().all(|id| id == "t0"),
            "repeat-one must keep the same track, got {order:?}"
        );
    }

    #[test]
    fn crossfade_shuffle_never_repeats_immediately() {
        let p = Player::start();
        let order = run_xf_across(&p, queue(5), "off", true);
        assert!(order.len() >= 2, "got {order:?}");
        for w in order.windows(2) {
            assert_ne!(w[0], w[1], "shuffle must not pick the current track again: {order:?}");
        }
    }

    #[test]
    fn shuffle_reorders_queue_and_restores() {
        let p = Player::start();
        let q = queue(8);
        let original: Vec<String> = q.iter().map(|t| t.id.clone()).collect();
        arm(&p, q, 3, 1.0, 10.0);
        p.set_shuffle(true);
        let on = p.snapshot();
        assert!(on.shuffle);
        assert_eq!(on.index, 0, "current track moves to front of Up next");
        assert_eq!(on.queue[0].id, original[3]);
        let shuffled_ids: Vec<_> = on.queue.iter().map(|t| t.id.clone()).collect();
        assert_ne!(
            shuffled_ids, original,
            "Up next order should change when shuffle turns on"
        );
        // Sequential next through the shuffled view.
        assert!(matches!(p.next_choice(true), NextChoice::Play(1)));

        p.set_shuffle(false);
        let off = p.snapshot();
        assert!(!off.shuffle);
        let restored: Vec<_> = off.queue.iter().map(|t| t.id.clone()).collect();
        assert_eq!(restored, original);
        assert_eq!(off.index, 3);
    }

    #[test]
    fn next_choice_repeat_off_all_one() {
        let p = Player::start();
        let q = queue(3);
        arm(&p, q.clone(), 0, 1.0, 10.0);
        p.set_repeat("off");
        p.set_shuffle(false);
        assert!(matches!(p.next_choice(false), NextChoice::Play(1)));
        arm(&p, q.clone(), 2, 1.0, 10.0);
        p.set_repeat("off");
        p.set_autoplay(false);
        assert!(matches!(p.next_choice(false), NextChoice::Stop));
        p.set_autoplay(true);
        assert!(matches!(p.next_choice(false), NextChoice::Autoplay));
        p.set_repeat("all");
        assert!(matches!(p.next_choice(false), NextChoice::Play(0)));
        p.set_repeat("one");
        assert!(matches!(p.next_choice(false), NextChoice::Play(2)));
    }
}
