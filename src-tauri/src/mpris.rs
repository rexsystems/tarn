//! MPRIS + desktop notifications. Failures are non-fatal.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::models::Track;

pub struct MediaHook {
    last_id: std::sync::Mutex<Option<String>>,
    notify: bool,
}

impl MediaHook {
    pub fn new() -> Self {
        Self {
            last_id: std::sync::Mutex::new(None),
            notify: true,
        }
    }

    pub fn set_notify(&self, on: bool) {
        // stored on App settings; this is a local cache updated each tick
        let _ = on;
    }
}

pub fn start(app: Arc<crate::App>, handle: AppHandle) {
    std::thread::Builder::new()
        .name("tarn-mpris".into())
        .spawn(move || run(app, handle))
        .ok();
}

fn run(app: Arc<crate::App>, handle: AppHandle) {
    match run_souvlaki(app.clone(), handle.clone()) {
        Ok(()) => {}
        Err(e) => eprintln!("mpris: {e}"),
    }
}

fn run_souvlaki(app: Arc<crate::App>, handle: AppHandle) -> Result<(), String> {
    use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};

    let hwnd = None;
    let config = PlatformConfig {
        dbus_name: "app.tarn.desktop",
        display_name: "Tarn",
        hwnd,
    };
    let mut controls = MediaControls::new(config).map_err(|e| format!("{e:?}"))?;
    let (tx, rx) = std::sync::mpsc::channel::<MediaControlEvent>();
    controls
        .attach(move |e| {
            let _ = tx.send(e);
        })
        .map_err(|e| format!("attach: {e:?}"))?;

    let mut last_id: Option<String> = None;
    let mut last_playing: Option<bool> = None;
    let mut cover_url: Option<String> = None;

    loop {
        while let Ok(ev) = rx.try_recv() {
            match ev {
                MediaControlEvent::Toggle => app.player.toggle(),
                MediaControlEvent::Play => app.player.resume(),
                MediaControlEvent::Pause => app.player.pause(),
                MediaControlEvent::Next => {
                    let _ = handle.emit("media-cmd", "next");
                }
                MediaControlEvent::Previous => {
                    let _ = handle.emit("media-cmd", "prev");
                }
                MediaControlEvent::Stop => app.player.stop(),
                MediaControlEvent::SetPosition(p) => {
                    app.player.seek(p.0.as_secs_f64());
                }
                MediaControlEvent::Seek(d) => {
                    let snap = app.player.snapshot();
                    let delta = match d {
                        souvlaki::SeekDirection::Forward => 10.0,
                        souvlaki::SeekDirection::Backward => -10.0,
                    };
                    app.player.seek((snap.position + delta).max(0.0));
                }
                MediaControlEvent::SetVolume(v) => app.player.set_volume(v as f32),
                _ => {}
            }
        }

        let snap = app.player.snapshot();
        let playing = snap.playing;
        let id = snap.track.as_ref().map(|t| t.id.clone());
        if id != last_id || Some(playing) != last_playing {
            if let Some(t) = &snap.track {
                let artists = t.artists.join(", ");
                let dur = snap.duration;
                let _ = controls.set_metadata(MediaMetadata {
                    title: Some(&t.title),
                    artist: Some(&artists),
                    album: t.album.as_deref(),
                    duration: if dur > 0.0 {
                        Some(Duration::from_secs_f64(dur))
                    } else {
                        None
                    },
                    cover_url: t.cover.as_deref(),
                    ..Default::default()
                });
                if id != last_id {
                    if snap.autoplay || true {
                        // notifications gated by settings in App
                        maybe_notify(&app, t);
                    }
                    cover_url = t.cover.clone();
                    let _ = cover_url;
                }
            }
            let _ = controls.set_playback(if playing {
                MediaPlayback::Playing {
                    progress: Some(souvlaki::MediaPosition(Duration::from_secs_f64(
                        snap.position.max(0.0),
                    ))),
                }
            } else if snap.track.is_some() {
                MediaPlayback::Paused {
                    progress: Some(souvlaki::MediaPosition(Duration::from_secs_f64(
                        snap.position.max(0.0),
                    ))),
                }
            } else {
                MediaPlayback::Stopped
            });
            last_id = id;
            last_playing = Some(playing);
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

fn maybe_notify(app: &crate::App, t: &Track) {
    if !app.notifications_enabled() {
        return;
    }
    let artists = t.artists.join(", ");
    let mut n = notify_rust::Notification::new();
    n.appname("Tarn")
        .summary(&t.title)
        .body(&artists)
        .timeout(notify_rust::Timeout::Milliseconds(4000));
    if let Some(icon) = crate::notify_icon() {
        n.icon(&icon);
    }
    if let Err(e) = n.show() {
        eprintln!("notify: {e}");
    }
}
