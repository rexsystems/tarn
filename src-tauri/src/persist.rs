//! Playback and window session — restored on launch, saved on a short
//! interval and again when the app closes.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Position, Size};

use crate::models::Track;
use crate::App;

const FILE: &str = "playback.json";
const MAX_QUEUE: usize = 250;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WinGeom {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    #[serde(default)]
    pub maximized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiMem {
    pub view: Option<serde_json::Value>,
    pub search_q: Option<String>,
    pub scroll: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SavedSession {
    #[serde(default)]
    pub queue: Vec<Track>,
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub position: f64,
    #[serde(default = "default_vol")]
    pub volume: f32,
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default = "default_repeat")]
    pub repeat: String,
    #[serde(default)]
    pub view: Option<serde_json::Value>,
    #[serde(default)]
    pub search_q: Option<String>,
    #[serde(default)]
    pub scroll: Option<f64>,
    #[serde(default)]
    pub window: Option<WinGeom>,
}

fn default_vol() -> f32 {
    0.85
}
fn default_repeat() -> String {
    "off".into()
}

pub fn path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join(FILE)
}

pub fn load(dir: &std::path::Path) -> Option<SavedSession> {
    let raw = std::fs::read_to_string(path(dir)).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn write(dir: &std::path::Path, s: &SavedSession) {
    if let Ok(j) = serde_json::to_string(s) {
        let _ = std::fs::write(path(dir), j);
    }
}

pub fn read_geom(win: &tauri::WebviewWindow) -> Option<WinGeom> {
    let pos = win.outer_position().ok()?;
    let size = win.inner_size().ok()?;
    let maximized = win.is_maximized().unwrap_or(false);
    Some(WinGeom {
        x: pos.x,
        y: pos.y,
        w: size.width.max(1),
        h: size.height.max(1),
        maximized,
    })
}

fn on_a_monitor(win: &tauri::WebviewWindow, g: &WinGeom) -> bool {
    let Ok(monitors) = win.available_monitors() else {
        return true;
    };
    if monitors.is_empty() {
        return true;
    }
    monitors.iter().any(|m| {
        let p = m.position();
        let s = m.size();
        let left = p.x;
        let top = p.y;
        let right = p.x.saturating_add(s.width.min(i32::MAX as u32) as i32);
        let bottom = p.y.saturating_add(s.height.min(i32::MAX as u32) as i32);
        // At least 80px of the title bar should sit on this display.
        let cx = g.x.saturating_add(80.min(g.w as i32));
        let cy = g.y.saturating_add(24);
        cx >= left && cx < right && cy >= top && cy < bottom
    })
}

pub fn apply_window(win: &tauri::WebviewWindow, g: &WinGeom) {
    let w = g.w.clamp(900, 8000);
    let h = g.h.clamp(620, 8000);
    let _ = win.set_size(Size::Physical(PhysicalSize { width: w, height: h }));
    if on_a_monitor(win, g) {
        let _ = win.set_position(Position::Physical(PhysicalPosition { x: g.x, y: g.y }));
    }
    if g.maximized {
        let _ = win.maximize();
    }
}

pub fn collect(app: &App, win: Option<&tauri::WebviewWindow>) -> SavedSession {
    let snap = app.player.snapshot();
    let ui = app.ui.lock().clone();
    let mut queue = snap.queue;
    if queue.len() > MAX_QUEUE {
        let start = snap.index.saturating_sub(MAX_QUEUE / 2);
        let end = (start + MAX_QUEUE).min(queue.len());
        queue = queue[start..end].to_vec();
    }
    let index = snap
        .track
        .as_ref()
        .and_then(|t| queue.iter().position(|x| x.id == t.id))
        .unwrap_or(snap.index.min(queue.len().saturating_sub(1)));
    SavedSession {
        queue,
        index,
        position: snap.position.max(0.0),
        volume: snap.volume.clamp(0.0, 1.0),
        shuffle: snap.shuffle,
        repeat: snap.repeat,
        view: ui.view,
        search_q: ui.search_q,
        scroll: ui.scroll,
        window: win.and_then(read_geom),
    }
}

pub fn save(handle: &AppHandle) {
    let Some(app) = handle.try_state::<std::sync::Arc<App>>() else {
        return;
    };
    let win = handle.get_webview_window("main");
    let mut sess = collect(&app, win.as_ref());
    // Closing the window can make geometry unreadable; keep the last good one.
    if sess.window.is_none() {
        if let Some(prev) = load(app.store.data_dir()) {
            sess.window = prev.window;
        }
    }
    write(app.store.data_dir(), &sess);
}

pub fn apply_player_prefs(app: &App, sess: &SavedSession) {
    app.player.set_volume(sess.volume);
    app.player.set_shuffle(sess.shuffle);
    app.player.set_repeat(&sess.repeat);
    if !sess.queue.is_empty() {
        let idx = sess.index.min(sess.queue.len() - 1);
        let track = sess.queue[idx].clone();
        let pos = sess.position.max(0.0);
        let dur = track.duration.map(|d| d as f64).unwrap_or(0.0);
        let pos = if dur > 1.0 { pos.min(dur - 0.4).max(0.0) } else { pos };
        app.player.prepare(track, sess.queue.clone(), idx, pos, false);
    }
}
