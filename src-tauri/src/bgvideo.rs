//! Fullscreen video background: the music video of the song that is playing.
//!
//! The video track (no audio, at most 720p) is downloaded once into the
//! cache and served to the webview over the `tarnvideo` scheme with Range
//! support, so a muted `<video>` can follow the audio and seek freely.
//! googlevideo URLs only allow one closed-range request without a PO token,
//! so streaming straight from YouTube into `<video>` would stop after ~1 MiB.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use tokio::io::AsyncWriteExt;

use crate::player_client::VideoSrc;
use crate::ytm::Ytm;

/// Where `primary_ms` of the song is `counterpart_ms` in the video, for
/// `duration_ms`.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct VideoSeg {
    pub primary_ms: f64,
    pub counterpart_ms: f64,
    pub duration_ms: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct BgVideo {
    /// File name in the video cache, served as `tarnvideo://localhost/<file>`.
    pub file: String,
    pub video_id: String,
    /// Song time -> video time. Empty when the song is the video itself.
    pub segments: Vec<VideoSeg>,
}

/// Skip anything bigger (very long videos).
const MAX_BYTES: u64 = 160 * 1024 * 1024;
/// Keep the cache around this size / count.
const KEEP_BYTES: u64 = 600 * 1024 * 1024;
const KEEP_FILES: usize = 24;
/// Largest slice returned for an open-ended Range request.
const SLICE: u64 = 4 * 1024 * 1024;

type Pair = Option<(String, Vec<VideoSeg>)>;

fn memo() -> &'static Mutex<HashMap<String, Pair>> {
    static M: OnceLock<Mutex<HashMap<String, Pair>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

/// One download at a time; a second request for the same video waits and
/// then finds the file.
static DOWNLOAD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn dir() -> PathBuf {
    dirs::cache_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".cache")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tarn")
        .join("videos")
}

fn existing(dir: &Path, vid: &str) -> Option<String> {
    ["mp4", "webm"].into_iter().find_map(|ext| {
        let f = format!("{vid}.{ext}");
        dir.join(&f).is_file().then_some(f)
    })
}

fn touch(path: &Path) {
    if let Ok(f) = std::fs::File::options().write(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// The music video to show behind `track_id`, downloading it if needed.
/// `Ok(None)`: this song has no video.
pub async fn resolve(ytm: &Ytm, track_id: &str, is_video: bool) -> Result<Option<BgVideo>> {
    let (vid, segments) = if is_video {
        (track_id.to_string(), Vec::new())
    } else {
        let known = memo().lock().unwrap().get(track_id).cloned();
        let pair = match known {
            Some(p) => p,
            None => {
                let p = ytm.counterpart(track_id).await?;
                memo().lock().unwrap().insert(track_id.to_string(), p.clone());
                p
            }
        };
        match pair {
            Some(p) => p,
            None => return Ok(None),
        }
    };
    let dir = dir();
    std::fs::create_dir_all(&dir).ok();
    let found = |dir: &Path| {
        existing(dir, &vid).map(|file| {
            touch(&dir.join(&file));
            BgVideo { file, video_id: vid.clone(), segments: segments.clone() }
        })
    };
    if let Some(v) = found(&dir) {
        return Ok(Some(v));
    }
    if ytm.is_offline() {
        return Ok(None);
    }
    let _guard = DOWNLOAD.lock().await;
    if let Some(v) = found(&dir) {
        return Ok(Some(v));
    }
    let (src, kind) = ytm.player_hub().video(&vid).await?;
    if src.len.is_some_and(|n| n > MAX_BYTES) {
        return Ok(None);
    }
    let ext = if src.mime.contains("webm") { "webm" } else { "mp4" };
    let file = format!("{vid}.{ext}");
    download(&src, kind.ua(), &dir.join(&file)).await?;
    crate::diag::log(format!("bg video {vid}: {}p {} saved", src.height, src.mime));
    prune(&dir, &file);
    Ok(Some(BgVideo { file, video_id: vid, segments }))
}

async fn download(src: &VideoSrc, ua: &str, to: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()?;
    let mut req = client
        .get(&src.url)
        .header("User-Agent", ua)
        .header("Accept", "*/*");
    // A single closed range is what googlevideo allows without a PO token.
    if let Some(n) = src.len {
        req = req.header("Range", format!("bytes=0-{}", n.saturating_sub(1)));
    }
    let mut resp = req.send().await.context("video download")?;
    if !resp.status().is_success() {
        return Err(anyhow!("video download: HTTP {}", resp.status()));
    }
    let part = to.with_extension("part");
    let mut f = tokio::fs::File::create(&part).await?;
    let mut got = 0u64;
    let res: Result<()> = async {
        while let Some(chunk) = resp.chunk().await.context("video body")? {
            got += chunk.len() as u64;
            if got > MAX_BYTES {
                return Err(anyhow!("video too large"));
            }
            f.write_all(&chunk).await?;
        }
        f.flush().await?;
        if src.len.is_some_and(|n| got < n) || got < 64 * 1024 {
            return Err(anyhow!("video download: short ({got} bytes)"));
        }
        Ok(())
    }
    .await;
    drop(f);
    if let Err(e) = res {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(e);
    }
    tokio::fs::rename(&part, to).await?;
    Ok(())
}

/// Drop the least recently shown videos beyond the size / count budget.
fn prune(dir: &Path, keep: &str) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| {
                (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len(), e.path())
            })
        })
        .collect();
    // Newest first.
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut total = 0u64;
    for (i, (_, len, path)) in files.iter().enumerate() {
        total += len;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let stale_part = name.ends_with(".part");
        if name != keep && (stale_part || i >= KEEP_FILES || total > KEEP_BYTES) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Size of the video cache in bytes.
pub fn cache_bytes() -> u64 {
    std::fs::read_dir(dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

pub fn clear_cache() {
    let _ = std::fs::remove_dir_all(dir());
}

fn valid_name(name: &str) -> bool {
    let Some((id, ext)) = name.split_once('.') else { return false };
    id.len() == 11
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && (ext == "mp4" || ext == "webm")
}

/// Parse `bytes=a-b`, `bytes=a-` or `bytes=-n` against a file of `len` bytes.
fn parse_range(h: &str, len: u64) -> Option<(u64, u64)> {
    let spec = h.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    if len == 0 {
        return None;
    }
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (len.saturating_sub(n), len - 1)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() {
            (start + SLICE - 1).min(len - 1)
        } else {
            b.parse::<u64>().ok()?.min(len - 1)
        };
        (start, end)
    };
    (start <= end && start < len).then_some((start, end))
}

/// Handler for the `tarnvideo` scheme.
pub fn serve(req: &tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    use tauri::http::Response;

    let empty = |status: u16| Response::builder().status(status).body(Vec::new()).unwrap();
    let name = req.uri().path().trim_start_matches('/');
    if !valid_name(name) {
        return empty(404);
    }
    let path = dir().join(name);
    let Ok(mut f) = std::fs::File::open(&path) else { return empty(404) };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mime = if name.ends_with(".webm") { "video/webm" } else { "video/mp4" };
    let range = req
        .headers()
        .get("range")
        .and_then(|v| v.to_str().ok())
        .map(|h| parse_range(h, len));
    let (status, start, end) = match range {
        Some(Some((a, b))) => (206, a, b),
        Some(None) => {
            return Response::builder()
                .status(416)
                .header("Content-Range", format!("bytes */{len}"))
                .body(Vec::new())
                .unwrap()
        }
        None if len == 0 => return empty(404),
        None => (200, 0, len - 1),
    };
    let mut buf = vec![0u8; (end - start + 1) as usize];
    if f.seek(SeekFrom::Start(start)).is_err() || f.read_exact(&mut buf).is_err() {
        return empty(500);
    }
    let mut b = Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Accept-Ranges", "bytes")
        .header("Content-Length", buf.len().to_string())
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-cache");
    if status == 206 {
        b = b.header("Content-Range", format!("bytes {start}-{end}/{len}"));
    }
    b.body(buf).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=0-5000", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=0-", 10 * SLICE), Some((0, SLICE - 1)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("items=0-1", 1000), None);
    }

    #[test]
    fn names() {
        assert!(valid_name("dQw4w9WgXcQ.mp4"));
        assert!(valid_name("a-b_c-d_e-f.webm"));
        assert!(!valid_name("../etc/passwd"));
        assert!(!valid_name("dQw4w9WgXcQ.exe"));
        assert!(!valid_name("short.mp4"));
    }
}
