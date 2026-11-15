//! Probe Home shelves: load session, list playlist/album cards, try fetch + first stream.
//!   cargo run --bin homeplay

use anyhow::{anyhow, Result};
use tarn_lib::auth::SessionStore;
use tarn_lib::ytm::Ytm;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("homeplay failed: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let y = Ytm::new()?;
    let store = SessionStore::new()?;
    match store.load() {
        Ok(Some((header, kind))) => {
            let names = tarn_lib::auth::header_cookie_names(&header);
            println!("session: {kind:?} cookies={names:?}");
            y.set_cookie_header(Some(header)).await?;
        }
        Ok(None) => println!("session: none (anonymous)"),
        Err(e) => println!("session: load err {e}"),
    }

    let home = y.home(None).await.map_err(|e| anyhow!("home: {e}"))?;
    println!(
        "home: signed_in={} shelves={}",
        home.signed_in,
        home.shelves.len()
    );

    let mut fail = 0u32;
    let mut ok = 0u32;
    for shelf in &home.shelves {
        let interesting: Vec<_> = shelf
            .items
            .iter()
            .filter(|i| matches!(i.kind.as_str(), "playlist" | "album" | "artist"))
            .collect();
        if interesting.is_empty() {
            continue;
        }
        println!(
            "\n## {} (kind={}, {} cards)",
            shelf.title,
            shelf.kind,
            interesting.len()
        );
        for item in interesting.into_iter().take(8) {
            print!(
                "  - [{}] {} … ",
                item.kind, item.title
            );
            let res = match item.kind.as_str() {
                "playlist" => y.playlist(&item.id).await.map(|c| (c.name, c.tracks.len(), c.tracks.first().map(|t| t.id.clone()))),
                "album" => {
                    let id = item.browse_id.as_deref().unwrap_or(&item.id);
                    y.album(id).await.map(|c| (c.name, c.tracks.len(), c.tracks.first().map(|t| t.id.clone())))
                }
                "artist" => {
                    let id = item.browse_id.as_deref().unwrap_or(&item.id);
                    y.artist(id).await.map(|a| (a.name, a.tracks.len(), a.tracks.first().map(|t| t.id.clone())))
                }
                _ => continue,
            };
            match res {
                Ok((name, n, first)) => {
                    if n == 0 {
                        fail += 1;
                        println!("EMPTY");
                    } else {
                        ok += 1;
                        println!("ok tracks={n} ({name})");
                        if let Some(id) = first {
                            if let Err(e) = y.stream(&id).await {
                                println!("      stream WARN: {e}");
                            }
                        }
                    }
                }
                Err(e) => {
                    fail += 1;
                    println!("FAIL {e}");
                }
            }
        }
    }
    println!("\nsummary: ok={ok} fail={fail}");
    if fail > 0 {
        anyhow::bail!("{fail} playable cards still broken");
    }
    Ok(())
}
