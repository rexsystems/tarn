//! Search YouTube Music for a query, resolve a playable stream, decode audio.
use std::time::Instant;

use anyhow::{anyhow, Result};
use rustypipe::client::RustyPipe;
use tarn_lib::{player, ytm};

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
        let album = t.album.as_ref().map(|a| a.name.as_str()).unwrap_or("-");
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

    let y = ytm::Ytm::new()?;
    let t2 = Instant::now();
    let stream = y
        .stream(&id)
        .await
        .map_err(|e| anyhow!("stream: {e}"))?;
    println!(
        "stream from {} in {} ms  mime={:?}  dur={:?}",
        stream.source,
        t2.elapsed().as_millis(),
        stream.mime,
        stream.duration
    );
    let audio = y.fetch_audio(&stream).await?;
    let t3 = Instant::now();
    let (sr, ch, n) = player::decode_head(&audio, 44100 * 2 * 2)?;
    println!(
        "decoded {n} samples at {sr} Hz, {ch} ch in {} ms ({} bytes)",
        t3.elapsed().as_millis(),
        audio.len()
    );
    if n < 1000 {
        return Err(anyhow!("decoded audio looks empty (samples={n})"));
    }
    println!("ok");
    Ok(())
}
