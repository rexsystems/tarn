//! OS media controls: MPRIS on Linux, SMTC on Windows, Now Playing on macOS.
//! Failures are non-fatal. Media-key events are applied directly to the
//! player (and to next/prev via the async advance path) so they do not
//! depend on a frontend event listener.

use std::sync::Arc;
use std::time::Duration;

use tauri::AppHandle;

use crate::models::Track;
use crate::player::NextChoice;

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
        let _ = on;
    }
}

/// Native window handle required by Windows SMTC. Ignored on other OS.
/// Native HWND stored as usize so it can cross threads. Ignored off Windows.
pub type WindowHandle = Option<usize>;

pub fn start(app: Arc<crate::App>, _handle: AppHandle, hwnd: WindowHandle) {
    let app_m = app.clone();
    std::thread::Builder::new()
        .name("tarn-mpris".into())
        .spawn(move || {
            if let Err(e) = run_souvlaki(app_m, hwnd) {
                eprintln!("media controls: {e}");
            }
        })
        .ok();

    // Global media-key shortcuts as a fallback when the OS does not
    // route keys through MPRIS/SMTC (some compositors, unlocked HUDs).
    let app_h = app.clone();
    std::thread::Builder::new()
        .name("tarn-mediakeys".into())
        .spawn(move || {
            if let Err(e) = run_global_hotkeys(app_h) {
                eprintln!("media key shortcuts: {e}");
            }
        })
        .ok();
}

fn dispatch(app: &Arc<crate::App>, ev: souvlaki::MediaControlEvent) {
    use souvlaki::{MediaControlEvent, SeekDirection};
    match ev {
        MediaControlEvent::Toggle => app.player.toggle(),
        MediaControlEvent::Play => app.player.resume(),
        MediaControlEvent::Pause => app.player.pause(),
        MediaControlEvent::Next => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::advance(&app, true).await {
                    eprintln!("media next: {e}");
                }
            });
        }
        MediaControlEvent::Previous => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                match app.player.prev_choice() {
                    NextChoice::SeekZero => app.player.seek(0.0),
                    NextChoice::Play(i) => {
                        let (q, _) = app.player.queue_and_index();
                        if let Some(t) = q.get(i).cloned() {
                            let _ = crate::start_playback(&app, t, q, i).await;
                        }
                    }
                    _ => {}
                }
            });
        }
        MediaControlEvent::Stop => app.player.stop(),
        MediaControlEvent::SetPosition(p) => app.player.seek(p.0.as_secs_f64()),
        MediaControlEvent::Seek(d) => {
            let snap = app.player.snapshot();
            let delta = match d {
                SeekDirection::Forward => 10.0,
                SeekDirection::Backward => -10.0,
            };
            app.player.seek((snap.position + delta).max(0.0));
        }
        MediaControlEvent::SeekBy(d, dur) => {
            let snap = app.player.snapshot();
            let secs = dur.as_secs_f64();
            let delta = match d {
                SeekDirection::Forward => secs,
                SeekDirection::Backward => -secs,
            };
            app.player.seek((snap.position + delta).max(0.0));
        }
        MediaControlEvent::SetVolume(v) => app.player.set_volume(v as f32),
        MediaControlEvent::Quit | MediaControlEvent::Raise | MediaControlEvent::OpenUri(_) => {}
    }
}

fn run_souvlaki(app: Arc<crate::App>, hwnd: WindowHandle) -> Result<(), String> {
    use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};

    #[cfg(windows)]
    let hwnd = Some(
        hwnd.ok_or_else(|| {
            "Windows SMTC requires a window HWND; none was provided".to_string()
        })? as *mut std::ffi::c_void,
    );
    #[cfg(not(windows))]
    let hwnd = {
        let _ = hwnd;
        None
    };

    // Short, valid MPRIS instance name: org.mpris.MediaPlayer2.tarn
    // (dots inside the instance part confuse some playerctl versions).
    // Session bus may not be ready at the exact moment of first startup.
    let mut last_err = None;
    let mut controls = None;
    for attempt in 0..8 {
        let config = PlatformConfig {
            dbus_name: "tarn",
            display_name: "Tarn",
            hwnd,
        };
        match MediaControls::new(config) {
            Ok(c) => {
                controls = Some(c);
                break;
            }
            Err(e) => {
                last_err = Some(format!("{e:?}"));
                std::thread::sleep(Duration::from_millis(250 * (attempt + 1) as u64));
            }
        }
    }
    let mut controls = controls.ok_or_else(|| {
        last_err.unwrap_or_else(|| "failed to create media controls".into())
    })?;

    let (tx, rx) = std::sync::mpsc::channel::<MediaControlEvent>();
    controls
        .attach(move |e| {
            let _ = tx.send(e);
        })
        .map_err(|e| format!("attach: {e:?}"))?;

    eprintln!("media controls: registered (MPRIS name org.mpris.MediaPlayer2.tarn / SMTC)");

    // Publish an initial Stopped state so the player is visible to
    // playerctl / GNOME / SMTC before the first track is chosen.
    let _ = controls.set_playback(MediaPlayback::Stopped);
    let _ = controls.set_metadata(MediaMetadata {
        title: Some("Tarn"),
        artist: Some(""),
        ..Default::default()
    });

    let mut last_id: Option<String> = None;
    let mut last_playing: Option<bool> = None;
    let mut last_pos_bucket: u64 = u64::MAX;
    let mut cover_url: Option<String> = None;

    loop {
        while let Ok(ev) = rx.try_recv() {
            dispatch(&app, ev);
        }

        let snap = app.player.snapshot();
        let playing = snap.playing;
        let id = snap.track.as_ref().map(|t| t.id.clone());
        // Refresh position about once a second so system UIs can seek.
        let pos_bucket = (snap.position.max(0.0) * 2.0) as u64;
        let changed = id != last_id
            || Some(playing) != last_playing
            || pos_bucket != last_pos_bucket;

        if changed {
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
                    maybe_notify(&app, t);
                    cover_url = t.cover.clone();
                    let _ = cover_url;
                }
            }
            let progress = Some(souvlaki::MediaPosition(Duration::from_secs_f64(
                snap.position.max(0.0),
            )));
            let _ = controls.set_playback(if playing {
                MediaPlayback::Playing { progress }
            } else if snap.track.is_some() {
                MediaPlayback::Paused { progress }
            } else {
                MediaPlayback::Stopped
            });
            last_id = id;
            last_playing = Some(playing);
            last_pos_bucket = pos_bucket;
        }

        // Windows SMTC needs the UI thread to pump messages (Tauri does
        // that for the real HWND). A short sleep keeps this thread polite.
        std::thread::sleep(Duration::from_millis(150));
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

fn run_global_hotkeys(app: Arc<crate::App>) -> Result<(), String> {
    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

    let manager = GlobalHotKeyManager::new().map_err(|e| format!("init: {e}"))?;
    let play = HotKey::new(None::<Modifiers>, Code::MediaPlayPause);
    let stop = HotKey::new(None::<Modifiers>, Code::MediaStop);
    let next = HotKey::new(None::<Modifiers>, Code::MediaTrackNext);
    let prev = HotKey::new(None::<Modifiers>, Code::MediaTrackPrevious);
    // Some keyboards expose these as "Play" rather than PlayPause.
    let play2 = HotKey::new(None::<Modifiers>, Code::MediaPlay);
    let pause2 = HotKey::new(None::<Modifiers>, Code::MediaPause);
    let keys = [play, stop, next, prev, play2, pause2];
    manager
        .register_all(&keys)
        .map_err(|e| format!("register: {e}"))?;
    eprintln!("media key shortcuts: registered PlayPause/Next/Previous/Stop");

    let rx = GlobalHotKeyEvent::receiver();
    for ev in rx {
        if ev.state != HotKeyState::Pressed {
            continue;
        }
        use souvlaki::MediaControlEvent;
        let cmd = if ev.id == play.id() || ev.id == play2.id() {
            MediaControlEvent::Toggle
        } else if ev.id == pause2.id() {
            MediaControlEvent::Pause
        } else if ev.id == stop.id() {
            MediaControlEvent::Stop
        } else if ev.id == next.id() {
            MediaControlEvent::Next
        } else if ev.id == prev.id() {
            MediaControlEvent::Previous
        } else {
            continue;
        };
        dispatch(&app, cmd);
    }
    // Keep the manager alive for the lifetime of the receiver loop.
    drop(manager);
    Ok(())
}
