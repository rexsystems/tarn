//! Discord Rich Presence: Listening status with album cover.
//! Failures are non-fatal (Discord not running, reconnect, etc).

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use discord_rich_presence::{
    DiscordIpc, DiscordIpcClient,
    activity::{Activity, ActivityType, Assets, Button, StatusDisplayType, Timestamps},
};

use crate::ytm::upgrade_cover;

/// Public Discord Application ID for Tarn (Rich Presence).
/// Create at https://discord.com/developers/applications (name it Tarn) — no secret needed.
/// Override with env `TARN_DISCORD_CLIENT_ID` for local testing.
fn client_id() -> String {
    std::env::var("TARN_DISCORD_CLIENT_ID").unwrap_or_else(|_| CLIENT_ID.to_string())
}

const CLIENT_ID: &str = "1571320476662497340";

pub fn start(app: Arc<crate::App>) {
    std::thread::Builder::new()
        .name("tarn-discord".into())
        .spawn(move || {
            if let Err(e) = run(app) {
                eprintln!("discord presence: {e}");
            }
        })
        .ok();
}

fn run(app: Arc<crate::App>) -> Result<(), String> {
    let mut client: Option<DiscordIpcClient> = None;
    let mut last_sig: Option<String> = None;
    let mut enabled_last = true;
    let mut clear_needed = false;

    loop {
        let enabled = app.discord_enabled();
        if !enabled {
            if enabled_last {
                if let Some(c) = client.as_mut() {
                    let _ = c.clear_activity();
                }
                last_sig = None;
                clear_needed = false;
            }
            enabled_last = false;
            std::thread::sleep(Duration::from_millis(800));
            continue;
        }
        enabled_last = true;

        if client.is_none() {
            match connect() {
                Ok(c) => {
                    eprintln!("discord presence: connected");
                    client = Some(c);
                    last_sig = None;
                }
                Err(e) => {
                    // Missing app id / Discord closed — quiet backoff.
                    if e.contains("CLIENT_ID") {
                        std::thread::sleep(Duration::from_secs(30));
                    } else {
                        std::thread::sleep(Duration::from_secs(5));
                    }
                    continue;
                }
            }
        }

        let snap = app.player.snapshot();
        let Some(track) = snap.track.as_ref() else {
            if clear_needed {
                if let Some(c) = client.as_mut() {
                    if c.clear_activity().is_err() {
                        client = None;
                    }
                }
                clear_needed = false;
                last_sig = None;
            }
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };

        let artists = track.artists.join(", ");
        let cover = track
            .cover
            .as_deref()
            .map(upgrade_cover)
            .filter(|u| u.starts_with("http"));
        // Bucket position so we don't spam IPC every tick; refresh ~every 2s.
        let pos_bucket = (snap.position.max(0.0) / 2.0) as u64;
        let sig = format!(
            "{}|{}|{}|{}|{}",
            track.id,
            snap.playing,
            pos_bucket,
            cover.as_deref().unwrap_or(""),
            artists
        );

        if Some(&sig) == last_sig.as_ref() {
            std::thread::sleep(Duration::from_millis(400));
            continue;
        }

        let ytm_url = format!("https://music.youtube.com/watch?v={}", track.id);
        let album = track.album.as_deref().unwrap_or("");
        let large_text = if !album.is_empty() {
            album
        } else if !artists.is_empty() {
            artists.as_str()
        } else {
            "Tarn"
        };

        let mut assets = Assets::new().large_text(large_text);
        if let Some(ref url) = cover {
            assets = assets.large_image(url.as_str());
        }

        let mut activity = Activity::new()
            .activity_type(ActivityType::Listening)
            .status_display_type(StatusDisplayType::Details)
            .details(track.title.as_str())
            .state(if artists.is_empty() {
                "Tarn"
            } else {
                artists.as_str()
            })
            .assets(assets)
            .buttons(vec![Button::new("Play on YouTube Music", ytm_url.as_str())]);

        if snap.playing && snap.duration > 1.0 {
            if let Some(ts) = timestamps(snap.position, snap.duration) {
                activity = activity.timestamps(ts);
            }
        }

        let c = client.as_mut().unwrap();
        match c.set_activity(activity) {
            Ok(()) => {
                last_sig = Some(sig);
                clear_needed = true;
            }
            Err(e) => {
                eprintln!("discord presence: set_activity: {e}; reconnecting");
                let _ = c.close();
                client = None;
                last_sig = None;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        }

        std::thread::sleep(Duration::from_millis(400));
    }
}

fn connect() -> Result<DiscordIpcClient, String> {
    let id = client_id();
    if id == "REPLACE_ME_DISCORD_APP_ID" || id.is_empty() {
        return Err("set TARN_DISCORD_CLIENT_ID or CLIENT_ID in discord.rs".into());
    }
    let mut client = DiscordIpcClient::new(id.as_str());
    client
        .connect()
        .map_err(|e| format!("connect: {e}"))?;
    Ok(client)
}

fn timestamps(position: f64, duration: f64) -> Option<Timestamps> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let pos = Duration::from_secs_f64(position.max(0.0));
    let dur = Duration::from_secs_f64(duration.max(0.0));
    let start = now.checked_sub(pos)?;
    let end = start.checked_add(dur)?;
    // discord-rich-presence 1.1 documents ms; IPC historically used seconds.
    // Seconds match working music clients (trackpresence, jellyfin-rpc).
    Some(
        Timestamps::new()
            .start(start.as_secs() as i64)
            .end(end.as_secs() as i64),
    )
}
