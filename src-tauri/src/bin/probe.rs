//! Search YouTube Music for a query, resolve a playable stream, decode audio.
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use rustypipe::client::RustyPipe;

#[tokio::main]
async fn main() {
    let q = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "daft punk".to_string());
    match run(&q).await {
        Ok(()) => {}
        Err(e) => {
            eprintln!("probe failed: {e:#}");
            std::process::exit(1);
        }
    }
}

async fn run(q: &str) -> Result<()> {
    println!("query: {q}");
    let dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("tarn");
    std::fs::create_dir_all(&dir).ok();
    let rp = RustyPipe::builder()
        .storage_dir(&dir)
        .build()
        .map_err(|e| anyhow!("{e}"))?;

    let t0 = Instant::now();
    let tracks = rp
        .query()
        .music_search_tracks(q)
        .await
        .map_err(|e| anyhow!("rustypipe search: {e}"))?;
    let search_ms = t0.elapsed().as_millis();
    println!(
        "rustypipe search: {} tracks in {search_ms} ms",
        tracks.items.items.len()
    );
    for (i, t) in tracks.items.items.iter().take(8).enumerate() {
        let artists = t
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let album = t
            .album
            .as_ref()
            .map(|a| a.name.as_str())
            .unwrap_or("-");
        let dur = t.duration.unwrap_or(0);
        println!(
            "  {:2}. {}  —  {}  [{}]  {}s  id={}",
            i + 1,
            t.name,
            artists,
            album,
            dur,
            t.id
        );
    }
    let first = tracks
        .items
        .items
        .first()
        .ok_or_else(|| anyhow!("no search results"))?;
    let id = first.id.clone();
    println!("\nresolving stream for {} ({id})", first.name);

    let mut source = "none";
    let mut url = String::new();
    let mut mime = String::new();

    // yt-dlp first — rustypipe's player endpoint often trips a bot check.
    let t2 = Instant::now();
    let watch = format!("https://www.youtube.com/watch?v={id}");
    let out = Command::new("yt-dlp")
        .args([
            "--no-warnings",
            "--no-playlist",
            "-f",
            "bestaudio[ext=m4a]/bestaudio/best",
            "-j",
            "--",
            &watch,
        ])
        .output()
        .context("yt-dlp")?;
    if out.status.success() {
        let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
        if let Some(u) = v["url"].as_str() {
            url = u.to_string();
            mime = v["ext"].as_str().unwrap_or("").to_string();
            source = "yt-dlp";
            println!(
                "yt-dlp stream in {} ms  ext={mime}  acodec={}",
                t2.elapsed().as_millis(),
                v["acodec"].as_str().unwrap_or("?")
            );
        }
    } else {
        println!(
            "yt-dlp failed ({} ms): {}",
            t2.elapsed().as_millis(),
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("?")
        );
    }

    if url.is_empty() {
        let t1 = Instant::now();
        match rp.query().player(&id).await {
            Ok(p) => {
                println!(
                    "rustypipe player in {} ms  client={:?}  audio_streams={}  video_streams={}",
                    t1.elapsed().as_millis(),
                    p.client_type,
                    p.audio_streams.len(),
                    p.video_streams.len()
                );
                if let Some(s) = p
                    .audio_streams
                    .iter()
                    .filter(|s| s.drm_systems.is_empty() && !s.url.is_empty())
                    .max_by_key(|s| s.average_bitrate.max(s.bitrate))
                {
                    println!(
                        "  candidate itag={} br={} mime={}",
                        s.itag, s.bitrate, s.mime
                    );
                    if url_alive(&s.url).await {
                        url = s.url.clone();
                        mime = s.mime.clone();
                        source = "rustypipe";
                        println!("  rustypipe url is reachable");
                    } else {
                        println!("  rustypipe url not reachable (likely 403 / PO token)");
                    }
                } else {
                    println!("  no usable audio streams");
                }
            }
            Err(e) => println!("rustypipe player error ({:?}): {e}", t1.elapsed()),
        }
    }

    if url.is_empty() {
        return Err(anyhow!("could not resolve a playable stream"));
    }

        println!("source={source}  mime={mime}");
    println!("url={}...", url.chars().take(90).collect::<String>());

    // Decode ~2 seconds of audio with ffmpeg
    let t3 = Instant::now();
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-t",
            "2",
            "-i",
            &url,
            "-vn",
            "-ac",
            "2",
            "-ar",
            "44100",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().unwrap();
    let mut buf = Vec::new();
    stdout.read_to_end(&mut buf)?;
    let status = child.wait()?;
    if !status.success() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        return Err(anyhow!("ffmpeg decode failed: {err}"));
    }
    let samples = buf.len() / 4;
    let frames = samples / 2;
    let mut sumsq = 0.0f64;
    let mut peak = 0.0f32;
    for c in buf.chunks_exact(4) {
        let s = f32::from_le_bytes(c.try_into().unwrap());
        sumsq += (s as f64) * (s as f64);
        peak = peak.max(s.abs());
    }
    let rms = (sumsq / samples.max(1) as f64).sqrt();
    println!(
        "decoded {frames} frames ({} bytes) in {} ms  rms={rms:.4}  peak={peak:.4}",
        buf.len(),
        t3.elapsed().as_millis()
    );
    if frames < 1000 || rms < 1e-6 {
        return Err(anyhow!("decoded audio looks empty (frames={frames} rms={rms})"));
    }
    println!("ok");
    Ok(())
}

async fn url_alive(url: &str) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    match client
        .get(url)
        .header("Range", "bytes=0-1023")
        .header(
            "User-Agent",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36",
        )
        .send()
        .await
    {
        Ok(r) => r.status().as_u16() < 400,
        Err(_) => false,
    }
}
