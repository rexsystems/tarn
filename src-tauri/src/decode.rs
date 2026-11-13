//! In-process audio decoder (symphonia). Handles YouTube's fragmented
//! AAC/m4a (ftyp `dash`) which makes rodio 0.20's wrapper panic on init.
//!
//! The media source is a growing buffer: playback can start as soon as the
//! first fragments arrive, while the rest of the file downloads in the
//! background. Reads block (with a timeout) if the decoder runs ahead.

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

/// Shared, append-only audio body filled by an HTTP download task.
pub struct GrowingBody {
    data: Mutex<Vec<u8>>,
    more: Condvar,
    content_len: AtomicU64,
    done: AtomicBool,
    error: Mutex<Option<String>>,
}

impl GrowingBody {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            data: Mutex::new(Vec::new()),
            more: Condvar::new(),
            content_len: AtomicU64::new(0),
            done: AtomicBool::new(false),
            error: Mutex::new(None),
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
        self.more.notify_all();
    }

    pub fn set_content_len(&self, n: u64) {
        self.content_len.store(n, Ordering::Relaxed);
    }

    pub fn finish(&self) {
        self.done.store(true, Ordering::SeqCst);
        self.more.notify_all();
    }

    pub fn fail(&self, msg: String) {
        *self.error.lock().unwrap() = Some(msg);
        self.done.store(true, Ordering::SeqCst);
        self.more.notify_all();
    }

    pub fn len(&self) -> usize {
        self.data.lock().unwrap().len()
    }

    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.data.lock().unwrap().clone()
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
}

impl Read for GrowingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut g = self.body.data.lock().unwrap();
        loop {
            let avail = g.len() as u64;
            if self.pos < avail {
                let start = self.pos as usize;
                let n = (g.len() - start).min(buf.len());
                buf[..n].copy_from_slice(&g[start..start + n]);
                self.pos += n as u64;
                return Ok(n);
            }
            if self.body.done.load(Ordering::SeqCst) {
                if let Some(e) = self.body.error.lock().unwrap().as_ref() {
                    return Err(std::io::Error::other(e.clone()));
                }
                return Ok(0);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "audio stalled",
                ));
            }
            let (gg, t) = self.body.more.wait_timeout(g, left).unwrap();
            g = gg;
            if t.timed_out() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "audio stalled",
                ));
            }
        }
    }
}

impl Seek for GrowingReader {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        // While the body is still growing, treat "end" as what we already
        // have — seeking to a future tail would block until the full file
        // arrives and defeat progressive start.
        let downloaded = self.body.data.lock().unwrap().len() as u64;
        let end = downloaded;
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
        Ok(self.pos)
    }
}

impl MediaSource for GrowingReader {
    fn is_seekable(&self) -> bool {
        // The ISO-MP4 demuxer refuses to open a "seekable" source without a
        // known length, and with a known length it scans to the end of the
        // file. Report unseekable until the download is finished so playback
        // can start after moov+sidx+the first mdat (YouTube puts those first).
        self.body.done.load(Ordering::SeqCst)
    }
    fn byte_len(&self) -> Option<u64> {
        if self.body.done.load(Ordering::SeqCst) {
            Some(self.body.len() as u64)
        } else {
            None
        }
    }
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
    samples: Arc<AtomicU64>,
}

impl AudioSource {
    fn open(body: Arc<GrowingBody>) -> Result<Self> {
        let reader = GrowingReader { body: body.clone(), pos: 0 };
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

        let (leftover, spec) = decode_next(&mut *format, &mut *decoder, track_id)?
            .ok_or_else(|| anyhow!("no audio packets"))?;
        let sample_rate = spec.rate;
        let channels = spec.channels.count() as u16;
        if sample_rate == 0 || channels == 0 {
            return Err(anyhow!(
                "invalid audio format ({sample_rate} Hz, {channels} ch)"
            ));
        }

        Ok(Self {
            body,
            decoder,
            format,
            track_id,
            leftover,
            leftover_pos: 0,
            sample_rate,
            channels,
            samples: Arc::new(AtomicU64::new(0)),
        })
    }

    fn jump(&mut self, start: f64) -> Result<()> {
        if start <= 0.05 {
            return Ok(());
        }
        // If we opened while the download was still going, the demuxer is
        // unseekable. Wait for the rest (usually already nearly done by the
        // time the user seeks) and reopen as a seekable source.
        if !self.body.is_done() {
            let _ = self.body.wait_len(usize::MAX, Duration::from_secs(20));
        }
        if self.body.is_done() {
            let samples = self.samples.clone();
            let mut fresh = AudioSource::open(self.body.clone())?;
            fresh.samples = samples;
            *self = fresh;
        }
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
        if let Some((samples, spec)) =
            decode_next(&mut *self.format, &mut *self.decoder, self.track_id)?
        {
            self.leftover = samples;
            self.leftover_pos = 0;
            self.sample_rate = spec.rate;
            self.channels = spec.channels.count() as u16;
        }
        Ok(())
    }

    fn fill(&mut self) -> bool {
        match decode_next(&mut *self.format, &mut *self.decoder, self.track_id) {
            Ok(Some((samples, spec))) => {
                self.leftover = samples;
                self.leftover_pos = 0;
                self.sample_rate = spec.rate;
                self.channels = spec.channels.count() as u16;
                true
            }
            _ => false,
        }
    }
}

fn decode_next(
    format: &mut dyn FormatReader,
    decoder: &mut dyn Decoder,
    track_id: u32,
) -> Result<Option<(Vec<i16>, SignalSpec)>> {
    let mut decode_errors = 0usize;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(_)) => return Ok(None),
            Err(SymError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(SymError::SeekError(_)) => continue,
            Err(e) => return Err(anyhow!("packet: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let mut buf = SampleBuffer::<i16>::new(decoded.capacity() as u64, spec);
                buf.copy_interleaved_ref(decoded);
                return Ok(Some((buf.samples().to_vec(), spec)));
            }
            Err(SymError::DecodeError(_)) => {
                decode_errors += 1;
                if decode_errors > 8 {
                    return Err(anyhow!("too many decode errors"));
                }
                continue;
            }
            Err(SymError::IoError(_)) => return Ok(None),
            Err(e) => return Err(anyhow!("decode: {e}")),
        }
    }
}

/// Open a (possibly still-growing) audio body and optionally seek to `start`.
pub fn open(
    audio: Arc<GrowingBody>,
    start: f64,
    samples: Arc<AtomicU64>,
) -> Result<AudioSource> {
    let mut src = AudioSource::open(audio)?;
    src.samples = samples;
    src.jump(start)?;
    Ok(src)
}

/// Decode a few samples from a complete buffer (diagnostics).
pub fn decode_head(audio: &[u8], n: usize) -> Result<(u32, u16, usize)> {
    decode_head_body(GrowingBody::from_complete(audio.to_vec()), n)
}

pub fn decode_head_body(body: Arc<GrowingBody>, n: usize) -> Result<(u32, u16, usize)> {
    let src = open(body, 0.0, Arc::new(AtomicU64::new(0)))?;
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
        loop {
            if self.leftover_pos < self.leftover.len() {
                let v = self.leftover[self.leftover_pos];
                self.leftover_pos += 1;
                self.samples.fetch_add(1, Ordering::Relaxed);
                return Some(v);
            }
            if !self.fill() {
                return None;
            }
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
