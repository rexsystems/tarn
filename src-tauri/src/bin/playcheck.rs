//! Diagnostic: resolve a stream, start progressive decode, and report
//! time-to-first-sound. Used to verify self-contained, near-instant play.
//!
//!   playcheck [video_id]
//!   playcheck --batch

use std::time::Instant;

use tarn_lib::{decode, player, ytdlp, ytm};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let batch = args.iter().any(|a| a == "--batch");
    let id = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .cloned()
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

    if batch {
        run_batch(&y).await;
        return;
    }

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
    let cold = time_to_first_sound(&y, &id)
        .await
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        });
    println!(
        "COLD first-sound: {} ms (resolve {} ms, ready {} bytes, source={}, {} Hz {} ch, {} samples) target < 2000",
        cold.total_ms, cold.resolve_ms, cold.ready_bytes, cold.source, cold.sr, cold.ch, cold.samples
    );

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let warm = time_to_first_sound(&y, &id)
        .await
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        });
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

async fn run_batch(y: &ytm::Ytm) {
    // Cold cache: no leftover URLs or audio files from earlier runs.
    y.clear_play_cache();
    println!("cleared play cache");

    let queries = [
        "Man I Need Olivia Dean",
        "Instant Crush Daft Punk",
        "Blinding Lights The Weeknd",
        "Espresso Sabrina Carpenter",
        "Get Lucky Daft Punk",
        "Anti-Hero Taylor Swift",
        "Good 4 U Olivia Rodrigo",
        "Not Like Us Kendrick Lamar",
        "About Damn Time Lizzo",
        "As It Was Harry Styles",
        "Birds of a Feather Billie Eilish",
        "Die With A Smile Lady Gaga Bruno Mars",
    ];

    let mut ok = 0u32;
    let mut fail = 0u32;
    for q in queries {
        let t0 = Instant::now();
        let tracks = match y.search_tracks(q).await {
            Ok(t) if !t.is_empty() => t,
            Ok(_) => {
                println!("FAIL  {q}: search returned no tracks");
                fail += 1;
                continue;
            }
            Err(e) => {
                println!("FAIL  {q}: search: {e}");
                fail += 1;
                continue;
            }
        };
        let t = &tracks[0];
        // Each song is first-play from a cold URL cache (disk may hold
        // earlier songs in the same run, which is fine).
        y.drop_url_cache();
        match time_to_first_sound(y, &t.id).await {
            Ok(r) => {
                ok += 1;
                println!(
                    "OK    {q} → {} — {}  {} ms (resolve {} ms, {}, {} bytes, {} Hz {} ch)",
                    t.id,
                    t.title,
                    r.total_ms,
                    r.resolve_ms,
                    r.source,
                    r.ready_bytes,
                    r.sr,
                    r.ch
                );
            }
            Err(e) => {
                fail += 1;
                println!(
                    "FAIL  {q} → {} — {}  after {} ms: {e}",
                    t.id,
                    t.title,
                    t0.elapsed().as_millis()
                );
            }
        }
    }
    println!("BATCH {ok} ok, {fail} failed, {} total", ok + fail);
    if fail > 0 {
        std::process::exit(1);
    }
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

async fn time_to_first_sound(y: &ytm::Ytm, id: &str) -> anyhow::Result<Timing> {
    let t0 = Instant::now();
    let stream = y
        .stream(id)
        .await
        .map_err(|e| anyhow::anyhow!("stream failed: {e:#}"))?;
    let resolve_ms = t0.elapsed().as_millis();
    let source = stream.label();
    let body = y
        .start_audio(id, &stream)
        .await
        .map_err(|e| anyhow::anyhow!("start_audio failed: {e:#}"))?;
    let ready_bytes = body.len();
    match decode::decode_head_body(body, 48_000) {
        Ok((sr, ch, samples)) => Ok(Timing {
            total_ms: t0.elapsed().as_millis(),
            resolve_ms,
            ready_bytes,
            source,
            sr,
            ch,
            samples,
        }),
        Err(e) => Err(anyhow::anyhow!("decode failed: {e:#}")),
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

#[allow(dead_code)]
fn _player_decode_head_link() {
    let _ = player::decode_head as fn(&[u8], usize) -> anyhow::Result<(u32, u16, usize)>;
}
