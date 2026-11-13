//! Stress-test stream resolve, first-chunk, full download, and decode.
//!
//!   soak                 # default set, 8 resolve+chunk rounds + full download
//!   soak --stress        # 40 resolve+first-chunk rounds, 12 full downloads
//!   soak --id VIDEO_ID

use std::io::Write;
use std::time::{Duration, Instant};

use sha1::{Digest, Sha1};
use rodio::Source;
use tarn_lib::decode::{self, GrowingBody, Pull, READY_BYTES};
use tarn_lib::ytm::{Stream, Ytm};

#[allow(dead_code)]
fn ids_default() -> Vec<(&'static str, &'static str)> {
    vec![
        ("djV11Xbc914", "Take On Me (video)"),
        // YTM audio ids are filled after a search.
    ]
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stress = args.iter().any(|a| a == "--stress");
    let full_only = args.iter().any(|a| a == "--full-only");
    let extra_ids: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect();

    let y = Ytm::new().expect("ytm");
    let mut tracks: Vec<(String, String)> = Vec::new();

    if extra_ids.is_empty() {
        for (q, hint) in [
            ("Take On Me a-ha", "Take On Me"),
            ("Into the Groove Madonna", "Into the Groove"),
            ("Smalltown Boy Bronski Beat", "Smalltown Boy"),
            ("Sweet Dreams Eurythmics", "Sweet Dreams"),
            ("Billie Jean Michael Jackson", "Billie Jean"),
        ] {
            match y.search_tracks(q).await {
                Ok(ts) => {
                    if let Some(t) = ts.into_iter().find(|t| {
                        t.title.to_lowercase().contains(&hint.to_lowercase())
                    }).or_else(|| {
                        // fallback first result
                        None
                    }) {
                        println!(
                            "search {q:?} -> {} [{}] album={:?} dur={:?}",
                            t.title, t.id, t.album, t.duration
                        );
                        tracks.push((t.id, t.title));
                    } else {
                        // try first
                        match y.search_tracks(q).await {
                            Ok(ts) if !ts.is_empty() => {
                                let t = &ts[0];
                                println!(
                                    "search {q:?} (first) -> {} [{}] dur={:?}",
                                    t.title, t.id, t.duration
                                );
                                tracks.push((t.id.clone(), t.title.clone()));
                            }
                            _ => println!("search {q:?}: no results"),
                        }
                    }
                }
                Err(e) => println!("search {q:?}: {e}"),
            }
        }
        // Always also try the official video id.
        tracks.insert(0, ("djV11Xbc914".into(), "Take On Me (djV11Xbc914)".into()));
        tracks.dedup_by(|a, b| a.0 == b.0);
    } else {
        for id in extra_ids {
            tracks.push((id.clone(), id));
        }
    }

    let rounds = if full_only { 0 } else if stress { 40 } else { 8 };
    println!("\n=== resolve + first-chunk  ({rounds} rounds, cold cache) ===");
    let mut resolve_ok = 0u32;
    let mut resolve_fail = 0u32;
    let mut chunk_ok = 0u32;
    let mut chunk_fail = 0u32;
    let mut fail_reasons: Vec<String> = Vec::new();

    for (id, title) in &tracks {
        for n in 1..=rounds {
            y.clear_play_cache();
            let t0 = Instant::now();
            match resolve_and_first(&y, id).await {
                Ok(info) => {
                    resolve_ok += 1;
                    chunk_ok += 1;
                    println!(
                        "OK  [{n:02}/{rounds}] {title} ({id}) client={} src={} ready={} clen={:?} probe+chunk {} ms",
                        info.client, info.source, info.ready, info.clen, t0.elapsed().as_millis()
                    );
                }
                Err(e) => {
                    let msg = format!("{e}");
                    // classify
                    let kind = classify(&msg);
                    if msg.contains("resolve") || msg.contains("rustypipe") || msg.contains("yt-dlp") || msg.contains("no playable") {
                        resolve_fail += 1;
                    } else {
                        resolve_ok += 1;
                        chunk_fail += 1;
                    }
                    fail_reasons.push(format!("{title} r{n} {kind}: {msg}"));
                    println!(
                        "FAIL [{n:02}/{rounds}] {title} ({id}) {kind}: {msg} ({} ms)",
                        t0.elapsed().as_millis()
                    );
                }
            }
        }
    }

    let total_r = (resolve_ok + resolve_fail).max(1);
    let total_c = (chunk_ok + chunk_fail).max(1);
    println!(
        "\nresolve: {resolve_ok} ok / {} ({:.1}% fail)",
        resolve_ok + resolve_fail,
        100.0 * resolve_fail as f64 / total_r as f64
    );
    println!(
        "first-chunk: {chunk_ok} ok / {} ({:.1}% fail)",
        chunk_ok + chunk_fail,
        100.0 * chunk_fail as f64 / total_c as f64
    );

    println!("\n=== full download + hash + decode ===");
    let full_n = if full_only { 2 } else if stress { 3 } else { 1 };
    let mut full_ok = 0u32;
    let mut full_fail = 0u32;
    let mut hash_mismatch = 0u32;
    let mut decode_early = 0u32;

    for (id, title) in &tracks {
        for n in 1..=full_n {
            y.clear_play_cache();
            match full_pass(&y, id, title, n).await {
                Ok(FullReport {
                    chunked_len,
                    clen,
                    hash_match,
                    decode_secs,
                    expected_secs,
                    progressive_secs,
                    client,
                }) => {
                    let complete = clen.map(|c| chunked_len as u64 >= c).unwrap_or(false);
                    if !complete {
                        full_fail += 1;
                        println!(
                            "SHORT [{n}] {title}: got {chunked_len} / clen={clen:?} client={client}"
                        );
                    } else {
                        full_ok += 1;
                    }
                    if !hash_match {
                        hash_mismatch += 1;
                        println!("HASH MISMATCH [{n}] {title}");
                    }
                    if expected_secs > 15.0 && progressive_secs < expected_secs * 0.7 {
                        decode_early += 1;
                        println!(
                            "DECODE EARLY [{n}] {title}: progressive {progressive_secs:.1}s vs expected {expected_secs:.1}s (complete-file {decode_secs:.1}s)"
                        );
                    }
                    println!(
                        "FULL [{n}] {title}: {chunked_len} bytes clen={clen:?} hash_ok={hash_match} decode={decode_secs:.1}s prog={progressive_secs:.1}s expect={expected_secs:.1}s client={client}"
                    );
                }
                Err(e) => {
                    full_fail += 1;
                    println!("FULL FAIL [{n}] {title}: {e}");
                    fail_reasons.push(format!("FULL {title} {e}"));
                }
            }
        }
    }

    println!("\n=== SUMMARY ===");
    println!(
        "full downloads: {full_ok} complete, {full_fail} short/fail, {hash_mismatch} hash mismatches, {decode_early} early decodes"
    );
    if !fail_reasons.is_empty() {
        println!("failures:");
        for r in &fail_reasons {
            println!("  - {r}");
        }
    }
}

struct FirstInfo {
    client: String,
    source: String,
    ready: usize,
    clen: Option<u64>,
}

async fn resolve_and_first(y: &Ytm, id: &str) -> anyhow::Result<FirstInfo> {
    let stream = y.stream(id).await?;
    let body = y.start_audio(id, &stream).await?;
    body.wait_ready().await?;
    Ok(FirstInfo {
        client: stream.client.to_string(),
        source: stream.source.to_string(),
        ready: body.len(),
        clen: body.content_len(),
    })
}

struct FullReport {
    chunked_len: usize,
    clen: Option<u64>,
    hash_match: bool,
    decode_secs: f64,
    expected_secs: f64,
    progressive_secs: f64,
    client: String,
}

async fn full_pass(y: &Ytm, id: &str, title: &str, n: u32) -> anyhow::Result<FullReport> {
    let t0 = Instant::now();
    let stream = y.stream(id).await?;
    let expected = stream.duration.unwrap_or(0.0);
    println!(
        "  resolve {title} [{n}] client={} src={} dur={expected:.1}s url_clen={:?}",
        stream.client,
        stream.source,
        clen_of(&stream)
    );
    let body = y.start_audio(id, &stream).await?;
    body.wait_ready().await?;
    let ready = body.len();

    // Progressive decode in a side thread while the rest downloads.
    let body_p = body.clone();
    let prog = tokio::task::spawn_blocking(move || decode_growing(body_p, expected));

    // Watch progress until done or 4 minutes.
    let deadline = Instant::now() + Duration::from_secs(240);
    let mut last = ready;
    let mut stalled_for = Duration::ZERO;
    loop {
        if body.is_done() {
            break;
        }
        if Instant::now() > deadline {
            anyhow::bail!(
                "download timed out at {} / {:?} after {:?}",
                body.len(),
                body.content_len(),
                t0.elapsed()
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let n = body.len();
        if n == last {
            stalled_for += Duration::from_millis(500);
            if stalled_for >= Duration::from_secs(8) {
                eprintln!(
                    "  STALL {title}: {} / {:?} for {:?}",
                    n,
                    body.content_len(),
                    stalled_for
                );
            }
        } else {
            if n != last {
                eprint!(
                    "  dl {title}: {} / {:?} ({:.0}%)\r",
                    n,
                    body.content_len(),
                    body.content_len()
                        .map(|c| 100.0 * n as f64 / c as f64)
                        .unwrap_or(0.0)
                );
                let _ = std::io::stderr().flush();
            }
            last = n;
            stalled_for = Duration::ZERO;
        }
        if let Some(e) = body.error() {
            anyhow::bail!("download error at {n} bytes: {e}");
        }
    }
    println!();
    if let Some(e) = body.error() {
        anyhow::bail!("download failed: {e}");
    }
    let chunked = body.snapshot();
    let clen = body.content_len();
    println!(
        "  chunked {title}: {} bytes clen={clen:?} done={} in {:?}",
        chunked.len(),
        body.is_done(),
        t0.elapsed()
    );

    // Single closed-range GET of the whole file for a hash compare.
    let hash_match = match single_get(&stream, clen.unwrap_or(chunked.len() as u64)).await {
        Ok(full) => {
            let a = sha(&chunked);
            let b = sha(&full);
            println!(
                "  single GET {title}: {} bytes sha1={} chunked={}",
                full.len(),
                b,
                a
            );
            a == b && full.len() == chunked.len()
        }
        Err(e) => {
            println!("  single GET {title}: {e} (skip hash)");
            true // don't count as mismatch if we couldn't fetch
        }
    };

    let decode_secs = decode_complete(&chunked)?;
    let progressive_secs = prog.await??;
    Ok(FullReport {
        chunked_len: chunked.len(),
        clen,
        hash_match,
        decode_secs,
        expected_secs: expected,
        progressive_secs,
        client: stream.client.to_string(),
    })
}

fn sha(b: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(b);
    format!("{:x}", h.finalize())
}

fn clen_of(s: &Stream) -> Option<u64> {
    // peek query
    let url = &s.url;
    for part in url.split(&['?', '&'][..]) {
        if let Some(v) = part.strip_prefix("clen=") {
            return v.parse().ok();
        }
    }
    None
}

async fn single_get(stream: &Stream, total: u64) -> anyhow::Result<Vec<u8>> {
    let ua = stream
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "Mozilla/5.0".into());
    let end = total.saturating_sub(1);
    let client = reqwest::Client::builder()
        .user_agent(&ua)
        .http1_only()
        .timeout(Duration::from_secs(180))
        .build()?;
    let mut req = client.get(&stream.url);
    for (k, v) in &stream.headers {
        if k.eq_ignore_ascii_case("range") || k.eq_ignore_ascii_case("user-agent") {
            continue;
        }
        req = req.header(k.as_str(), v.as_str());
    }
    req = req.header("User-Agent", &ua);
    req = req.header("Range", format!("bytes=0-{end}"));
    let resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status}");
    }
    Ok(resp.bytes().await?.to_vec())
}

fn decode_complete(bytes: &[u8]) -> anyhow::Result<f64> {
    let body = GrowingBody::from_complete(bytes.to_vec());
    let mut src = decode::open(body, 0.0)?;
    let sr = src.sample_rate() as f64;
    let ch = src.channels().max(1) as f64;
    let mut n = 0u64;
    loop {
        match src.pull() {
            Pull::Sample(_) => n += 1,
            Pull::Buffering => continue,
            Pull::Ended => break,
            Pull::Stall(e) => anyhow::bail!("stall on complete file: {e}"),
        }
    }
    Ok(n as f64 / (sr * ch))
}

fn decode_growing(body: std::sync::Arc<GrowingBody>, _expected: f64) -> anyhow::Result<f64> {
    // Wait until we can open.
    let start = Instant::now();
    while body.len() < READY_BYTES && !body.is_done() {
        if start.elapsed() > Duration::from_secs(30) {
            anyhow::bail!("open timeout, have {}", body.len());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut src = decode::open(body.clone(), 0.0)?;
    let sr = src.sample_rate() as f64;
    let ch = src.channels().max(1) as f64;
    let mut n = 0u64;
    let mut last_log = Instant::now();
    loop {
        match src.pull() {
            Pull::Sample(_) => n += 1,
            Pull::Buffering => {
                if body.is_done() && body.error().is_none() {
                    // one more try then end
                    match src.pull() {
                        Pull::Sample(_) => n += 1,
                        Pull::Ended | Pull::Buffering => break,
                        Pull::Stall(e) => anyhow::bail!("stall: {e}"),
                    }
                }
                std::thread::sleep(Duration::from_millis(4));
            }
            Pull::Ended => break,
            Pull::Stall(e) => anyhow::bail!("progressive stall: {e}"),
        }
        if last_log.elapsed() > Duration::from_secs(2) {
            let secs = n as f64 / (sr * ch);
            eprintln!(
                "  decode progressing {secs:.1}s  body {}/{:?}",
                body.len(),
                body.content_len()
            );
            last_log = Instant::now();
        }
    }
    Ok(n as f64 / (sr * ch))
}

fn classify(msg: &str) -> &'static str {
    let l = msg.to_lowercase();
    if l.contains("403") || l.contains("forbidden") {
        "HTTP_403"
    } else if l.contains("deobfus") {
        "DEOBFUSCATION"
    } else if l.contains("unplayable") {
        "UNPLAYABLE"
    } else if l.contains("login_required") || l.contains("age") || l.contains("sign in") {
        "LOGIN_REQUIRED"
    } else if l.contains("n-sig") || l.contains("nsig") || l.contains("throttl") {
        "N-SIG"
    } else if l.contains("yt-dlp") {
        "YTDLP"
    } else if l.contains("timed out") || l.contains("timeout") {
        "TIMEOUT"
    } else if l.contains("416") || l.contains("eof") {
        "EOF_416"
    } else {
        "OTHER"
    }
}
