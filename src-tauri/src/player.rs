use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use rodio::Sink;

use crate::analysis::{Analyzer, TrackAnalysis};
use crate::models::{PlayerState, Track};
use crate::transition::XfConfig;
use crate::ytm::Stream;

/// Commands sent to the audio thread.
enum Cmd {
    Play(Arc<crate::decode::GrowingBody>, f64),
    Arm(Arc<crate::decode::GrowingBody>, f64),
    Crossfade(Arc<crate::decode::GrowingBody>, f64, Box<XfConfig>, Box<XfHints>),
    /// Analysis for a crossfade finished off-thread; `u64` is the command
    /// generation it was started for (stale results are dropped).
    XfReady(Box<crate::decode::AudioSource>, Box<crate::mix::XfPlan>, u64),
    Gapless(Arc<crate::decode::GrowingBody>, f64),
    Cut(Arc<crate::decode::GrowingBody>, f64),
    Pause,
    Resume,
    Stop,
    Seek(f64),
    Volume(f32),
    Eq([f32; crate::eq::BANDS], bool),
    Fx(crate::mix::Fx),
    /// Move playback to another output device (None = system default).
    Output(Option<String>),
    /// Loudness gain for the deck loaded by the next play-type command.
    NextGain(f32),
    /// Loudness gain for what is audible now (its analysis just finished).
    LiveGain(f32),
}

/// Cached analysis of both sides of a crossfade (see `analysis.rs`).
#[derive(Default)]
struct XfHints {
    out: Option<TrackAnalysis>,
    inc: Option<TrackAnalysis>,
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
    /// (track id, count) of premature track-ends turned into re-resolves.
    early_ends: Arc<Mutex<(String, u32)>>,
    /// Set by adopt_track after a successful blend; take_ended must not
    /// also advance (that restarted the same song or skipped ahead).
    blend_adopted: Arc<AtomicBool>,
    /// Bumped on every prepare/play so a stale crossfade from the previous
    /// queue cannot adopt into the new one.
    play_gen: Arc<AtomicU64>,
    /// Next track to show in the UI once the mixer actually starts it.
    /// Avoids "new title, old audio" while crossfade analysis is in flight.
    pending_adopt: Arc<Mutex<Option<(Track, usize, u64)>>>,
    /// Like/dislike for the pending adopt track. Applied with the title swap
    /// so the heart does not flip to the next song mid-crossfade.
    pending_rating: Arc<Mutex<Option<(bool, bool)>>>,
    /// Original playlist order while shuffle is on (for restore + Up next).
    linear_queue: Arc<Mutex<Option<Vec<Track>>>>,
    /// Per-track loudness / beat cache.
    analyzer: Arc<Mutex<Option<Arc<Analyzer>>>>,
    /// Loudness normalization target in LUFS (None = off).
    normalize: Arc<Mutex<Option<f32>>>,
    /// Video id of the track the mixer is (about to be) playing.
    audible: Arc<Mutex<String>>,
    /// Auto-DJ wants upcoming tracks analysed.
    auto_dj: Arc<AtomicBool>,
    /// AutoMix: per-transition timing chosen from the outgoing track.
    automix: Arc<AtomicBool>,
    waveform: Arc<AtomicBool>,
    /// Live crossfade progress‰ from the mixer (0 = idle).
    xf_milli: Arc<AtomicU32>,
    /// Outgoing track while a shaped crossfade is audible (for the UI morph).
    xfade_from: Arc<Mutex<Option<Track>>>,
    /// Last manual Next/Prev/pick time — rapid skips skip the long fade.
    last_manual: Arc<Mutex<Option<Instant>>>,
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
        let early_ends = Arc::new(Mutex::new((String::new(), 0u32)));
        let blend_adopted = Arc::new(AtomicBool::new(false));
        let play_gen = Arc::new(AtomicU64::new(0));
        let pending_adopt = Arc::new(Mutex::new(None));
        let pending_rating = Arc::new(Mutex::new(None));
        let linear_queue = Arc::new(Mutex::new(None));
        let xf_milli = Arc::new(AtomicU32::new(0));
        let xfade_from = Arc::new(Mutex::new(None));
        let last_manual = Arc::new(Mutex::new(None));

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
        let pending_rating_t = pending_rating.clone();
        let play_gen_t = play_gen.clone();
        let blend_adopted_t = blend_adopted.clone();
        let quiet_ms_t = quiet_ms.clone();
        let blending_t = blending.clone();
        let last_ended_t = last_ended.clone();
        let xf_milli_t = xf_milli.clone();

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
                    xf_milli_t,
                    AudioExtras {
                        tx: tx_t,
                        cfg: cfg_t,
                        quiet_ms: quiet_t,
                        pending_adopt: pending_t,
                        pending_rating: pending_rating_t,
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
            early_ends,
            blend_adopted,
            play_gen,
            pending_adopt,
            pending_rating,
            linear_queue,
            analyzer: Arc::new(Mutex::new(None)),
            normalize: Arc::new(Mutex::new(None)),
            audible: Arc::new(Mutex::new(String::new())),
            auto_dj: Arc::new(AtomicBool::new(false)),
            automix: Arc::new(AtomicBool::new(false)),
            waveform: Arc::new(AtomicBool::new(false)),
            xf_milli,
            xfade_from,
            last_manual,
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
        let milli = self.xf_milli.load(Ordering::Relaxed);
        let arming = self.blending.load(Ordering::Relaxed)
            || self.pending_adopt.lock().is_some();
        let mut from = self.xfade_from.lock();
        if milli > 0 {
            s.xfade = Some((milli as f64 / 1000.0).clamp(0.0, 1.0));
            s.xfade_from = from.clone();
        } else if arming && from.is_some() {
            // Analysis / beat-wait: hold the ghost at 0 until audio overlaps.
            s.xfade = Some(0.0);
            s.xfade_from = from.clone();
        } else {
            *from = None;
            s.xfade = None;
            s.xfade_from = None;
        }
        s
    }

    /// True while a shaped crossfade is reporting UI progress (or arming).
    pub fn xfade_active(&self) -> bool {
        self.xf_milli.load(Ordering::Relaxed) > 0 || self.xfade_from.lock().is_some()
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
        *self.pending_rating.lock() = None;
        self.ended.store(false, Ordering::Relaxed);
        self.recover.store(false, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
        self.blending.store(false, Ordering::Relaxed);
        self.blend_adopted.store(false, Ordering::SeqCst);
        *self.last_ended.lock() = None;
        *self.xfade_from.lock() = None;
        self.xf_milli.store(0, Ordering::Relaxed);
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
        // Always take the new track's length (0 = not known yet). Keeping the
        // previous track's value would show the wrong total and leave the seek
        // bar pointing at the wrong length until the stream reports its own.
        s.duration = track
            .duration
            .filter(|&d| d > 0)
            .map(|d| d as f64)
            .unwrap_or(0.0);
        s.track = Some(track);
        s.loading = loading;
        s.playing = false;
        s.position = position.max(0.0);
        s.error = None;
        Self::install_queue(&mut s, &self.linear_queue, queue, index);
    }

    /// Manual skip / pick-another-track with crossfade (or cut): keep the
    /// outgoing title, duration, index and seek clock until the mixer
    /// actually starts the overlap. Early `prepare()` used to swap in the
    /// next track at position 0 while `samples` still held the old elapsed
    /// time, so the bar snapped to the end of the new song mid-fade.
    ///
    /// Returns the (possibly shuffle-remapped) queue index for `queue_adopt`.
    pub fn begin_manual_blend(&self, queue: Vec<Track>, index: usize) -> usize {
        self.play_gen.fetch_add(1, Ordering::SeqCst);
        *self.pending_adopt.lock() = None;
        // Keep pending_rating: start_playback stashes the next heart before this.
        self.ended.store(false, Ordering::Relaxed);
        self.recover.store(false, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
        self.blending.store(true, Ordering::Relaxed);
        self.blend_adopted.store(false, Ordering::SeqCst);
        *self.last_ended.lock() = None;
        self.quiet_ms.store(0, Ordering::Relaxed);
        let mut queue = queue;
        for t in &mut queue {
            t.ensure_cover();
        }
        let mut s = self.state.lock();
        s.error = None;
        // Keep showing the outgoing index until adopt; only refresh the list.
        let keep = s.index;
        let mapped = Self::install_queue(&mut s, &self.linear_queue, queue, index);
        s.index = keep.min(s.queue.len().saturating_sub(1));
        mapped
    }

    fn install_queue(
        s: &mut PlayerState,
        linear_queue: &Arc<Mutex<Option<Vec<Track>>>>,
        queue: Vec<Track>,
        index: usize,
    ) -> usize {
        if s.shuffle {
            let mut lin = linear_queue.lock();
            if lin
                .as_ref()
                .is_some_and(|orig| Self::same_track_set(orig, &queue))
            {
                // Same playlist (e.g. click in Up next) - keep linear, keep order.
                s.queue = queue;
                s.index = index;
                index
            } else {
                // New playlist while shuffle is on: remember linear, show shuffled.
                *lin = Some(queue.clone());
                let mut q = queue;
                let mut i = index;
                Self::shuffle_queue_view(&mut q, &mut i);
                s.queue = q;
                s.index = i;
                i
            }
        } else {
            *linear_queue.lock() = None;
            s.queue = queue;
            s.index = index;
            index
        }
    }



    pub fn set_accent(&self, rgb: Option<[u8; 3]>) {
        self.state.lock().accent = rgb;
    }

    pub fn set_error(&self, msg: String) {
        self.phase.store(0, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
        self.recover.store(false, Ordering::Relaxed);
        let mut s = self.state.lock();
        s.loading = false;
        s.playing = false;
        s.error = Some(msg);
    }

    /// Remember a track's length once it's known, so lists (Up next) that
    /// were given a track without one can show it. Matched by id.
    pub fn fill_track_duration(&self, id: &str, d: f64) {
        if d <= 0.0 {
            return;
        }
        let secs = d.round() as u32;
        let mut s = self.state.lock();
        let fill = |t: &mut Track| {
            if t.id == id && t.duration.map_or(true, |x| x == 0) {
                t.duration = Some(secs);
            }
        };
        if let Some(t) = s.track.as_mut() {
            fill(t);
        }
        for t in s.queue.iter_mut() {
            fill(t);
        }
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
        // Which track this audio is: a queued blend adopts later, otherwise
        // the state already shows it.
        let in_id = match self.pending_adopt.lock().as_ref() {
            Some((t, _, _)) => t.id.clone(),
            None => self
                .state
                .lock()
                .track
                .as_ref()
                .map(|t| t.id.clone())
                .unwrap_or_default(),
        };
        let out_id = std::mem::replace(&mut *self.audible.lock(), in_id.clone());
        // Stash the audible/UI track so the front-end can morph covers while
        // the mixer overlaps. Arm progress immediately so analysis wait does
        // not clear the ghost on the next snapshot. Short cuts clear it.
        if matches!(style, PlayStyle::Crossfade(_)) {
            let outgoing = self.state.lock().track.clone();
            *self.xfade_from.lock() = outgoing;
            // 1‰ placeholder until the mixer reports real progress.
            if self.xf_milli.load(Ordering::Relaxed) == 0 {
                self.xf_milli.store(1, Ordering::Relaxed);
            }
        } else {
            *self.xfade_from.lock() = None;
            self.xf_milli.store(0, Ordering::Relaxed);
        }
        self.analyze(&in_id, audio.clone());
        let _ = self.tx.send(Cmd::NextGain(self.gain_for(&in_id)));
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
                let hints = if cfg.bpm_match || cfg.smart {
                    XfHints {
                        out: self.cached(&out_id),
                        inc: self.cached(&in_id),
                    }
                } else {
                    XfHints::default()
                };
                Cmd::Crossfade(audio, start, Box::new(cfg), Box::new(hints))
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
    pub fn set_analyzer(&self, a: Arc<Analyzer>) {
        *self.analyzer.lock() = Some(a);
    }

    /// Loudness normalization on/off and its target level (LUFS).
    pub fn set_normalize(&self, on: bool, target: f32) {
        let new = on.then_some(target.clamp(-30.0, -5.0));
        let old = std::mem::replace(&mut *self.normalize.lock(), new);
        if old != new {
            // Re-level the playing track right away.
            let id = self.audible.lock().clone();
            let _ = self.tx.send(Cmd::LiveGain(self.gain_for(&id)));
        }
    }

    /// True when a feature needs per-track analysis (normalization or BPM
    /// matching), so nothing is decoded in the background otherwise.
    pub fn wants_analysis(&self) -> bool {
        self.normalize.lock().is_some()
            || self.xf_cfg.lock().bpm_match
            || self.auto_dj.load(Ordering::Relaxed)
            || self.automix.load(Ordering::Relaxed)
            || self.waveform.load(Ordering::Relaxed)
    }

    /// Analyse tracks so the seek bar can show their waveform.
    pub fn set_waveform(&self, on: bool) {
        self.waveform.store(on, Ordering::Relaxed);
    }

    pub fn set_automix(&self, on: bool) {
        self.automix.store(on, Ordering::Relaxed);
    }

    pub fn automix_on(&self) -> bool {
        self.automix.load(Ordering::Relaxed)
    }

    pub fn set_auto_dj(&self, on: bool) {
        self.auto_dj.store(on, Ordering::Relaxed);
    }

    /// Queue a complete (or downloading) body for analysis when useful.
    pub fn analyze(&self, id: &str, body: Arc<crate::decode::GrowingBody>) {
        if !self.wants_analysis() {
            return;
        }
        if let Some(a) = self.analyzer.lock().clone() {
            a.enqueue(id, body);
        }
    }

    fn cached(&self, id: &str) -> Option<TrackAnalysis> {
        self.analyzer.lock().as_ref()?.get(id)
    }

    /// Normalization gain for a track (1 = unchanged).
    fn gain_for(&self, id: &str) -> f32 {
        let Some(target) = *self.normalize.lock() else {
            return 1.0;
        };
        self.cached(id).map(|a| a.gain_for(target)).unwrap_or(1.0)
    }

    /// Called by the analyzer when a track has been measured.
    pub fn analysis_ready(&self, id: &str) {
        if self.normalize.lock().is_some() && *self.audible.lock() == id {
            let _ = self.tx.send(Cmd::LiveGain(self.gain_for(id)));
        }
    }

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
    ///
    /// Call [`Self::note_manual_nav`] after the play decision so probing this
    /// twice (or a resolve retry) cannot poison the rapid-skip window.
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
        // Mash next/prev while a shaped fade is already audible, or within 2s
        // of the last manual nav: cut hard so long fades don't stack.
        if manual && self.rapid_manual_nav() {
            return false;
        }
        true
    }

    /// True when the user is mashing skip (or skipping mid-fade).
    pub fn rapid_manual_nav(&self) -> bool {
        if self.xf_milli.load(Ordering::Relaxed) > 0 {
            return true;
        }
        self.last_manual
            .lock()
            .map(|t| t.elapsed() < Duration::from_millis(2000))
            .unwrap_or(false)
    }

    /// Mark a user Next/Prev/pick so a second press within 2s skips the fade.
    pub fn note_manual_nav(&self) {
        *self.last_manual.lock() = Some(Instant::now());
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
        // AutoMix: start where the outgoing track's analysis says (outro,
        // bar line, music end) with a length to match.
        let mix = (xf && self.automix.load(Ordering::Relaxed)).then(|| {
            let id = s.track.as_ref().map(|t| t.id.clone()).unwrap_or_default();
            crate::automix::mix_point(self.cached(&id).as_ref(), s.duration)
        });
        let lead = match mix {
            Some(m) => (s.duration - m.start_s).max(0.0) + extra,
            None if xf => self.crossfade_secs() as f64 + 0.2 + extra,
            None => 0.35,
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
        let style = if let Some(m) = mix {
            // Starting late (seek near the end, analysis arrived late):
            // fit the blend into what is left.
            let left = (s.duration - s.position - 0.2).max(1.0) as f32;
            PlayStyle::Crossfade(m.secs.min(left))
        } else if xf {
            PlayStyle::Crossfade(self.crossfade_secs())
        } else {
            PlayStyle::Gapless
        };
        Some((track, index, style))
    }

    pub fn clear_blending(&self) {
        self.blending.store(false, Ordering::Relaxed);
        *self.pending_adopt.lock() = None;
        *self.pending_rating.lock() = None;
        // Aborted blend: drop the morph ghost so the UI doesn't hang mid-fade.
        if self.xf_milli.load(Ordering::Relaxed) == 0 {
            *self.xfade_from.lock() = None;
        }
    }

    /// Show `track` in the UI only after the mixer has started it.
    pub fn queue_adopt(&self, track: Track, index: usize) {
        let gen = self.play_gen.load(Ordering::SeqCst);
        *self.pending_adopt.lock() = Some((track, index, gen));
    }

    /// Heart state to apply when the pending adopt track becomes visible.
    pub fn set_pending_rating(&self, liked: bool, disliked: bool) {
        *self.pending_rating.lock() = Some((liked, disliked));
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
        let rating = self.pending_rating.lock().take();
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
        if let Some(d) = track.duration.filter(|&d| d > 0) {
            s.duration = d as f64;
        }
        s.track = Some(track);
        s.index = index;
        s.loading = false;
        s.playing = true;
        s.position = 0.0;
        s.error = None;
        if let Some((liked, disliked)) = rating {
            s.liked = liked;
            s.disliked = disliked;
        }
        self.blending.store(false, Ordering::Relaxed);
    }

    pub fn pause(&self) {
        // Reflect it now: the audio thread applies Pause a moment later, and a
        // second quick key press must already see "paused".
        if matches!(self.phase.load(Ordering::Relaxed), 1 | 4) {
            self.phase.store(2, Ordering::Relaxed);
        }
        let _ = self.tx.send(Cmd::Pause);
    }
    pub fn resume(&self) {
        if !self.has_src.load(Ordering::Relaxed) {
            self.request_recover(true);
            return;
        }
        if self.phase.load(Ordering::Relaxed) == 2 {
            self.phase.store(1, Ordering::Relaxed);
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
        // Near the natural end, prefer auto-advance / autoplay over a
        // mid-track re-resolve - otherwise the last song sticks forever.
        {
            let s = self.snapshot();
            if s.duration > 1.0 && s.position >= (s.duration - 1.5).max(0.0) {
                eprintln!(
                    "recover: at end ({:.2}/{:.2}); treating as track-end",
                    s.position, s.duration
                );
                self.ended.store(true, Ordering::Relaxed);
                self.buffering.store(false, Ordering::Relaxed);
                return None;
            }
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
    /// Playback speed (0.5..=2), keep pitch, reverb wet (0..=1).
    pub fn set_fx(&self, speed: f64, keep_pitch: bool, reverb: f64) {
        let speed = if speed.is_finite() { (speed.clamp(0.5, 2.0) * 100.0).round() / 100.0 } else { 1.0 };
        let reverb = if reverb.is_finite() { reverb.clamp(0.0, 1.0) as f32 } else { 0.0 };
        let _ = self.tx.send(Cmd::Fx(crate::mix::Fx { speed, keep_pitch, reverb }));
    }
    /// Reopen the audio output on `name` (None = system default).
    pub fn set_output_device(&self, name: Option<String>) {
        let _ = self.tx.send(Cmd::Output(name));
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
            let s = self.snapshot();
            if s.duration > 1.0 && s.position >= (s.duration - 1.5).max(0.0) {
                // Let track-end win so autoplay can extend the queue.
                self.recover.store(false, Ordering::SeqCst);
            } else {
                self.ended.store(false, Ordering::Relaxed);
                return false;
            }
        }
        if !self.ended.swap(false, Ordering::Relaxed) {
            return false;
        }
        // The decoder said "finished" far from the real end: a truncated
        // download or a dropped connection, not a finished song. Advancing
        // here is what made songs change by themselves mid-listen. Re-resolve
        // the same track instead (twice at most, then give up and advance).
        {
            let s = self.snapshot();
            if s.duration > 30.0 && s.position < s.duration - 8.0 {
                if let Some(t) = s.track.as_ref() {
                    let mut e = self.early_ends.lock();
                    if e.0 != t.id {
                        *e = (t.id.clone(), 0);
                    }
                    if e.1 < 2 {
                        e.1 += 1;
                        eprintln!(
                            "track-end at {:.1}/{:.1}s is premature ({}); re-resolving instead of advancing",
                            s.position, s.duration, t.id
                        );
                        drop(e);
                        // Keep playing after the re-resolve (phase is 0 now).
                        self.want_play.store(true, Ordering::Relaxed);
                        self.recover.store(true, Ordering::SeqCst);
                        return false;
                    }
                }
            }
        }
        // A new track is still resolving (UI already shows it). The old
        // decoder ending now is stale: advancing would skip the track the
        // user just picked.
        if self.state.lock().loading {
            eprintln!("track-end ignored (next track still loading)");
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

    /// Drop everything after the current track (keeps what is playing).
    pub fn clear_upcoming(&self) {
        let mut s = self.state.lock();
        let keep = s.index.saturating_add(1);
        if keep >= s.queue.len() {
            return;
        }
        s.queue.truncate(keep);
        let ids: std::collections::HashSet<String> =
            s.queue.iter().map(|t| t.id.clone()).collect();
        if let Some(lin) = self.linear_queue.lock().as_mut() {
            lin.retain(|t| ids.contains(&t.id));
        }
    }

    pub fn remove_at(&self, i: usize) {
        let mut s = self.state.lock();
        // The playing entry can't be removed - the audio would keep going
        // while the queue pointed at the next song.
        if i == s.index {
            return;
        }
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

    /// Replace `queue[start..]` (currently `expected`, by id) with the same
    /// tracks in `new_ids` order. Returns false if the queue changed since
    /// the caller looked at it.
    pub fn reorder_window(&self, start: usize, expected: &[String], new_ids: &[String]) -> bool {
        let mut s = self.state.lock();
        let end = start + expected.len();
        if s.shuffle
            || end > s.queue.len()
            || start <= s.index
            || expected.len() != new_ids.len()
            || !s.queue[start..end].iter().zip(expected).all(|(t, id)| &t.id == id)
        {
            return false;
        }
        let mut pool: Vec<Option<Track>> = s.queue[start..end].iter().cloned().map(Some).collect();
        let mut out = Vec::with_capacity(pool.len());
        for id in new_ids {
            let Some(t) = pool.iter_mut().find(|t| t.as_ref().is_some_and(|t| &t.id == id)) else {
                return false;
            };
            out.extend(t.take());
        }
        s.queue.splice(start..end, out);
        true
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
    pending_rating: Arc<Mutex<Option<(bool, bool)>>>,
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
    let rating = extras.pending_rating.lock().take();
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
    // Prefer a known length on the incoming track so the seek bar doesn't
    // keep the outgoing duration after a mid-song skip crossfade.
    if let Some(d) = track.duration.filter(|&d| d > 0) {
        s.duration = d as f64;
    }
    s.track = Some(track);
    s.index = index;
    s.loading = false;
    s.playing = true;
    s.position = 0.0;
    s.error = None;
    if let Some((liked, disliked)) = rating {
        s.liked = liked;
        s.disliked = disliked;
    }
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
    xf_milli: Arc<AtomicU32>,
    extras: AudioExtras,
) {
    let wanted = crate::output::prefs().0;
    let (mut _stream, handle) = match crate::output::open(wanted.as_deref()) {
        Some((s, h, _)) => (s, h),
        None => {
            // Keep the thread alive so commands don't fail the whole app.
            for _ in rx {
                // discard
            }
            return;
        }
    };
    let mut sink = Sink::try_new(&handle).expect("sink");
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
        xf_milli,
    );
    sink.append(mix.source());
    sink.play();
    let tap = mix.tap();
    // Bumped by every command that changes what should be playing, so a
    // slow crossfade analysis can't land on top of a newer action.
    let mut generation: u64 = 0;
    // Normalization gain for the next deck, and whether a crossfade is
    // still being analysed (so a live gain lands on the right deck).
    let mut next_gain: f32 = 1.0;
    let mut xf_pending = false;

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
                | Cmd::Seek(_))
        ) {
            generation = generation.wrapping_add(1);
            xf_pending = false;
        }
        if matches!(
            cmd,
            Ok(Cmd::Play(..) | Cmd::Arm(..) | Cmd::Crossfade(..) | Cmd::Gapless(..) | Cmd::Cut(..))
        ) {
            mix.set_next_gain(next_gain);
        }
        match cmd {
            Ok(Cmd::Play(audio, start)) => {
                match make_source(audio, start) {
                    Ok(src) => {
                        state.lock().position = start;
                        mix.play(src, crate::mix::CLICK_FADE);
                        mix.set_origin(start);
                    }
                    Err(e) => {
                        eprintln!("decode: {e}");
                        state.lock().error = Some("Can't play this track".into());
                        phase.store(0, Ordering::Relaxed);
                        buffering.store(false, Ordering::Relaxed);
                        // Don't auto-advance from the audio thread - the
                        // emit loop / user decides. Mid-play reopen misses
                        // used to leave the UI spinning on buffering.
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
                        mix.set_origin(start);
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
            Ok(Cmd::Crossfade(audio, start, cfg, hints)) => {
                if cfg.needs_analysis() {
                    let tx = extras.tx.clone();
                    let tap = tap.clone();
                    let gen = generation;
                    xf_pending = true;
                    let spawned = std::thread::Builder::new()
                        .name("tarn-xf-analysis".into())
                        .spawn(move || match prepare_transition(audio, start, &cfg, &tap, &hints) {
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
                            let plan = crate::mix::XfPlan {
                                origin_s: start,
                                ..plain_plan(&cfg)
                            };
                            mix.crossfade_plan(src, plan);
                            flush_pending_adopt(&extras, &state);
                        }
                        Err(e) => eprintln!("crossfade: {e}"),
                    }
                }
            }
            Ok(Cmd::XfReady(src, plan, gen)) => {
                if gen == generation {
                    xf_pending = false;
                    mix.set_next_gain(next_gain);
                }
                if gen != generation {
                    eprintln!("crossfade: analysis superseded, dropping");
                    *extras.pending_adopt.lock() = None;
                    *extras.pending_rating.lock() = None;
                } else if mix.is_paused() {
                    // Paused while analysing: swap silently, stay paused.
                    let skipped = plan.skipped_frames;
                    mix.arm(*src);
                    // arm() zeros the clock; keep smart-start / beat trim.
                    mix.apply_skipped_frames(skipped);
                    let sr = sample_rate.load(Ordering::Relaxed).max(1) as f64;
                    mix.set_origin(plan.origin_s + skipped as f64 / sr);
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
                        // The UI switches to the new track now (position 0);
                        // restart the clock so it doesn't read as already over.
                        samples.store(0, Ordering::Relaxed);
                        flush_pending_adopt(&extras, &state);
                    }
                    Err(e) => eprintln!("gapless: {e}"),
                }
            }
            Ok(Cmd::Cut(audio, start)) => {
                match make_source(audio, start) {
                    Ok(src) => {
                        mix.cut(src, crate::mix::CLICK_FADE, start);
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
                            mix.set_origin(secs);
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
            Ok(Cmd::Output(name)) => {
                // Same mixer, new device: the old sink stops pulling before
                // the new one starts, so nothing plays twice.
                if let Some((s, h, _)) = crate::output::open(name.as_deref()) {
                    match Sink::try_new(&h) {
                        Ok(new_sink) => {
                            new_sink.set_volume(state.lock().volume);
                            sink.stop();
                            new_sink.append(mix.source());
                            new_sink.play();
                            sink = new_sink;
                            _stream = s;
                        }
                        Err(e) => eprintln!("audio output: sink: {e}"),
                    }
                }
            }
            Ok(Cmd::Eq(gains, on)) => mix.set_eq(gains, on),
            Ok(Cmd::Fx(fx)) => mix.set_fx(fx),
            Ok(Cmd::NextGain(g)) => next_gain = g,
            Ok(Cmd::LiveGain(g)) => {
                if xf_pending {
                    // The track it belongs to isn't in the mixer yet.
                    next_gain = g;
                } else {
                    mix.set_live_gain(g, 2.0);
                }
            }
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
        keylock: cfg.keylock,
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
    hints: &XfHints,
) -> Result<(crate::decode::AudioSource, crate::mix::XfPlan)> {
    use crate::transition::{analyze, match_rate, EnvBuilder, FrameRes, MIN_CONFIDENCE, SILENCE};
    use rodio::Source;
    let t0 = Instant::now();
    let mut src = make_source(audio.clone(), start)?;
    let sr = src.sample_rate().max(1);
    let sr_f = sr as f64;
    let ch = src.channels().max(1) as usize;
    let mut plan = crate::mix::XfPlan {
        origin_s: start,
        ..plain_plan(cfg)
    };

    // Incoming head: (bpm, first beat in frames from `start`, confidence).
    let mut in_beat: Option<(f64, f64)> = None;
    let mut lead_silence = 0u64;
    if let Some(a) = hints.inc.as_ref() {
        // Cached analysis: no probe decode needed, and it works for
        // mid-track starts too.
        if start <= 0.05 {
            lead_silence = ((a.lead_silence_s as f64 - start).max(0.0) * sr_f) as u64;
        }
        if let Some(g) = a.head {
            in_beat = Some((g.bpm, (g.phase_s - start) * sr_f));
        }
    } else if start <= 0.05 {
        // Look at the head of the incoming track with a second decoder so
        // the playback one stays untouched. Only from the top: a mid-track
        // start would make the decoder wait for the whole download.
        let mut env = EnvBuilder::new(sr, 12.0);
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
        if cfg.bpm_match {
            let full: Vec<f32> = env.full.iter().copied().collect();
            let low: Vec<f32> = env.low.iter().copied().collect();
            in_beat = analyze(&full, &low, sr)
                .filter(|b| b.confidence >= MIN_CONFIDENCE)
                .map(|b| (b.bpm, env.window_start() as f64 + b.phase));
        }
    }

    let mut skip = 0u64;
    if cfg.smart {
        skip = lead_silence.min(8 * sr as u64);
    }
    if cfg.bpm_match {
        // Outgoing: the live tap when it heard enough, else the cached
        // grid of the track's ending (e.g. a skip right after it started).
        let live = tap
            .outgoing_beat()
            .filter(|b| b.confidence >= MIN_CONFIDENCE)
            .map(|b| (b.bpm, b.period, b.phase));
        let cached = || {
            let g = hints.out.as_ref()?.tail?;
            let (_, rate, out_sr) = tap.outgoing_clock();
            let out_sr = out_sr.max(1) as f64;
            Some((g.bpm * rate, g.period_s() * out_sr, g.phase_s * out_sr))
        };
        let outb = live.or_else(cached);
        match (outb, in_beat) {
            (Some((o_bpm, o_period, o_phase)), Some((i_bpm, i_phase))) => {
                if let Some((rate, eff)) = match_rate(o_bpm, i_bpm, cfg.bpm_max_pct as f64) {
                    plan.in_rate = rate;
                    eprintln!(
                        "bpm: out {:.1} / in {:.1} (as {:.1}) → incoming at {:.2}%{}",
                        o_bpm,
                        i_bpm,
                        eff,
                        (rate - 1.0) * 100.0,
                        if hints.inc.is_some() || hints.out.is_some() { " (cached)" } else { "" }
                    );
                    if cfg.beat_align {
                        let p_in = 60.0 * sr_f / eff;
                        let k = ((skip as f64 - i_phase) / p_in).ceil().max(0.0);
                        let first = i_phase + k * p_in;
                        if first >= skip as f64 && first - (skip as f64) <= p_in {
                            skip = first.round() as u64;
                        }
                        plan.out_grid = Some((o_period, o_phase));
                    }
                } else {
                    eprintln!(
                        "bpm: out {:.1} / in {:.1} needs more than ±{:.1}%, not matching",
                        o_bpm, i_bpm, cfg.bpm_max_pct
                    );
                }
            }
            _ => eprintln!("bpm: tempo unclear, crossfading without matching"),
        }
    }
    if skip > 0 {
        let skip_secs = skip as f64 / sr_f;
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
    fn rapid_manual_skip_skips_crossfade() {
        let p = Player::start();
        p.set_crossfade(true, 4.0, false, true);
        let a = t("a", "A");
        let b = t("b", "B");
        let c = t("c", "C");
        assert!(p.should_crossfade(Some(&a), &b, true), "first skip fades");
        p.note_manual_nav();
        assert!(
            !p.should_crossfade(Some(&b), &c, true),
            "second skip within 2s must cut"
        );
        // Natural end-of-track handoff is unaffected by the manual timer.
        assert!(p.should_crossfade(Some(&b), &c, false));
    }

    #[test]
    fn manual_skip_during_live_fade_skips_crossfade() {
        let p = Player::start();
        p.set_crossfade(true, 4.0, false, true);
        let a = t("a", "A");
        let b = t("b", "B");
        p.xf_milli.store(400, Ordering::Relaxed);
        assert!(
            !p.should_crossfade(Some(&a), &b, true),
            "skip while a shaped fade is audible must cut"
        );
    }

    #[test]
    fn probing_should_crossfade_does_not_poison_rapid_window() {
        let p = Player::start();
        p.set_crossfade(true, 4.0, false, true);
        let a = t("a", "A");
        let b = t("b", "B");
        assert!(p.should_crossfade(Some(&a), &b, true));
        assert!(
            p.should_crossfade(Some(&a), &b, true),
            "without note_manual_nav, a second probe must still allow fade"
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

#[cfg(test)]
mod reorder_tests {
    use super::*;

    #[test]
    fn reorders_only_an_unchanged_window() {
        let p = Player::start();
        let q: Vec<Track> = (0..6)
            .map(|i| Track { id: format!("t{i}"), ..Default::default() })
            .collect();
        p.replace_queue(q, 0);
        let ids = |p: &Player| p.queue_and_index().0.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        let win: Vec<String> = ["t2", "t3", "t4", "t5"].iter().map(|s| s.to_string()).collect();
        let new: Vec<String> = ["t4", "t2", "t5", "t3"].iter().map(|s| s.to_string()).collect();
        assert!(p.reorder_window(2, &win, &new));
        assert_eq!(ids(&p), ["t0", "t1", "t4", "t2", "t5", "t3"]);
        // Stale view of the queue: refuse.
        assert!(!p.reorder_window(2, &win, &new));
        // Never touches the current track.
        let w0: Vec<String> = ["t0", "t1"].iter().map(|s| s.to_string()).collect();
        assert!(!p.reorder_window(0, &w0, &w0));
    }
}
