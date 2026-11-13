use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use rodio::{OutputStream, Sink, Source};

use crate::models::{PlayerState, Track};
use crate::ytm::Stream;

/// Commands sent to the audio thread.
enum Cmd {
    Play(Arc<Vec<u8>>, f64),
    Pause,
    Resume,
    Stop,
    Seek(f64),
    Volume(f32),
}

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
    current_audio: Arc<Mutex<Option<Arc<Vec<u8>>>>>,
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

        let samples_t = samples.clone();
        let rate_t = sample_rate.clone();
        let ch_t = channels.clone();
        let ended_t = ended.clone();
        let phase_t = phase.clone();
        let audio_t = current_audio.clone();
        let state_t = state.clone();

        std::thread::Builder::new()
            .name("tarn-audio".into())
            .spawn(move || {
                audio_thread(rx, samples_t, rate_t, ch_t, ended_t, phase_t, audio_t, state_t)
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
        }
    }

    pub fn snapshot(&self) -> PlayerState {
        let mut s = self.state.lock().clone();
        let rate = self.sample_rate.load(Ordering::Relaxed).max(1) as f64
            * self.channels.load(Ordering::Relaxed).max(1) as f64;
        let extra = self.samples.load(Ordering::Relaxed) as f64 / rate;
        // `position` in state is the seek origin; add consumed samples.
        if self.phase.load(Ordering::Relaxed) != 0 {
            s.position = (s.position + extra).min(if s.duration > 0.0 {
                s.duration
            } else {
                f64::MAX
            });
        }
        s.playing = self.phase.load(Ordering::Relaxed) == 1;
        s.loading = self.phase.load(Ordering::Relaxed) == 3;
        s
    }

    pub fn set_loading(&self, track: Track, queue: Vec<Track>, index: usize) {
        self.ended.store(false, Ordering::Relaxed);
        self.phase.store(3, Ordering::Relaxed);
        let mut s = self.state.lock();
        s.track = Some(track);
        s.queue = queue;
        s.index = index;
        s.loading = true;
        s.playing = false;
        s.position = 0.0;
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
        }
    }

    pub fn play_stream(&self, _stream: Stream, audio: Arc<Vec<u8>>, start: f64) -> Result<()> {
        *self.current_audio.lock() = Some(audio.clone());
        self.samples.store(0, Ordering::Relaxed);
        {
            let mut s = self.state.lock();
            s.position = start;
        }
        self.tx
            .send(Cmd::Play(audio, start))
            .map_err(|_| anyhow!("audio thread died"))?;
        Ok(())
    }

    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }
    pub fn resume(&self) {
        let _ = self.tx.send(Cmd::Resume);
    }
    pub fn toggle(&self) {
        match self.phase.load(Ordering::Relaxed) {
            1 => self.pause(),
            2 => self.resume(),
            _ => {}
        }
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
        self.ended.swap(false, Ordering::Relaxed)
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

fn audio_thread(
    rx: std::sync::mpsc::Receiver<Cmd>,
    samples: Arc<AtomicU64>,
    sample_rate: Arc<AtomicU32>,
    channels: Arc<AtomicU32>,
    ended: Arc<AtomicBool>,
    phase: Arc<AtomicU8>,
    current_audio: Arc<Mutex<Option<Arc<Vec<u8>>>>>,
    state: Arc<Mutex<PlayerState>>,
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

    loop {
        let cmd = rx.recv_timeout(Duration::from_millis(120));
        match cmd {
            Ok(Cmd::Play(audio, start)) => {
                sink.stop();
                match make_source(audio, start, samples.clone()) {
                    Ok(src) => {
                        sample_rate.store(src.sample_rate(), Ordering::Relaxed);
                        channels.store(src.channels() as u32, Ordering::Relaxed);
                        samples.store(0, Ordering::SeqCst);
                        state.lock().position = start;
                        sink.append(src);
                        sink.play();
                        phase.store(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        eprintln!("decode: {e}");
                        state.lock().error = Some("Can't play this track".into());
                        phase.store(0, Ordering::Relaxed);
                    }
                }
            }
            Ok(Cmd::Pause) => {
                sink.pause();
                phase.store(2, Ordering::Relaxed);
            }
            Ok(Cmd::Resume) => {
                if !sink.empty() {
                    sink.play();
                    phase.store(1, Ordering::Relaxed);
                }
            }
            Ok(Cmd::Stop) => {
                sink.stop();
                phase.store(0, Ordering::Relaxed);
            }
            Ok(Cmd::Seek(secs)) => {
                sink.stop();
                let audio = current_audio.lock().clone();
                if let Some(audio) = audio {
                    match make_source(audio, secs, samples.clone()) {
                        Ok(src) => {
                            sample_rate.store(src.sample_rate(), Ordering::Relaxed);
                            channels.store(src.channels() as u32, Ordering::Relaxed);
                            samples.store(0, Ordering::SeqCst);
                            state.lock().position = secs;
                            sink.append(src);
                            sink.play();
                            phase.store(1, Ordering::Relaxed);
                        }
                        Err(e) => eprintln!("seek: {e}"),
                    }
                }
            }
            Ok(Cmd::Volume(v)) => sink.set_volume(v),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let ph = phase.load(Ordering::Relaxed);
                if ph == 1 && sink.empty() {
                    phase.store(0, Ordering::Relaxed);
                    ended.store(true, Ordering::Relaxed);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Decode a few samples so callers can verify the in-process pipeline
/// without opening an audio device.
pub fn decode_head(audio: &[u8], n: usize) -> Result<(u32, u16, usize)> {
    crate::decode::decode_head(audio, n)
}

fn make_source(
    audio: Arc<Vec<u8>>,
    start: f64,
    samples: Arc<AtomicU64>,
) -> Result<crate::decode::AudioSource> {
    crate::decode::open(audio, start, samples)
}
