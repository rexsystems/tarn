use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use rodio::{OutputStream, Sink};

use crate::models::{PlayerState, Track};
use crate::ytm::Stream;

/// Commands sent to the audio thread.
enum Cmd {
    Play(Arc<crate::decode::GrowingBody>, f64),
    Arm(Arc<crate::decode::GrowingBody>, f64),
    Crossfade(Arc<crate::decode::GrowingBody>, f64, f32),
    Gapless(Arc<crate::decode::GrowingBody>, f64),
    Cut(Arc<crate::decode::GrowingBody>, f64),
    Pause,
    Resume,
    Stop,
    Seek(f64),
    Volume(f32),
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
    /// Equal-power overlap of `f32` seconds.
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
    album_cont: Arc<AtomicBool>,
    blending: Arc<AtomicBool>,
    /// If the user hits play while a session is still arming.
    want_play: Arc<AtomicBool>,
    buffering: Arc<AtomicBool>,
    recover: Arc<AtomicBool>,
    has_src: Arc<AtomicBool>,
    recovering: Arc<AtomicBool>,
    last_ended: Arc<Mutex<Option<String>>>,
    last_recover: Arc<Mutex<Option<Instant>>>,
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
        let album_cont = Arc::new(AtomicBool::new(true));
        let blending = Arc::new(AtomicBool::new(false));
        let want_play = Arc::new(AtomicBool::new(false));
        let buffering = Arc::new(AtomicBool::new(false));
        let recover = Arc::new(AtomicBool::new(false));
        let has_src = Arc::new(AtomicBool::new(false));
        let recovering = Arc::new(AtomicBool::new(false));
        let last_ended = Arc::new(Mutex::new(None));
        let last_recover = Arc::new(Mutex::new(None));

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
            album_cont,
            blending,
            want_play,
            buffering,
            recover,
            has_src,
            recovering,
            last_ended,
            last_recover,
        }
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
        let body_buf = body.as_ref().is_some_and(|b| b.is_buffering() || b.is_stalled());
        s.buffering = ph == 4 || self.buffering.load(Ordering::Relaxed) || body_buf;
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
        self.ended.store(false, Ordering::Relaxed);
        self.recover.store(false, Ordering::Relaxed);
        self.buffering.store(false, Ordering::Relaxed);
        *self.last_ended.lock() = None;
        self.phase
            .store(if loading { 3 } else { 2 }, Ordering::Relaxed);
        self.want_play.store(false, Ordering::Relaxed);
        let mut s = self.state.lock();
        if let Some(d) = track.duration {
            if d > 0 {
                s.duration = d as f64;
            }
        }
        s.track = Some(track);
        s.queue = queue;
        s.index = index;
        s.loading = loading;
        s.playing = false;
        s.position = position.max(0.0);
        s.error = None;
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
        self.samples.store(0, Ordering::Relaxed);
        {
            let mut s = self.state.lock();
            s.position = start;
        }
        self.ended.store(false, Ordering::Relaxed);
        if !matches!(style, PlayStyle::Crossfade(_) | PlayStyle::Gapless | PlayStyle::Cut) {
            *self.last_ended.lock() = None;
        }
        self.recovering.store(false, Ordering::Relaxed);
        self.blending.store(
            matches!(style, PlayStyle::Crossfade(_) | PlayStyle::Gapless | PlayStyle::Cut),
            Ordering::Relaxed,
        );
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
            PlayStyle::Crossfade(s) => Cmd::Crossfade(audio, start, s),
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

    pub fn set_crossfade(&self, on: bool, seconds: f64, album_continuity: bool) {
        self.xf_on.store(on, Ordering::Relaxed);
        let ms = (seconds.clamp(0.0, 12.0) * 1000.0).round() as u32;
        self.xf_ms.store(ms, Ordering::Relaxed);
        self.album_cont.store(album_continuity, Ordering::Relaxed);
    }

    pub fn crossfade_secs(&self) -> f32 {
        self.xf_ms.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn should_crossfade(&self, from: Option<&Track>, to: &Track) -> bool {
        if !self.xf_on.load(Ordering::Relaxed) {
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
        let xf = self.should_crossfade(s.track.as_ref(), &track);
        let lead = if xf {
            self.crossfade_secs() as f64 + 0.2
        } else {
            0.35
        };
        if s.position + 0.04 < (s.duration - lead).max(0.0) {
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
    }

    pub fn adopt_track(&self, track: Track, index: usize) {
        self.ended.store(false, Ordering::Relaxed);
        *self.last_ended.lock() = None;
        let mut s = self.state.lock();
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
                // Still arming a restored track — play as soon as it's ready.
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
        self.buffering.load(Ordering::Relaxed)
            || self.phase.load(Ordering::Relaxed) == 4
            || self
                .current_audio
                .lock()
                .as_ref()
                .is_some_and(|b| b.is_buffering())
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
        let v = v.clamp(0.0, 1.0);
        self.state.lock().volume = v;
        let _ = self.tx.send(Cmd::Volume(v));
    }
    pub fn take_ended(&self) -> bool {
        if self.blending.load(Ordering::Relaxed) {
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
        self.state.lock().shuffle = on;
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
        for (k, t) in tracks.into_iter().enumerate() {
            s.queue.insert(i + k, t);
        }
    }

    pub fn append(&self, tracks: Vec<Track>) {
        self.state.lock().queue.extend(tracks);
    }

    pub fn append_unique(&self, tracks: Vec<Track>) {
        let mut s = self.state.lock();
        let existing: std::collections::HashSet<_> =
            s.queue.iter().map(|t| t.id.clone()).collect();
        for t in tracks {
            if !t.id.is_empty() && !existing.contains(&t.id) {
                s.queue.push(t);
            }
        }
    }

    /// Swap the queue without interrupting the current source. Used when
    /// starting radio of the track that is already playing.
    pub fn replace_queue(&self, queue: Vec<Track>, index: usize) {
        let mut s = self.state.lock();
        s.queue = queue;
        s.index = index;
    }

    pub fn remove_at(&self, i: usize) {
        let mut s = self.state.lock();
        if i < s.queue.len() {
            s.queue.remove(i);
            if s.index > i {
                s.index -= 1;
            }
            if s.index >= s.queue.len() && !s.queue.is_empty() {
                s.index = s.queue.len() - 1;
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
        if s.shuffle && s.queue.len() > 1 {
            // pick a different random index
            let n = s.queue.len();
            let mut idx = (s.index + 1) % n;
            // Prefer an unplayed-looking next: just walk a shuffled offset.
            let step = (s.position as usize).wrapping_mul(17).wrapping_add(3).max(1);
            for k in 0..n {
                let j = (s.index + 1 + k * step) % n;
                if j != s.index {
                    idx = j;
                    break;
                }
            }
            return NextChoice::Play(idx);
        }
        let n = s.index + 1;
        if n < s.queue.len() {
            NextChoice::Play(n)
        } else if s.repeat == "all" {
            NextChoice::Play(0)
        } else if s.autoplay {
            NextChoice::Autoplay
        } else if user && !s.queue.is_empty() {
            NextChoice::Play(0)
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
    );
    sink.append(mix.source());
    sink.play();

    loop {
        let cmd = rx.recv_timeout(Duration::from_millis(80));
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
                match make_source(audio, start) {
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
                        state.lock().error = Some("Can't play this track".into());
                        phase.store(0, Ordering::Relaxed);
                    }
                }
            }
            Ok(Cmd::Crossfade(audio, start, secs)) => {
                match make_source(audio, start) {
                    Ok(src) => mix.crossfade(src, secs),
                    Err(e) => eprintln!("crossfade: {e}"),
                }
            }
            Ok(Cmd::Gapless(audio, start)) => {
                match make_source(audio, start) {
                    Ok(src) => mix.queue_gapless(src),
                    Err(e) => eprintln!("gapless: {e}"),
                }
            }
            Ok(Cmd::Cut(audio, start)) => {
                match make_source(audio, start) {
                    Ok(src) => mix.cut(src, crate::mix::CLICK_FADE),
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
