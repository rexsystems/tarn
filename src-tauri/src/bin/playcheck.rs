//! Diagnostic: resolve a stream, download audio, and decode it without
//! ffmpeg or a system yt-dlp. Used to verify self-contained playback.

use std::time::Instant;

use tarn_lib::{player, ytdlp, ytm};

#[tokio::main]
async fn main() {
    let id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "dQw4w9WgXcQ".into());

    println!("PATH={}", std::env::var("PATH").unwrap_or_default());
    println!(
        "system yt-dlp: {}",
        which("yt-dlp").unwrap_or_else(|| "not on PATH".into())
    );
    println!(
        "system ffmpeg: {}",
        which("ffmpeg").unwrap_or_else(|| "not on PATH".into())
    );

    let t0 = Instant::now();
    match ytdlp::ensure().await {
        Ok(p) => {
            println!("yt-dlp: {} ({})", p.display(), ytdlp::version(&p).unwrap_or("?".into()));
        }
        Err(e) => {
            println!("yt-dlp ensure: {e} (rustypipe-only fallback)");
        }
    }

    let y = ytm::Ytm::new().expect("ytm");
    match y.search_tracks("daft punk").await {
        Ok(tracks) => {
            println!("search: {} tracks (first: {:?})", tracks.len(), tracks.first().map(|t| &t.title));
        }
        Err(e) => {
            eprintln!("search failed: {e:#}");
            std::process::exit(1);
        }
    }
    let stream = y.stream(&id).await.unwrap_or_else(|e| {
        eprintln!("stream failed: {e:#}");
        std::process::exit(1);
    });
    println!(
        "stream: source={} mime={:?} dur={:?} ({} ms)",
        stream.source,
        stream.mime,
        stream.duration,
        t0.elapsed().as_millis()
    );

    let audio = y.fetch_audio(&stream).await.unwrap_or_else(|e| {
        eprintln!("download failed: {e:#}");
        std::process::exit(1);
    });
    println!("downloaded {} bytes in {} ms", audio.len(), t0.elapsed().as_millis());

    match player::decode_head(&audio, 48_000) {
        Ok((sr, ch, n)) => {
            println!("decoded {n} samples at {sr} Hz, {ch} ch in {} ms", t0.elapsed().as_millis());
            println!("OK: in-process search+playback pipeline works without system yt-dlp or ffmpeg");
        }
        Err(e) => {
            eprintln!("decode failed: {e:#}");
            std::process::exit(1);
        }
    }
}

fn which(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p.display().to_string());
        }
    }
    None
}
