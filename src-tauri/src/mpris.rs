//! OS media controls: MPRIS on Linux, SMTC on Windows, Now Playing on macOS.
//! Failures are non-fatal. Media-key events are applied directly to the
//! player (and to next/prev via the async advance path) so they do not
//! depend on a frontend event listener.

use std::sync::Arc;
use std::time::Duration;

use tauri::AppHandle;

use crate::models::Track;
use crate::player::NextChoice;

/// Native window handle required by Windows SMTC. Ignored on other OS.
/// Native HWND stored as usize so it can cross threads. Ignored off Windows.
pub type WindowHandle = Option<usize>;

pub fn start(app: Arc<crate::App>, handle: AppHandle, hwnd: WindowHandle) {
    let _ = &handle;
    // Windows: SystemMediaTransportControls must be created and attached on
    // a thread that pumps window messages — ideally the window's UI thread.
    // A background thread with just `sleep` never delivers ButtonPressed,
    // which is why media keys appeared to "do nothing" even with a valid HWND.
    #[cfg(windows)]
    {
        let app_w = app.clone();
        let handle_w = handle.clone();
        match start_smtc_ui(app_w, handle_w, hwnd) {
            Ok(()) => eprintln!("media controls: SMTC attached on the UI thread"),
            Err(e) => {
                eprintln!("media controls: UI-thread attach failed: {e}; trying STA pump");
                let app_s = app.clone();
                std::thread::Builder::new()
                    .name("tarn-smtc".into())
                    .spawn(move || {
                        if let Err(e) = run_windows_smtc(app_s, hwnd) {
                            eprintln!("media controls (sta): {e}");
                        }
                    })
                    .ok();
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        let app_m = app.clone();
        std::thread::Builder::new()
            .name("tarn-mpris".into())
            .spawn(move || {
                if let Err(e) = run_souvlaki(app_m, None) {
                    eprintln!("media controls: {e}");
                }
            })
            .ok();
    }

    // Global media-key shortcuts as a fallback when the OS does not
    // route keys through MPRIS/SMTC (some compositors, unlocked HUDs).
    // Registered one-by-one: register_all is all-or-nothing, and on Linux
    // MediaPlayPause often has no scancode, which disabled every key.
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
    eprintln!("media key: {ev:?}");
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
                });
                if id != last_id {
                    maybe_notify(&app, t);
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
    n.appname(crate::win::APP_NAME)
        .summary(&t.title)
        .body(&artists)
        .timeout(notify_rust::Timeout::Milliseconds(4000));
    #[cfg(windows)]
    {
        n.app_id(crate::win::APP_USER_MODEL_ID);
    }
    if let Some(icon) = crate::notify_icon() {
        n.icon(&icon);
        n.image_path(&icon);
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
    let named = [
        ("MediaPlayPause", play),
        ("MediaStop", stop),
        ("MediaTrackNext", next),
        ("MediaTrackPrevious", prev),
        ("MediaPlay", play2),
        ("MediaPause", pause2),
    ];
    for (name, key) in &named {
        match manager.register(*key) {
            Ok(()) => eprintln!("media key shortcuts: registered {name}"),
            Err(e) => eprintln!("media key shortcuts: {name} failed: {e}"),
        }
    }

    let rx = GlobalHotKeyEvent::receiver();
    let mut last = std::time::Instant::now() - Duration::from_secs(1);
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
            eprintln!("media key shortcuts: unknown id {}", ev.id);
            continue;
        };
        // Debounce so SMTC and the global hook don't double-fire.
        let now = std::time::Instant::now();
        if now.duration_since(last) < Duration::from_millis(250) {
            eprintln!("media key: debounce skip {cmd:?}");
            continue;
        }
        last = now;
        eprintln!("media key (global): {cmd:?}");
        dispatch(&app, cmd);
    }
    // Keep the manager alive for the lifetime of the receiver loop.
    drop(manager);
    Ok(())
}


/// Create and attach SMTC on the calling thread (must pump messages —
/// Tauri's UI thread does). MediaControls stays on that thread; a
/// background loop posts metadata/status updates via `run_on_main_thread`.
#[cfg(windows)]
fn start_smtc_ui(
    app: Arc<crate::App>,
    handle: AppHandle,
    hwnd: WindowHandle,
) -> Result<(), String> {
    use souvlaki::{MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};
    use std::cell::RefCell;

    let hwnd = hwnd.ok_or_else(|| {
        "Windows SMTC requires a window HWND; none was provided".to_string()
    })? as *mut std::ffi::c_void;
    eprintln!("media controls: GetForWindow hwnd={hwnd:?} (UI thread)");

    let mut controls = MediaControls::new(PlatformConfig {
        dbus_name: "tarn",
        display_name: "Tarn",
        hwnd: Some(hwnd),
    })
    .map_err(|e| format!("SMTC new: {e:?}"))?;

    let app_cb = app.clone();
    controls
        .attach(move |e| {
            eprintln!("media key (SMTC callback): {e:?}");
            dispatch(&app_cb, e);
        })
        .map_err(|e| format!("SMTC attach: {e:?}"))?;

    let _ = controls.set_playback(MediaPlayback::Stopped);
    let _ = controls.set_metadata(MediaMetadata {
        title: Some("Tarn"),
        artist: Some(""),
        ..Default::default()
    });

    // Leak a RefCell so we can touch the object only on this thread.
    // `MediaControls` may not be Send; `run_on_main_thread` keeps access here.
    let slot: &'static RefCell<Option<MediaControls>> =
        Box::leak(Box::new(RefCell::new(Some(controls))));

    std::thread::Builder::new()
        .name("tarn-smtc-loop".into())
        .spawn(move || {
            let mut last_id: Option<String> = None;
            let mut last_playing: Option<bool> = None;
            let mut last_pos_bucket: u64 = u64::MAX;
            loop {
                let snap = app.player.snapshot();
                let playing = snap.playing;
                let id = snap.track.as_ref().map(|t| t.id.clone());
                let pos_bucket = (snap.position.max(0.0) * 2.0) as u64;
                let changed =
                    id != last_id || Some(playing) != last_playing || pos_bucket != last_pos_bucket;
                if changed {
                    let id_changed = id != last_id;
                    last_id = id.clone();
                    last_playing = Some(playing);
                    last_pos_bucket = pos_bucket;
                    if id_changed {
                        if let Some(t) = &snap.track {
                            maybe_notify(&app, t);
                        }
                    }
                    if let Err(e) = handle.run_on_main_thread(move || {
                        use souvlaki::{MediaMetadata, MediaPlayback};
                        let mut slot = slot.borrow_mut();
                        let Some(controls) = slot.as_mut() else {
                            return;
                        };
                        if let Some(t) = &snap.track {
                            let artists = t.artists.join(", ");
                            let dur = snap.duration;
                            if let Err(e) = controls.set_metadata(MediaMetadata {
                                title: Some(&t.title),
                                artist: Some(&artists),
                                album: t.album.as_deref(),
                                duration: if dur > 0.0 {
                                    Some(Duration::from_secs_f64(dur))
                                } else {
                                    None
                                },
                                cover_url: t.cover.as_deref(),
                            }) {
                                eprintln!("media controls: set_metadata: {e:?}");
                            }
                        }
                        let progress = Some(souvlaki::MediaPosition(Duration::from_secs_f64(
                            snap.position.max(0.0),
                        )));
                        // SMTC ignores media keys unless status is Playing
                        // or Paused (a Stopped session swallows them).
                        let status = if playing {
                            MediaPlayback::Playing { progress }
                        } else if snap.track.is_some() {
                            MediaPlayback::Paused { progress }
                        } else {
                            MediaPlayback::Stopped
                        };
                        if let Err(e) = controls.set_playback(status) {
                            eprintln!("media controls: set_playback: {e:?}");
                        }
                    }) {
                        eprintln!("media controls: run_on_main_thread: {e}");
                    }
                }
                std::thread::sleep(Duration::from_millis(150));
            }
        })
        .map_err(|e| format!("smtc loop: {e}"))?;
    Ok(())
}

/// Fallback: dedicated STA thread with a Win32 message pump, in case
/// attaching on the UI thread failed (COM apartment, etc.).
#[cfg(windows)]
fn run_windows_smtc(app: Arc<crate::App>, hwnd: WindowHandle) -> Result<(), String> {
    win_pump::co_init_sta();
    use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};

    let hwnd = hwnd.ok_or_else(|| {
        "Windows SMTC requires a window HWND; none was provided".to_string()
    })? as *mut std::ffi::c_void;
    eprintln!("media controls: STA GetForWindow hwnd={hwnd:?}");

    let mut controls = MediaControls::new(PlatformConfig {
        dbus_name: "tarn",
        display_name: "Tarn",
        hwnd: Some(hwnd),
    })
    .map_err(|e| format!("SMTC new: {e:?}"))?;

    let (tx, rx) = std::sync::mpsc::channel::<MediaControlEvent>();
    controls
        .attach(move |e| {
            eprintln!("media key (SMTC STA): {e:?}");
            let _ = tx.send(e);
        })
        .map_err(|e| format!("SMTC attach: {e:?}"))?;

    let _ = controls.set_playback(MediaPlayback::Stopped);
    let _ = controls.set_metadata(MediaMetadata {
        title: Some("Tarn"),
        artist: Some(""),
        ..Default::default()
    });

    eprintln!("media controls: SMTC attached on STA thread with message pump");
    smtc_update_loop_pumped(app, controls, rx);
    Ok(())
}

#[cfg(windows)]
fn smtc_update_loop(
    app: Arc<crate::App>,
    mut controls: souvlaki::MediaControls,
    rx: std::sync::mpsc::Receiver<souvlaki::MediaControlEvent>,
) {
    use souvlaki::{MediaMetadata, MediaPlayback};
    let mut last_id: Option<String> = None;
    let mut last_playing: Option<bool> = None;
    let mut last_pos_bucket: u64 = u64::MAX;
    loop {
        while let Ok(ev) = rx.try_recv() {
            dispatch(&app, ev);
        }
        let snap = app.player.snapshot();
        let playing = snap.playing;
        let id = snap.track.as_ref().map(|t| t.id.clone());
        let pos_bucket = (snap.position.max(0.0) * 2.0) as u64;
        let changed = id != last_id || Some(playing) != last_playing || pos_bucket != last_pos_bucket;
        if changed {
            if let Some(t) = &snap.track {
                let artists = t.artists.join(", ");
                let dur = snap.duration;
                if let Err(e) = controls.set_metadata(MediaMetadata {
                    title: Some(&t.title),
                    artist: Some(&artists),
                    album: t.album.as_deref(),
                    duration: if dur > 0.0 {
                        Some(Duration::from_secs_f64(dur))
                    } else {
                        None
                    },
                    cover_url: t.cover.as_deref(),
                }) {
                    eprintln!("media controls: set_metadata: {e:?}");
                }
                if id != last_id {
                    maybe_notify(&app, t);
                }
            }
            let progress = Some(souvlaki::MediaPosition(Duration::from_secs_f64(
                snap.position.max(0.0),
            )));
            // SMTC ignores media keys unless status is Playing or Paused
            // (Stopped with no session swallows them). Always publish.
            let status = if playing {
                MediaPlayback::Playing { progress }
            } else if snap.track.is_some() {
                MediaPlayback::Paused { progress }
            } else {
                MediaPlayback::Stopped
            };
            if let Err(e) = controls.set_playback(status) {
                eprintln!("media controls: set_playback: {e:?}");
            }
            last_id = id;
            last_playing = Some(playing);
            last_pos_bucket = pos_bucket;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

#[cfg(windows)]
fn smtc_update_loop_pumped(
    app: Arc<crate::App>,
    mut controls: souvlaki::MediaControls,
    rx: std::sync::mpsc::Receiver<souvlaki::MediaControlEvent>,
) {
    // Same as smtc_update_loop, but drain the Win32 queue between ticks so
    // ButtonPressed arrives on this STA thread.
    use souvlaki::{MediaMetadata, MediaPlayback};
    let mut last_id: Option<String> = None;
    let mut last_playing: Option<bool> = None;
    let mut last_pos_bucket: u64 = u64::MAX;
    loop {
        win_pump::pump();
        while let Ok(ev) = rx.try_recv() {
            dispatch(&app, ev);
        }
        let snap = app.player.snapshot();
        let playing = snap.playing;
        let id = snap.track.as_ref().map(|t| t.id.clone());
        let pos_bucket = (snap.position.max(0.0) * 2.0) as u64;
        let changed = id != last_id || Some(playing) != last_playing || pos_bucket != last_pos_bucket;
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
                });
                if id != last_id {
                    maybe_notify(&app, t);
                }
            }
            let progress = Some(souvlaki::MediaPosition(Duration::from_secs_f64(
                snap.position.max(0.0),
            )));
            let status = if playing {
                MediaPlayback::Playing { progress }
            } else if snap.track.is_some() {
                MediaPlayback::Paused { progress }
            } else {
                MediaPlayback::Stopped
            };
            let _ = controls.set_playback(status);
            last_id = id;
            last_playing = Some(playing);
            last_pos_bucket = pos_bucket;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Minimal Win32 FFI so we can pump messages without a new `windows` crate
/// feature set (and the version conflict that would come with it).
#[cfg(windows)]
mod win_pump {
    use std::ffi::c_void;

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct Msg {
        hwnd: *mut c_void,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: Point,
    }

    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const PM_REMOVE: u32 = 0x1;

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(pv: *mut c_void, dw: u32) -> i32;
    }
    #[link(name = "user32")]
    extern "system" {
        fn PeekMessageW(msg: *mut Msg, hwnd: *mut c_void, min: u32, max: u32, remove: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
    }

    pub fn co_init_sta() {
        unsafe {
            let hr = CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED);
            // S_OK (0) or S_FALSE (1, already initialized) are fine.
            if hr < 0 {
                eprintln!("media controls: CoInitializeEx failed: {hr:#x}");
            }
        }
    }

    pub fn pump() {
        unsafe {
            let mut msg = std::mem::zeroed::<Msg>();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
