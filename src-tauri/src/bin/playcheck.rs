//! Diagnostic: resolve a stream, start progressive decode, and report
//! time-to-first-sound. Used to verify self-contained, near-instant play.

use std::time::Instant;

use tarn_lib::{decode, player, ytdlp, ytm};

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

    let t_boot = Instant::now();
    match ytdlp::ensure().await {
        Ok(p) => {
            println!(
                "yt-dlp: {} ({}) in {} ms (not on the play path)",
                p.display(),
                ytdlp::version(&p).unwrap_or("?".into()),
                t_boot.elapsed().as_millis()
            );
        }
        Err(e) => {
            println!("yt-dlp ensure: {e} (rustypipe-only fallback)");
        }
    }

    let y = ytm::Ytm::new().expect("ytm");
    match y.search_tracks("daft punk").await {
        Ok(tracks) => {
            println!(
                "search: {} tracks (first: {:?})",
                tracks.len(),
                tracks.first().map(|t| &t.title)
            );
        }
        Err(e) => {
            eprintln!("search failed: {e:#}");
            std::process::exit(1);
        }
    }

    // --- cold play (no cached URL) ---
    let cold = time_to_first_sound(&y, &id).await;
    println!(
        "COLD first-sound: {} ms (resolve {} ms, ready {} bytes, source={}, {} Hz {} ch, {} samples) target < 2000",
        cold.total_ms, cold.resolve_ms, cold.ready_bytes, cold.source, cold.sr, cold.ch, cold.samples
    );

    // Let the rest of the download finish so the warm hit uses a filled body.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // --- warm play (cached URL + already-fetched body) ---
    let warm = time_to_first_sound(&y, &id).await;
    println!(
        "WARM first-sound: {} ms (resolve {} ms, ready {} bytes, source={}, {} Hz {} ch, {} samples) target < 1000",
        warm.total_ms, warm.resolve_ms, warm.ready_bytes, warm.source, warm.sr, warm.ch, warm.samples
    );

    if cold.samples == 0 || warm.samples == 0 {
        eprintln!("decode produced no samples");
        std::process::exit(1);
    }
    println!("OK: in-process search+playback pipeline works without system yt-dlp or ffmpeg");
}

struct Timing {
    total_ms: u128,
    resolve_ms: u128,
    ready_bytes: usize,
    source: String,
    sr: u32,
    ch: u16,
    samples: usize,
}

async fn time_to_first_sound(y: &ytm::Ytm, id: &str) -> Timing {
    let t0 = Instant::now();
    let stream = y.stream(id).await.unwrap_or_else(|e| {
        eprintln!("stream failed: {e:#}");
        std::process::exit(1);
    });
    let resolve_ms = t0.elapsed().as_millis();
    let source = stream.source.to_string();
    let body = y.start_audio(id, &stream).await.unwrap_or_else(|e| {
        eprintln!("start_audio failed: {e:#}");
        std::process::exit(1);
    });
    let ready_bytes = body.len();
    // Decode the first second of audio — this is "first sound" for the
    // in-process player (rodio starts consuming as soon as open() returns).
    match decode::decode_head_body(body, 48_000) {
        Ok((sr, ch, samples)) => Timing {
            total_ms: t0.elapsed().as_millis(),
            resolve_ms,
            ready_bytes,
            source,
            sr,
            ch,
            samples,
        },
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

// Keep player import used so the crate still exports the diagnostic helper.
#[allow(dead_code)]
fn _player_decode_head_link() {
    let _ = player::decode_head as fn(&[u8], usize) -> anyhow::Result<(u32, u16, usize)>;
}
