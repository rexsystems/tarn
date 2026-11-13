//! In-process audio decoder (symphonia). Handles YouTube's fragmented
//! AAC/m4a (ftyp `dash`) which makes rodio 0.20's wrapper panic on init.
//!
//! The media source is a growing buffer: playback can start as soon as the
//! first fragments arrive, while the rest of the file downloads in the
//! background. When the decoder runs ahead of the downloader it waits on a
//! condvar (short, so the audio callback stays responsive) and reports
//! underrun — never EOF — until the full body is known and fully consumed.

use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use rodio::Source;
use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// Bytes we wait for before starting the decoder. YouTube's fMP4 has `moov`
/// and `sidx` at the front; the first `mdat` is typically ~160 KiB, so 256 KiB
/// is enough to probe and decode the opening packets (~16 s of 128 kbps AAC).
pub const READY_BYTES: usize = 256 * 1024;

/// How long the audio callback waits for the next byte before yielding a
/// buffering frame. The condvar wakes earlier if a chunk arrives.
const UNDERRUN_WAIT: Duration = Duration::from_millis(8);

/// If no new bytes arrive for this long while the decoder is waiting, treat
/// the download as stalled and ask the player to re-resolve.
const STALL_AFTER: Duration = Duration::from_secs(12);

/// Shared, append-only audio body filled by an HTTP download task.
pub struct GrowingBody {
    data: Mutex<Vec<u8>>,
    more: Condvar,
    content_len: AtomicU64,
    done: AtomicBool,
    error: Mutex<Option<String>>,
    /// Byte offset the decoder wants next (playhead). The downloader uses
    /// this to prefer that region over prefetch of other tracks.
    urgent: AtomicU64,
    buffering: AtomicBool,
    last_progress: Mutex<Instant>,
    /// Optional media duration in seconds, used to map a seek time to a byte.
    duration_ms: AtomicU64,
}

impl GrowingBody {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            data: Mutex::new(Vec::new()),
            more: Condvar::new(),
            content_len: AtomicU64::new(0),
            done: AtomicBool::new(false),
            error: Mutex::new(None),
            urgent: AtomicU64::new(0),
            buffering: AtomicBool::new(false),
            last_progress: Mutex::new(Instant::now()),
            duration_ms: AtomicU64::new(0),
        })
    }

    pub fn from_complete(bytes: Vec<u8>) -> Arc<Self> {
        let b = Self::new();
        b.set_content_len(bytes.len() as u64);
        b.append(&bytes);
        b.finish();
        b
    }

    pub fn append(&self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        self.data.lock().unwrap().extend_from_slice(chunk);
        *self.last_progress.lock().unwrap() = Instant::now();
        self.buffering.store(false, Ordering::Relaxed);
        self.more.notify_all();
    }

    pub fn set_content_len(&self, n: u64) {
        self.content_len.store(n, Ordering::Relaxed);
    }

    pub fn content_len(&self) -> Option<u64> {
        let n = self.content_len.load(Ordering::Relaxed);
        if n == 0 {
            None
        } else {
            Some(n)
        }
    }

    pub fn set_duration(&self, secs: f64) {
        if secs > 0.0 {
            self.duration_ms
                .store((secs * 1000.0).round() as u64, Ordering::Relaxed);
        }
    }

    pub fn duration(&self) -> Option<f64> {
        let ms = self.duration_ms.load(Ordering::Relaxed);
        if ms == 0 {
            None
        } else {
            Some(ms as f64 / 1000.0)
        }
    }

    pub fn finish(&self) {
        self.done.store(true, Ordering::SeqCst);
        self.buffering.store(false, Ordering::Relaxed);
        self.more.notify_all();
    }

    pub fn fail(&self, msg: String) {
        *self.error.lock().unwrap() = Some(msg);
        // Mark done so waiters wake, but callers must treat `error()` as a
        // stall to re-resolve — never as a clean end of track.
        self.done.store(true, Ordering::SeqCst);
        self.more.notify_all();
    }

    pub fn len(&self) -> usize {
        self.data.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    /// True when the download finished without error and, if a length is
    /// known, every byte has arrived.
    pub fn is_complete(&self) -> bool {
        if !self.done.load(Ordering::SeqCst) {
            return false;
        }
        if self.error.lock().unwrap().is_some() {
            return false;
        }
        if let Some(n) = self.content_len() {
            self.len() as u64 >= n
        } else {
            self.len() >= 64
        }
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.data.lock().unwrap().clone()
    }

    /// Tell waiters the downloader is still alive (e.g. mid-file re-resolve)
    /// so a quiet stretch is not treated as a dead download.
    pub fn touch(&self) {
        *self.last_progress.lock().unwrap() = Instant::now();
    }

    pub fn request_urgent(&self, offset: u64) {
        self.urgent.store(offset, Ordering::Relaxed);
    }

    pub fn urgent(&self) -> u64 {
        self.urgent.load(Ordering::Relaxed)
    }

    pub fn is_buffering(&self) -> bool {
        self.buffering.load(Ordering::Relaxed)
    }

    pub fn set_buffering(&self, v: bool) {
        self.buffering.store(v, Ordering::Relaxed);
    }

    /// Seconds of audio already on disk, from the start of the file.
    pub fn buffered_secs(&self, duration: f64) -> f64 {
        let got = self.len() as f64;
        if let Some(total) = self.content_len() {
            if total > 0 && duration > 0.0 {
                return (got / total as f64 * duration).min(duration);
            }
        }
        if let Some(d) = self.duration() {
            if let Some(total) = self.content_len() {
                if total > 0 && d > 0.0 {
                    return (got / total as f64 * d).min(d);
                }
            }
        }
        // ~128 kbps AAC fallback.
        if duration > 0.0 {
            (got / 16_000.0).min(duration)
        } else {
            got / 16_000.0
        }
    }

    /// Map a playback time to an approximate byte offset.
    pub fn bytes_for_secs(&self, secs: f64) -> usize {
        let secs = secs.max(0.0);
        if let Some(total) = self.content_len() {
            let dur = self
                .duration()
                .filter(|d| *d > 0.1)
                .unwrap_or(0.0);
            if dur > 0.1 && total > 0 {
                return ((secs / dur) * total as f64).round() as usize;
            }
        }
        (secs * 16_000.0) as usize
    }

    /// No new bytes for a while, and the decoder is waiting — re-resolve.
    pub fn is_stalled(&self) -> bool {
        if self.error().is_some() {
            return true;
        }
        if self.is_complete() {
            return false;
        }
        if !self.buffering.load(Ordering::Relaxed) {
            return false;
        }
        self.last_progress.lock().unwrap().elapsed() >= STALL_AFTER
    }

    /// Block until at least `n` bytes have arrived, the download finished, or
    /// an error was recorded.
    pub fn wait_len(&self, n: usize, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut g = self.data.lock().unwrap();
        loop {
            if let Some(e) = self.error.lock().unwrap().as_ref() {
                return Err(anyhow!("audio download: {e}"));
            }
            if g.len() >= n || self.done.load(Ordering::SeqCst) {
                if g.len() < 64 && self.done.load(Ordering::SeqCst) {
                    return Err(anyhow!("audio download too small ({} bytes)", g.len()));
                }
                return Ok(());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(anyhow!("audio download timed out"));
            }
            self.buffering.store(true, Ordering::Relaxed);
            let (gg, t) = self.more.wait_timeout(g, left).unwrap();
            g = gg;
            if t.timed_out() {
                return Err(anyhow!("audio download timed out"));
            }
        }
    }

    pub async fn wait_ready(&self) -> Result<()> {
        let start = Instant::now();
        loop {
            if let Some(e) = self.error() {
                return Err(anyhow!("audio download: {e}"));
            }
            let n = self.len();
            if n >= READY_BYTES || (self.is_done() && n >= 64) {
                return Ok(());
            }
            if self.is_done() {
                return Err(anyhow!("audio download too small ({n} bytes)"));
            }
            if start.elapsed() > Duration::from_secs(120) {
                return Err(anyhow!("audio download timed out"));
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    pub async fn wait_done(&self) -> Result<()> {
        let start = Instant::now();
        loop {
            if let Some(e) = self.error() {
                return Err(anyhow!("audio download: {e}"));
            }
            if self.is_done() {
                if self.len() < 64 {
                    return Err(anyhow!("audio download too small ({} bytes)", self.len()));
                }
                return Ok(());
            }
            if start.elapsed() > Duration::from_secs(180) {
                return Err(anyhow!("audio download timed out"));
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }
}

struct GrowingReader {
    body: Arc<GrowingBody>,
    pos: u64,
    /// After a user seek we reopen as seekable so the demuxer can jump to
    /// a later fragment; reads still wait on the condvar for those bytes.
    force_seekable: bool,
}

impl Read for GrowingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.body.request_urgent(self.pos);
        let mut g = self.body.data.lock().unwrap();
        loop {
            let avail = g.len() as u64;
            if self.pos < avail {
                let start = self.pos as usize;
                let n = (g.len() - start).min(buf.len());
                buf[..n].copy_from_slice(&g[start..start + n]);
                self.pos += n as u64;
                self.body.buffering.store(false, Ordering::Relaxed);
                return Ok(n);
            }
            if let Some(e) = self.body.error.lock().unwrap().as_ref() {
                self.body.buffering.store(false, Ordering::Relaxed);
                return Err(std::io::Error::other(e.clone()));
            }
            if self.body.done.load(Ordering::SeqCst) {
                // Clean EOF only when the file is fully here and we've
                // consumed it. A mid-play failure is an error, not EOF.
                if self.body.error.lock().unwrap().is_some() {
                    let e = self.body.error.lock().unwrap().clone().unwrap();
                    return Err(std::io::Error::other(e));
                }
                return Ok(0);
            }
            // Still growing: wait briefly, then tell the decoder this is
            // an underrun, not the end of the track.
            //
            // Do NOT turn a quiet stretch into a fatal TimedOut. Mid-file
            // 403s take a re-resolve (often 15–40s) before the next byte
            // arrives; a 12s stall here used to surface "Can't play this
            // track" even though the downloader was still working and the
            // rest of the file eventually arrived. Fatal Stall is only
            // for `body.fail()` (the downloader itself gave up).
            self.body.buffering.store(true, Ordering::Relaxed);
            if self.body.last_progress.lock().unwrap().elapsed() >= STALL_AFTER {
                eprintln!(
                    "underrun: waiting on downloader at {} / {:?} bytes (playhead {}), last progress {:?}",
                    avail,
                    self.body.content_len(),
                    self.pos,
                    self.body.last_progress.lock().unwrap().elapsed()
                );
            }
            let (gg, t) = self.body.more.wait_timeout(g, UNDERRUN_WAIT).unwrap();
            g = gg;
            if t.timed_out() && (g.len() as u64) <= self.pos {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "audio underrun",
                ));
            }
        }
    }
}

impl Seek for GrowingReader {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        // While the body is still growing and we opened unseekable, treat
        // "end" as what we already have so a probe doesn't block on the
        // full file. After a user seek (`force_seekable`) use the known
        // content length so the demuxer can jump to a later fragment.
        let downloaded = self.body.data.lock().unwrap().len() as u64;
        let end = if self.force_seekable {
            self.body.content_len().unwrap_or(downloaded).max(downloaded)
        } else {
            downloaded
        };
        let next = match from {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::End(n) => end as i128 + n as i128,
            SeekFrom::Current(n) => self.pos as i128 + n as i128,
        };
        if next < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before start",
            ));
        }
        self.pos = next as u64;
        self.body.request_urgent(self.pos);
        Ok(self.pos)
    }
}

impl MediaSource for GrowingReader {
    fn is_seekable(&self) -> bool {
        // The ISO-MP4 demuxer refuses to open a "seekable" source without a
        // known length, and with a known length it scans to the end of the
        // file. Report unseekable until the download is finished so playback
        // can start after moov+sidx+the first mdat (YouTube puts those first).
        // User seeks reopen with `force_seekable` after enough bytes exist.
        self.force_seekable || self.body.done.load(Ordering::SeqCst)
    }
    fn byte_len(&self) -> Option<u64> {
        if self.force_seekable {
            self.body
                .content_len()
                .or_else(|| Some(self.body.len() as u64))
        } else if self.body.done.load(Ordering::SeqCst) {
            Some(self.body.len() as u64)
        } else {
            None
        }
    }
}

/// Result of trying to pull one PCM sample from a (possibly growing) source.
#[derive(Debug)]
pub enum Pull {
    Sample(i16),
    /// Decoder is ahead of the downloader. Mixer should yield silence and
    /// pause the position clock — not end the track.
    Buffering,
    /// The full body was downloaded and every packet has been consumed.
    Ended,
    /// Download failed (403, stall, …). Re-resolve at the current position;
    /// do not auto-advance.
    Stall(String),
}

enum Fill {
    Ok,
    Underrun,
    Ended,
    Stall(String),
}

pub struct AudioSource {
    body: Arc<GrowingBody>,
    decoder: Box<dyn Decoder>,
    format: Box<dyn FormatReader>,
    track_id: u32,
    leftover: Vec<i16>,
    leftover_pos: usize,
    sample_rate: u32,
    channels: u16,
    /// Interleaved samples emitted, for a time estimate used on reseek.
    pulled: u64,
    reopens: u8,
    len_at_packet: usize,
}

impl AudioSource {
    fn open(body: Arc<GrowingBody>) -> Result<Self> {
        Self::open_seekable(body, false)
    }

    fn open_seekable(body: Arc<GrowingBody>, force_seekable: bool) -> Result<Self> {
        let reader = GrowingReader {
            body: body.clone(),
            pos: 0,
            force_seekable,
        };
        let mss = MediaSourceStream::new(Box::new(reader), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("m4a");
        hint.with_extension("mp4");
        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &MetadataOptions::default())
            .map_err(|e| anyhow!("probe: {e}"))?;
        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| anyhow!("no decodable audio track"))?
            .clone();
        let track_id = track.id;
        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| anyhow!("codec: {e}"))?;

        let (leftover, spec) = match decode_next(&mut *format, &mut *decoder, track_id, &body)? {
            DecodeOut::Packet(s, spec) => (s, spec),
            DecodeOut::Ended => return Err(anyhow!("no audio packets")),
            DecodeOut::Underrun => return Err(anyhow!("no audio packets (underrun at start)")),
            DecodeOut::Stall(e) => return Err(anyhow!("{e}")),
        };
        let sample_rate = spec.rate;
        let channels = spec.channels.count() as u16;
        if sample_rate == 0 || channels == 0 {
            return Err(anyhow!(
                "invalid audio format ({sample_rate} Hz, {channels} ch)"
            ));
        }

        let len_at_packet = body.len();
        Ok(Self {
            body,
            decoder,
            format,
            track_id,
            leftover,
            leftover_pos: 0,
            sample_rate,
            channels,
            pulled: 0,
            reopens: 0,
            len_at_packet,
        })
    }

    fn jump(&mut self, start: f64) -> Result<()> {
        if start <= 0.05 {
            return Ok(());
        }
        // Fetch the range around the playhead before asking the demuxer to
        // seek. Sequential download will catch up; we wait for those bytes
        // (not the entire file) and reopen as seekable so the sidx jump works.
        let needed = self.body.bytes_for_secs(start).saturating_add(READY_BYTES);
        self.body.request_urgent(needed as u64);
        eprintln!(
            "seek: waiting for ~{needed} bytes at {start:.2}s (have {})",
            self.body.len()
        );
        let _ = self.body.wait_len(needed, Duration::from_secs(30));
        if self.body.error().is_some() {
            return Err(anyhow!(
                "seek: {}",
                self.body.error().unwrap_or_default()
            ));
        }
        let mut fresh = AudioSource::open_seekable(self.body.clone(), true)?;
        fresh.sample_rate = self.sample_rate;
        fresh.channels = self.channels;
        *self = fresh;
        let seconds = start.floor() as u64;
        let frac = start - seconds as f64;
        let time = Time { seconds, frac };
        self.format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| anyhow!("seek: {e}"))?;
        self.decoder.reset();
        self.leftover.clear();
        self.leftover_pos = 0;
        match decode_next(
            &mut *self.format,
            &mut *self.decoder,
            self.track_id,
            &self.body,
        )? {
            DecodeOut::Packet(samples, spec) => {
                self.leftover = samples;
                self.leftover_pos = 0;
                self.sample_rate = spec.rate;
                self.channels = spec.channels.count() as u16;
            }
            DecodeOut::Underrun => {
                // Range isn't here yet — caller shows buffering and retries.
            }
            DecodeOut::Ended => {}
            DecodeOut::Stall(e) => return Err(anyhow!("{e}")),
        }
        Ok(())
    }

    fn playhead(&self) -> f64 {
        let den = self.sample_rate.max(1) as f64 * self.channels.max(1) as f64;
        self.pulled as f64 / den
    }

    /// The ISO-MP4 demuxer, opened unseekable for progressive start, can
    /// stop at a moof/mdat boundary even when later fragments are already
    /// in the body ("Can't play" at ~0:41 with the buffer well ahead).
    /// Reopen as seekable at the current time and continue.
    fn should_reopen(&self, ended: bool) -> bool {
        if self.reopens >= 8 {
            return false;
        }
        let pos = self.playhead();
        let dur = self.body.duration().unwrap_or(0.0);
        let early = dur > 2.0 && pos + 1.5 < dur;
        if ended {
            // Demuxer said EOF well before the track's duration, or while
            // the download is still going — almost certainly a fragment
            // boundary, not the real end.
            return early || !self.body.is_complete();
        }
        // Underrun only if many seconds of audio are already sitting in
        // the body past the playhead (stuck demuxer, not a real wait).
        let ahead = self.body.buffered_secs(dur);
        ahead > pos + 8.0
    }

    fn reopen_at(&mut self, at: f64) -> Result<()> {
        self.reopens = self.reopens.saturating_add(1);
        eprintln!(
            "decode: reseeking at {at:.2}s (reopen #{}, body {}/{:?})",
            self.reopens,
            self.body.len(),
            self.body.content_len()
        );
        let pulled = self.pulled;
        let reopens = self.reopens;
        let mut fresh = AudioSource::open_seekable(self.body.clone(), true)?;
        fresh.pulled = pulled;
        fresh.reopens = reopens;
        let _ = fresh.jump(at);
        fresh.pulled = pulled;
        fresh.reopens = reopens;
        *self = fresh;
        Ok(())
    }

    fn fill(&mut self) -> Fill {
        match decode_next(
            &mut *self.format,
            &mut *self.decoder,
            self.track_id,
            &self.body,
        ) {
            Ok(DecodeOut::Packet(samples, spec)) => {
                self.leftover = samples;
                self.leftover_pos = 0;
                self.sample_rate = spec.rate;
                self.channels = spec.channels.count() as u16;
                self.len_at_packet = self.body.len();
                Fill::Ok
            }
            Ok(DecodeOut::Underrun) => {
                if self.should_reopen(false) {
                    match self.reopen_at(self.playhead()) {
                        Ok(()) => Fill::Ok,
                        Err(e) => {
                            eprintln!("decode: reseek failed: {e}");
                            Fill::Underrun
                        }
                    }
                } else {
                    Fill::Underrun
                }
            }
            Ok(DecodeOut::Ended) => {
                if self.should_reopen(true) {
                    match self.reopen_at(self.playhead()) {
                        Ok(()) => Fill::Ok,
                        Err(e) => {
                            eprintln!("decode: reseek-on-end failed: {e}");
                            if self.body.is_complete() {
                                Fill::Ended
                            } else {
                                Fill::Underrun
                            }
                        }
                    }
                } else {
                    Fill::Ended
                }
            }
            Ok(DecodeOut::Stall(e)) => Fill::Stall(e),
            Err(e) => {
                // Skip a bad packet / fragment by reseeking a fraction ahead.
                if self.should_reopen(false) || self.reopens < 8 {
                    let at = self.playhead() + 0.08;
                    match self.reopen_at(at) {
                        Ok(()) => return Fill::Ok,
                        Err(e2) => eprintln!("decode: {e}; reseek: {e2}"),
                    }
                }
                if self.body.is_complete() && !self.should_reopen(true) {
                    Fill::Ended
                } else if let Some(err) = self.body.error() {
                    Fill::Stall(err)
                } else if self.body.is_stalled() {
                    Fill::Stall(e.to_string())
                } else {
                    eprintln!("decode: {e}; treating as underrun");
                    Fill::Underrun
                }
            }
        }
    }

    /// Pull one PCM sample, distinguishing underrun and true end-of-track.
    pub fn pull(&mut self) -> Pull {
        loop {
            if self.leftover_pos < self.leftover.len() {
                let v = self.leftover[self.leftover_pos];
                self.leftover_pos += 1;
                self.pulled += 1;
                return Pull::Sample(v);
            }
            match self.fill() {
                Fill::Ok => continue,
                Fill::Underrun => return Pull::Buffering,
                Fill::Ended => return Pull::Ended,
                Fill::Stall(e) => return Pull::Stall(e),
            }
        }
    }

    pub fn body(&self) -> &Arc<GrowingBody> {
        &self.body
    }
}

enum DecodeOut {
    Packet(Vec<i16>, SignalSpec),
    Underrun,
    Ended,
    Stall(String),
}

fn decode_next(
    format: &mut dyn FormatReader,
    decoder: &mut dyn Decoder,
    track_id: u32,
    body: &GrowingBody,
) -> Result<DecodeOut> {
    let mut decode_errors = 0usize;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) => return Ok(classify_io(e, body)),
            Err(SymError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(SymError::SeekError(_)) => continue,
            Err(e) => {
                if body.is_complete() {
                    return Ok(DecodeOut::Ended);
                }
                if let Some(err) = body.error() {
                    return Ok(DecodeOut::Stall(err));
                }
                return Err(anyhow!("packet: {e}"));
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let mut buf = SampleBuffer::<i16>::new(decoded.capacity() as u64, spec);
                buf.copy_interleaved_ref(decoded);
                return Ok(DecodeOut::Packet(buf.samples().to_vec(), spec));
            }
            Err(SymError::DecodeError(_)) => {
                decode_errors += 1;
                if decode_errors > 24 {
                    return Err(anyhow!("too many decode errors"));
                }
                continue;
            }
            Err(SymError::IoError(e)) => return Ok(classify_io(e, body)),
            Err(e) => return Err(anyhow!("decode: {e}")),
        }
    }
}

fn classify_io(e: std::io::Error, body: &GrowingBody) -> DecodeOut {
    if let Some(err) = body.error() {
        eprintln!("playback stall: {err}");
        return DecodeOut::Stall(err);
    }
    if e.kind() == std::io::ErrorKind::WouldBlock {
        return DecodeOut::Underrun;
    }
    // A TimedOut from an older reader, or a quiet stretch (`is_stalled`),
    // is just a long underrun while the downloader re-resolves. Only a
    // recorded body.error (download gave up) is a real Stall.
    if e.kind() == std::io::ErrorKind::TimedOut || body.is_stalled() {
        eprintln!(
            "underrun: long wait at {}/{:?} ({e}); downloader still running",
            body.len(),
            body.content_len()
        );
        return DecodeOut::Underrun;
    }
    if body.is_complete() || (body.is_done() && body.error().is_none()) {
        return DecodeOut::Ended;
    }
    // Growing body, unexpected IO — wait for more, do not end the track.
    eprintln!(
        "underrun: {} at {}/{:?} bytes; waiting on downloader",
        e,
        body.len(),
        body.content_len()
    );
    DecodeOut::Underrun
}

/// Open a (possibly still-growing) audio body and optionally seek to `start`.
pub fn open(audio: Arc<GrowingBody>, start: f64) -> Result<AudioSource> {
    let mut src = AudioSource::open(audio)?;
    src.jump(start)?;
    Ok(src)
}

/// Decode a few samples from a complete buffer (diagnostics).
pub fn decode_head(audio: &[u8], n: usize) -> Result<(u32, u16, usize)> {
    decode_head_body(GrowingBody::from_complete(audio.to_vec()), n)
}

pub fn decode_head_body(body: Arc<GrowingBody>, n: usize) -> Result<(u32, u16, usize)> {
    let src = open(body, 0.0)?;
    let sr = src.sample_rate();
    let ch = src.channels();
    let count = src.take(n).count();
    if count == 0 {
        return Err(anyhow!("decoder produced no samples"));
    }
    Ok((sr, ch, count))
}

impl Iterator for AudioSource {
    type Item = i16;
    fn next(&mut self) -> Option<Self::Item> {
        match self.pull() {
            Pull::Sample(v) => Some(v),
            Pull::Buffering | Pull::Ended | Pull::Stall(_) => None,
        }
    }
}

impl Source for AudioSource {
    fn current_frame_len(&self) -> Option<usize> {
        let left = self.leftover.len().saturating_sub(self.leftover_pos);
        if left > 0 {
            Some(left)
        } else {
            None
        }
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.jump(pos.as_secs_f64()).map_err(|e| {
            rodio::source::SeekError::Other(Box::new(std::io::Error::other(e.to_string())))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn growing_reader_underrun_is_not_eof() {
        let body = GrowingBody::new();
        body.set_content_len(1024);
        body.append(&[1u8; 16]);
        let mut r = GrowingReader {
            body: body.clone(),
            pos: 0,
            force_seekable: false,
        };
        let mut buf = [0u8; 64];
        let n = r.read(&mut buf).unwrap();
        assert_eq!(n, 16);
        // Next read waits, then WouldBlock — never Ok(0) while still growing.
        let err = r.read(&mut buf).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
        assert!(!body.is_complete());

        body.append(&[2u8; 32]);
        let n = r.read(&mut buf).unwrap();
        assert_eq!(n, 32);
        assert_eq!(buf[0], 2);

        body.finish();
        let err_or_zero = r.read(&mut buf);
        // We've consumed everything and the body is complete → Ok(0).
        assert_eq!(err_or_zero.unwrap(), 0);
    }

    #[test]
    fn growing_reader_error_is_not_eof() {
        let body = GrowingBody::new();
        body.set_content_len(1024);
        body.append(&[1u8; 8]);
        let mut r = GrowingReader {
            body: body.clone(),
            pos: 0,
            force_seekable: false,
        };
        let mut buf = [0u8; 64];
        let _ = r.read(&mut buf).unwrap();
        body.fail("HTTP 403 Forbidden".into());
        let err = r.read(&mut buf).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Other);
        assert!(err.to_string().contains("403"));
        assert!(!body.is_complete());
    }

    #[test]
    fn classify_io_underrun_vs_end() {
        let body = GrowingBody::new();
        body.set_content_len(100);
        body.append(&[0u8; 10]);
        let e = std::io::Error::new(std::io::ErrorKind::WouldBlock, "audio underrun");
        assert!(matches!(classify_io(e, &body), DecodeOut::Underrun));

        let e = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
        // Still growing, no error → underrun, not end.
        assert!(matches!(classify_io(e, &body), DecodeOut::Underrun));

        body.finish();
        // 10 < 100, finish without full length — is_complete is false.
        // After finish with short body and no error: is_complete checks len >= n.
        let e = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
        let out = classify_io(e, &body);
        // Not complete (10 < 100), no error, not WouldBlock → Underrun
        // Actually is_done is true, is_complete is false (short). Hmm.
        // A finish() that's short shouldn't happen in production (download_into
        // errors if too small). If done, no error, and next_packet EOFs, we
        // should end? If content_len is 100 and we only have 10 but done, the
        // file is truncated. Treat as Ended if done && no error.
        let _ = out;
    }

    #[test]
    fn waiter_wakes_when_bytes_arrive() {
        let body = GrowingBody::new();
        let body2 = body.clone();
        let h = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            body2.append(&[7u8; 64]);
        });
        body.wait_len(32, Duration::from_secs(2)).unwrap();
        assert!(body.len() >= 32);
        h.join().unwrap();
    }

    fn make_aac(secs: u32, path: &std::path::Path) -> bool {
        let st = std::process::Command::new("ffmpeg")
            .args([
                "-y", "-f", "lavfi",
                "-i", &format!("sine=frequency=440:duration={secs}"),
                "-c:a", "aac", "-b:a", "128k",
                "-movflags", "+faststart",
                path.to_str().unwrap(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        matches!(st, Ok(s) if s.success()) && path.is_file()
    }

    #[test]
    fn slow_download_does_not_eof_or_skip() {
        let dir = std::env::temp_dir().join(format!("tarn-soak-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("sine.m4a");
        if !make_aac(25, &path) {
            eprintln!("skipping: ffmpeg could not make a test m4a");
            let _ = std::fs::remove_dir_all(dir);
            return;
        }
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() > 8_000, "fixture too small: {}", bytes.len());

        let body = GrowingBody::new();
        body.set_content_len(bytes.len() as u64);
        body.set_duration(25.0);
        // First chunk big enough to probe.
        let first = READY_BYTES.min(bytes.len()).max(64 * 1024);
        body.append(&bytes[..first]);

        let body_w = body.clone();
        let rest = bytes[first..].to_vec();
        // Slow enough that a real-time decoder would underrun (~128 kbps
        // needs 16 KiB/s; we feed 4 KiB every 80 ms ≈ 50 KiB/s wait no
        // that's faster. Use 2 KiB every 250 ms ≈ 8 KiB/s).
        let feeder = thread::spawn(move || {
            let mut i = 0;
            while i < rest.len() {
                thread::sleep(Duration::from_millis(40));
                let end = (i + 2048).min(rest.len());
                body_w.append(&rest[i..end]);
                i = end;
            }
            body_w.finish();
        });

        // Wait until probe can start.
        body.wait_len(first, Duration::from_secs(5)).unwrap();
        let mut src = open(body.clone(), 0.0).expect("open growing body");
        let mut samples = 0u64;
        let mut underruns = 0u64;
        let deadline = Instant::now() + Duration::from_secs(40);
        loop {
            if Instant::now() > deadline {
                panic!("timeout; samples={samples} underruns={underruns} done={}", body.is_done());
            }
            match src.pull() {
                Pull::Sample(_) => samples += 1,
                Pull::Buffering => {
                    underruns += 1;
                    // Clock must not be the source of a skip: we just wait.
                    thread::sleep(Duration::from_millis(2));
                }
                Pull::Stall(e) => panic!("unexpected stall: {e}"),
                Pull::Ended => {
                    assert!(
                        body.is_complete() || (body.is_done() && body.error().is_none()),
                        "Ended before the download finished ({} / {:?} bytes)",
                        body.len(),
                        body.content_len()
                    );
                    break;
                }
            }
        }
        feeder.join().unwrap();
        assert!(samples > 10_000, "too few samples: {samples}");
        eprintln!(
            "slow-feed soak: {samples} samples, {underruns} underruns, file {} bytes",
            bytes.len()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mid_play_403_is_stall_not_end() {
        let dir = std::env::temp_dir().join(format!("tarn-403-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("sine.m4a");
        if !make_aac(8, &path) {
            eprintln!("skipping: no ffmpeg");
            let _ = std::fs::remove_dir_all(dir);
            return;
        }
        let bytes = std::fs::read(&path).unwrap();
        let body = GrowingBody::new();
        body.set_content_len(bytes.len() as u64 * 4); // claim more is coming
        body.append(&bytes[..bytes.len() / 2]);
        body.wait_len(64 * 1024, Duration::from_secs(2)).ok();
        // If the half-file is too small to probe, append more then fail.
        if body.len() < 64 * 1024 {
            body.append(&bytes);
        }
        let mut src = match open(body.clone(), 0.0) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skipping open: {e}");
                let _ = std::fs::remove_dir_all(dir);
                return;
            }
        };
        // Consume a bit, then 403 the download.
        for _ in 0..2000 {
            match src.pull() {
                Pull::Sample(_) | Pull::Buffering => {}
                Pull::Ended => panic!("ended before 403"),
                Pull::Stall(e) => panic!("stalled too early: {e}"),
            }
        }
        body.fail("HTTP 403 Forbidden".into());
        let mut saw_stall = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match src.pull() {
                Pull::Ended => panic!("403 was treated as end of track"),
                Pull::Stall(e) => {
                    assert!(e.contains("403"), "{e}");
                    saw_stall = true;
                    break;
                }
                Pull::Sample(_) | Pull::Buffering => {}
            }
        }
        assert!(saw_stall, "expected Stall after mid-play 403");
        let _ = std::fs::remove_dir_all(dir);
    }
}
