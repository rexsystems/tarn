//! In-process audio decoder (symphonia). Handles YouTube's fragmented
//! AAC/m4a (ftyp `dash`) which makes rodio 0.20's wrapper panic on init.

use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

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

/// Seekable in-memory media source backed by a shared buffer.
struct Blob {
    data: Arc<Vec<u8>>,
    pos: u64,
}

impl Read for Blob {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let start = self.pos as usize;
        let data = self.data.as_slice();
        if start >= data.len() {
            return Ok(0);
        }
        let n = (data.len() - start).min(buf.len());
        buf[..n].copy_from_slice(&data[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Blob {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let len = self.data.len() as i128;
        let next = match from {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::End(n) => len + n as i128,
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

impl MediaSource for Blob {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.data.len() as u64)
    }
}

pub struct AudioSource {
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
    fn open(audio: Arc<Vec<u8>>) -> Result<Self> {
        let blob = Blob {
            data: audio,
            pos: 0,
        };
        let mss = MediaSourceStream::new(Box::new(blob), Default::default());
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

        // Decode the first packet so we know the signal spec up front.
        let (leftover, spec) = decode_next(&mut *format, &mut *decoder, track_id)?
            .ok_or_else(|| anyhow!("no audio packets"))?;
        let sample_rate = spec.rate;
        let channels = spec.channels.count() as u16;
        if sample_rate == 0 || channels == 0 {
            return Err(anyhow!("invalid audio format ({sample_rate} Hz, {channels} ch)"));
        }

        Ok(Self {
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
        // Pull one packet so playback starts immediately at the new position.
        if let Some((samples, spec)) = decode_next(&mut *self.format, &mut *self.decoder, self.track_id)?
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
            // Fragmented MP4 can surface this mid-stream; skip and keep going.
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

/// Open a downloaded audio file and optionally seek to `start` seconds.
pub fn open(
    audio: Arc<Vec<u8>>,
    start: f64,
    samples: Arc<AtomicU64>,
) -> Result<AudioSource> {
    let mut src = AudioSource::open(audio)?;
    src.samples = samples;
    src.jump(start)?;
    Ok(src)
}

/// Decode a few samples so callers can verify the pipeline without a device.
pub fn decode_head(audio: &[u8], n: usize) -> Result<(u32, u16, usize)> {
    let src = open(Arc::new(audio.to_vec()), 0.0, Arc::new(AtomicU64::new(0)))?;
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
