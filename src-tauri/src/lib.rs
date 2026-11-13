mod auth;
mod diag;
mod cache;
mod color;
pub mod decode;
mod innertube;
mod login;
mod mix;
mod mpris;
mod models;
mod persist;
mod win;
pub mod player;
pub mod ytdlp;
pub mod ytm;

use std::sync::Arc;

use models::{
    ArtistPage, AuthStatus, Collection, Explore, GenrePage, Home, Library, LikeState, Lyrics,
    PlayerState, Related, SearchResults, Settings, Track,
};
use parking_lot::Mutex;
use player::{NextChoice, Player};
use tauri::{AppHandle, Emitter, Manager, State};
use ytm::Ytm;

use crate::auth::SessionStore;

pub struct App {
    ytm: std::sync::Arc<Ytm>,
    pub player: Player,
    http: reqwest::Client,
    store: SessionStore,
    settings: Mutex<Settings>,
    auth: Mutex<AuthStatus>,
    notify: Mutex<bool>,
    pub ui: Mutex<persist::UiMem>,
    pub pending_toast: Mutex<Option<String>>,
    skip_fails: std::sync::atomic::AtomicU32,
}

impl App {
    fn new() -> anyhow::Result<Self> {
        let store = SessionStore::new()?;
        let settings = load_settings(store.data_dir());
        let ytm = std::sync::Arc::new(Ytm::new()?);
        ytm.apply_settings(&settings);
        let mock = std::env::var("TARN_MOCK_AUTH").ok().as_deref() == Some("1");
        let auth = if mock {
            AuthStatus {
                signed_in: true,
                mock: true,
                account: Some(models::AccountInfo {
                    name: "Alex Stefan".into(),
                    photo: None,
                    handle: Some("@preview".into()),
                    page_id: None,
                }),
                store: Some("mock".into()),
            }
        } else {
            AuthStatus::default()
        };
        let player = Player::start();
        player.set_autoplay(settings.autoplay);
        player.set_crossfade(
            settings.crossfade,
            settings.crossfade_seconds,
            settings.album_continuity,
        );
        if let Some(sess) = persist::load(store.data_dir()) {
            // Volume / shuffle / repeat / queue metadata — audio is armed later.
            player.set_volume(sess.volume);
            player.set_shuffle(sess.shuffle);
            player.set_repeat(&sess.repeat);
        }
        Ok(Self {
            ytm,
            player,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(12))
                .build()?,
            store,
            notify: Mutex::new(settings.notifications),
            settings: Mutex::new(settings),
            auth: Mutex::new(auth),
            ui: Mutex::new(persist::UiMem::default()),
            pending_toast: Mutex::new(None),
            skip_fails: std::sync::atomic::AtomicU32::new(0),
        })
    }

    pub fn notifications_enabled(&self) -> bool {
        *self.notify.lock()
    }
}

pub fn notify_icon() -> Option<String> {
    for p in [
        "/usr/share/icons/hicolor/256x256/apps/tarn.png",
        "/usr/share/pixmaps/tarn.png",
    ] {
        if std::path::Path::new(p).exists() {
            return Some(p.into());
        }
    }
    // Fall back to a copy of the bundled icon in the data dir so Windows
    // toasts and unpackaged runs have a file path to point at.
    let dir = dirs::data_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("tarn");
    let dest = dir.join("notify.png");
    if !dest.exists() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(&dest, include_bytes!("../icons/icon.png"));
    }
    if dest.is_file() {
        return Some(dest.to_string_lossy().into());
    }
    None
}

fn load_settings(dir: &std::path::Path) -> Settings {
    let p = dir.join("settings.json");
    if let Ok(s) = std::fs::read_to_string(&p) {
        if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&s) {
            if let Ok(mut v) = serde_json::from_value::<Settings>(raw.clone()) {
                // One-shot: older builds defaulted notifications to on, and
                // that value was saved. Flip those to off exactly once.
                if raw.get("notifications_migrated").is_none() {
                    if v.notifications {
                        eprintln!(
                            "settings: migrating notifications from on (old default) to off"
                        );
                    }
                    v.notifications = false;
                    v.notifications_migrated = true;
                    save_settings(dir, &v);
                }
                return v;
            }
        }
    }
    Settings::default()
}

fn save_settings(dir: &std::path::Path, s: &Settings) {
    let p = dir.join("settings.json");
    if let Ok(j) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(p, j);
    }
}

fn is_mock() -> bool {
    std::env::var("TARN_MOCK_AUTH").ok().as_deref() == Some("1")
}

fn mock_library() -> Library {
    Library {
        signed_in: true,
        mock: true,
        account: Some(models::AccountInfo {
            name: "Alex Stefan".into(),
            photo: None,
            handle: Some("@preview".into()),
            page_id: None,
        }),
        playlists: vec![
            models::PlaylistCard {
                id: "LM".into(),
                name: "Liked".into(),
                cover: None,
                track_count: Some(128),
                ..Default::default()
            },
            models::PlaylistCard {
                id: "PLmock1".into(),
                name: "Evening".into(),
                cover: None,
                track_count: Some(42),
                ..Default::default()
            },
        ],
        liked: Vec::new(),
        albums: Vec::new(),
        artists: Vec::new(),
        recents: Vec::new(),
    }
}

async fn restore_session(app: &App) {
    if is_mock() {
        return;
    }
    match app.store.load() {
        Ok(Some((header, kind))) => {
            if auth::session_ready_from_header(&header) {
                if let Err(e) = app.ytm.set_cookie_header(Some(header)).await {
                    eprintln!("restore session: {e}");
                    return;
                }
                let account = app.ytm.account().await.ok().flatten();
                let mut a = app.auth.lock();
                a.signed_in = true;
                a.mock = false;
                a.account = account;
                a.store = Some(match kind {
                    auth::StoreKind::Keyring => "keyring".into(),
                    auth::StoreKind::EncryptedFile => "encrypted_file".into(),
                });
            }
        }
        Ok(None) => {}
        Err(e) => eprintln!("session load: {e}"),
    }
}

#[tauri::command]
async fn search_tracks(app: State<'_, Arc<App>>, query: String) -> Result<Vec<Track>, String> {
    app.ytm.search_tracks(&query).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn search_suggestions(app: State<'_, Arc<App>>, query: String) -> Result<Vec<String>, String> {
    app.ytm
        .search_suggestions(&query)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn search_all(app: State<'_, Arc<App>>, query: String) -> Result<SearchResults, String> {
    app.ytm.search_all(&query).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_home(
    app: State<'_, Arc<App>>,
    params: Option<String>,
) -> Result<Home, String> {
    app.ytm
        .home(params.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_explore(app: State<'_, Arc<App>>) -> Result<Explore, String> {
    app.ytm.explore().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_genre(app: State<'_, Arc<App>>, id: String) -> Result<GenrePage, String> {
    app.ytm.genre(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_charts(app: State<'_, Arc<App>>) -> Result<Home, String> {
    app.ytm.charts_page().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_new_releases(app: State<'_, Arc<App>>) -> Result<Home, String> {
    app.ytm.new_releases().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_artist(app: State<'_, Arc<App>>, id: String) -> Result<ArtistPage, String> {
    app.ytm.artist(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_album(app: State<'_, Arc<App>>, id: String) -> Result<Collection, String> {
    app.ytm.album(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_playlist(app: State<'_, Arc<App>>, id: String) -> Result<Collection, String> {
    app.ytm.playlist(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_lyrics(app: State<'_, Arc<App>>, id: String) -> Result<Lyrics, String> {
    app.ytm.lyrics(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_related(app: State<'_, Arc<App>>, id: String) -> Result<Related, String> {
    app.ytm.related(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_radio(app: State<'_, Arc<App>>, id: String) -> Result<(), String> {
    let snap = app.player.snapshot();
    let seed = snap.track.clone().filter(|t| t.id == id);
    let tracks = app.ytm.radio(&id).await.map_err(|e| e.to_string())?;
    let tracks = crate::ytm::seed_first(tracks, seed, &id);
    if tracks.is_empty() {
        return Err("No radio tracks".into());
    }
    // Already playing this seed: replace the queue without restarting.
    if snap.track.as_ref().map(|t| t.id.as_str()) == Some(id.as_str())
        && (snap.playing || snap.loading)
    {
        app.player.replace_queue(tracks.clone(), 0);
        prefetch_ahead(&app, &tracks, 0);
        return Ok(());
    }
    let t = tracks[0].clone();
    start_playback(&app, t, tracks, 0).await
}

#[tauri::command]
async fn start_artist_radio(app: State<'_, Arc<App>>, radio_id: String) -> Result<(), String> {
    let tracks = app
        .ytm
        .radio_id(&radio_id)
        .await
        .map_err(|e| e.to_string())?;
    let tracks = crate::ytm::seed_first(tracks, None, "");
    if tracks.is_empty() {
        return Err("No radio tracks".into());
    }
    let t = tracks[0].clone();
    start_playback(&app, t, tracks, 0).await
}

#[tauri::command]
async fn play(
    app: State<'_, Arc<App>>,
    track: Track,
    queue: Vec<Track>,
    index: usize,
) -> Result<(), String> {
    start_playback(&app, track, queue, index).await
}

pub(crate) async fn start_playback(
    app: &App,
    track: Track,
    queue: Vec<Track>,
    index: usize,
) -> Result<(), String> {
    start_playback_at(app, track, queue, index, 0.0, false).await
}

pub(crate) async fn start_playback_at(
    app: &App,
    track: Track,
    queue: Vec<Track>,
    index: usize,
    start: f64,
    paused: bool,
) -> Result<(), String> {
    let id = track.id.clone();
    let cover = track.cover.clone();
    let snap = app.player.snapshot();
    let same = snap
        .track
        .as_ref()
        .is_some_and(|t| t.id == track.id);
    // Mid-track re-resolve must not crossfade or cut into itself, and must
    // keep the current position (set_loading used to zero it).
    let style = if paused {
        crate::player::PlayStyle::Armed
    } else if same && start > 0.05 {
        crate::player::PlayStyle::Fresh
    } else if snap.playing && snap.track.is_some() {
        if app.player.should_crossfade(snap.track.as_ref(), &track) {
            crate::player::PlayStyle::Crossfade(app.player.crossfade_secs())
        } else {
            crate::player::PlayStyle::Cut
        }
    } else {
        crate::player::PlayStyle::Fresh
    };
    if let Some(d) = track.duration {
        app.player.set_duration(d as f64);
    }
    app.player.set_rating(false, false);
    app.player.prepare(track.clone(), queue.clone(), index, start, true);

    // Cover/accent is decorative — never block the play path on it.
    if let Some(url) = cover {
        let http = app.http.clone();
        let player = app.player.clone();
        tauri::async_runtime::spawn(async move {
            if let Ok(resp) = http.get(&url).send().await {
                if let Ok(bytes) = resp.bytes().await {
                    if let Some(rgb) = color::accent_from_bytes(&bytes) {
                        player.set_accent(Some(rgb));
                    }
                }
            }
        });
    }

    match app.ytm.stream(&id).await {
        Ok(stream) => {
            if let Some(d) = stream.duration {
                app.player.set_duration(d);
            }
            match app.ytm.start_audio(&id, &stream).await {
                Ok(audio) => {
                    let r = app
                        .player
                        .play_stream(stream, audio, start, style)
                        .map_err(|e| e.to_string());
                    if r.is_ok() {
                        app.skip_fails.store(0, std::sync::atomic::Ordering::Relaxed);
                    }
                    prefetch_ahead(app, &queue, index);
                    r
                }
                Err(e) => {
                    let msg = short_err(&e.to_string());
                    playback_failed(app, &track, msg, paused, start).await
                }
            }
        }
        Err(e) => {
            let msg = short_err(&e.to_string());
            playback_failed(app, &track, msg, paused, start).await
        }
    }
}

/// After every client and yt-dlp path has failed, skip to the next track
/// with a subtle toast instead of leaving "Can't play this track" stuck
/// on the current song. Stop after three consecutive skips.
async fn playback_failed(
    app: &App,
    track: &Track,
    msg: String,
    paused: bool,
    start: f64,
) -> Result<(), String> {
    // Don't walk the queue on a paused restore / mid-track recover.
    if paused && start > 0.05 {
        app.player.set_error(msg.clone());
        return Err(msg);
    }
    let n = app
        .skip_fails
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        + 1;
    let title = track.title.clone();
    let failed_id = track.id.clone();
    eprintln!("playback failed ({n}): {msg} — {title}");
    if n > 3 {
        app.player.set_error(msg.clone());
        return Err(msg);
    }
    *app.pending_toast.lock() = Some(format!("Couldn't play {title}, skipping"));
    match Box::pin(advance(app, false)).await {
        Ok(()) => {
            let still = app.player.snapshot();
            if still.track.as_ref().is_some_and(|t| t.id == failed_id) {
                app.player.set_error(msg.clone());
                Err(msg)
            } else {
                Ok(())
            }
        }
        Err(_) => {
            app.player.set_error(msg.clone());
            Err(msg)
        }
    }
}


async fn restore_playback(app: &App) {
    let Some(sess) = persist::load(app.store.data_dir()) else {
        return;
    };
    {
        let mut ui = app.ui.lock();
        if ui.view.is_none() {
            *ui = persist::UiMem {
                view: sess.view.clone(),
                search_q: sess.search_q.clone(),
                scroll: sess.scroll,
            };
        }
    }
    persist::apply_player_prefs(app, &sess);
    if sess.queue.is_empty() {
        return;
    }
    let idx = sess.index.min(sess.queue.len() - 1);
    let track = sess.queue[idx].clone();
    let queue = sess.queue.clone();
    let pos = {
        let dur = track.duration.map(|d| d as f64).unwrap_or(0.0);
        let p = sess.position.max(0.0);
        if dur > 1.0 {
            p.min(dur - 0.4).max(0.0)
        } else {
            p
        }
    };
    if let Err(e) = start_playback_at(app, track, queue, idx, pos, true).await {
        eprintln!("restore playback: {e}");
    }
}

fn prefetch_ahead(app: &App, queue: &[Track], index: usize) {
    let ids: Vec<String> = queue
        .iter()
        .skip(index + 1)
        .take(3)
        .map(|t| t.id.clone())
        .filter(|id| !id.is_empty())
        .collect();
    let remaining = queue.len().saturating_sub(index + 1);
    let ytm = app.ytm.clone();
    let player = app.player.clone();
    tauri::async_runtime::spawn(async move {
        // Don't steal bandwidth from a starving current track.
        for _ in 0..40 {
            if !player.is_buffering() {
                let st = player.snapshot();
                if st.buffered >= st.position + 10.0
                    || (st.duration > 0.0 && st.buffered + 0.4 >= st.duration)
                    || st.track.is_none()
                {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        for id in &ids {
            if player.is_buffering() {
                break;
            }
            ytm.prefetch(id).await;
        }
        if remaining < 3 {
            if let Ok(more) = ytm.radio_continue().await {
                if !more.is_empty() {
                    player.append_unique(more.clone());
                    for t in more.iter().take(3) {
                        ytm.prefetch(&t.id).await;
                    }
                }
            }
        }
    });
}

#[tauri::command]
async fn prefetch(app: State<'_, Arc<App>>, id: String) -> Result<(), String> {
    if id.is_empty() {
        return Ok(());
    }
    let ytm = app.ytm.clone();
    tauri::async_runtime::spawn(async move {
        ytm.prefetch(&id).await;
    });
    Ok(())
}

fn short_err(s: &str) -> String {
    friendly_err(s, "Can't play this track")
}

fn friendly_err(s: &str, fallback: &str) -> String {
    let s = s.trim();
    let low = s.to_lowercase();
    if s.is_empty() {
        return fallback.into();
    }
    if low.contains("401") || low.contains("unauthorized") || low.contains("not logged") {
        return "Sign in to continue".into();
    }
    if low.contains("403") || low.contains("forbidden") {
        // googlevideo 403s are expired/throttled URLs, not a permission error.
        if low.contains("audio") || low.contains("download") || low.contains("stream") {
            return "Can't play this track".into();
        }
        return "You don't have access to this.".into();
    }
    if low.contains("404") || low.contains("not found") {
        return "We couldn't find that.".into();
    }
    if low.contains("timed out")
        || low.contains("timeout")
        || low.contains("connection")
        || low.contains("network")
        || low.contains("dns")
    {
        return "Can't reach YouTube right now.".into();
    }
    if low.contains("innertube")
        || low.contains("http ")
        || low.contains("status code")
        || low.contains("reqwest")
        || low.contains("os error")
        || s.len() > 120
    {
        return fallback.into();
    }
    s.to_string()
}

#[tauri::command]
fn pause(app: State<Arc<App>>) {
    app.player.pause();
}

#[tauri::command]
fn resume(app: State<Arc<App>>) {
    app.player.resume();
}

#[tauri::command]
fn toggle(app: State<Arc<App>>) {
    app.player.toggle();
}

#[tauri::command]
fn seek(app: State<Arc<App>>, seconds: f64) {
    app.player.seek(seconds);
}

#[tauri::command]
fn set_volume(app: State<Arc<App>>, volume: f32) {
    app.player.set_volume(volume);
}

#[tauri::command]
fn get_state(app: State<Arc<App>>) -> PlayerState {
    app.player.snapshot()
}

#[tauri::command]
fn set_shuffle(app: State<Arc<App>>, on: bool) {
    app.player.set_shuffle(on);
}

#[tauri::command]
fn set_repeat(app: State<Arc<App>>, mode: String) {
    app.player.set_repeat(&mode);
}

#[tauri::command]
fn cycle_repeat(app: State<Arc<App>>) -> String {
    app.player.cycle_repeat()
}

#[tauri::command]
fn queue_add(app: State<Arc<App>>, tracks: Vec<Track>, next: bool) {
    if next {
        app.player.insert_next(tracks);
    } else {
        app.player.append(tracks);
    }
    let (q, i) = app.player.queue_and_index();
    prefetch_ahead(&app, &q, i);
}

#[tauri::command]
fn queue_remove(app: State<Arc<App>>, index: usize) {
    app.player.remove_at(index);
    let (q, i) = app.player.queue_and_index();
    prefetch_ahead(&app, &q, i);
}

#[tauri::command]
fn queue_move(app: State<Arc<App>>, from: usize, to: usize) {
    app.player.move_item(from, to);
    let (q, i) = app.player.queue_and_index();
    prefetch_ahead(&app, &q, i);
}

#[tauri::command]
async fn next(app: State<'_, Arc<App>>) -> Result<(), String> {
    advance(&app, true).await
}

#[tauri::command]
async fn prev(app: State<'_, Arc<App>>) -> Result<(), String> {
    match app.player.prev_choice() {
        NextChoice::SeekZero => {
            app.player.seek(0.0);
            Ok(())
        }
        NextChoice::Play(i) => {
            let (queue, _) = app.player.queue_and_index();
            if let Some(t) = queue.get(i).cloned() {
                start_playback(&app, t, queue, i).await
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

pub(crate) async fn advance(app: &App, user: bool) -> Result<(), String> {
    match app.player.next_choice(user) {
        NextChoice::Play(i) => {
            let (queue, _) = app.player.queue_and_index();
            if let Some(t) = queue.get(i).cloned() {
                start_playback(app, t, queue, i).await
            } else {
                Ok(())
            }
        }
        NextChoice::Autoplay => {
            let (mut queue, index) = app.player.queue_and_index();
            let id = queue.get(index).map(|t| t.id.clone());
            let Some(id) = id else {
                return Ok(());
            };
            let more = match app.ytm.radio_continue().await {
                Ok(m) if !m.is_empty() => m,
                _ => match app.ytm.radio(&id).await {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("autoplay radio: {e}");
                        Vec::new()
                    }
                },
            };
            let existing: std::collections::HashSet<_> =
                queue.iter().map(|t| t.id.clone()).collect();
            for t in more {
                if !t.id.is_empty() && !existing.contains(&t.id) {
                    queue.push(t);
                }
            }
            let i = index + 1;
            if i < queue.len() {
                let t = queue[i].clone();
                start_playback(app, t, queue, i).await
            } else {
                Ok(())
            }
        }
        NextChoice::SeekZero => {
            app.player.seek(0.0);
            Ok(())
        }
        NextChoice::Stop => Ok(()),
    }
}

#[tauri::command]
async fn like_track(app: State<'_, Arc<App>>, id: String, action: String) -> Result<LikeState, String> {
    if is_mock() {
        let st = match action.as_str() {
            "like" => LikeState {
                liked: true,
                disliked: false,
            },
            "dislike" => LikeState {
                liked: false,
                disliked: true,
            },
            _ => LikeState::default(),
        };
        app.player.set_rating(st.liked, st.disliked);
        return Ok(st);
    }
    let st = app
        .ytm
        .like(&id, &action)
        .await
        .map_err(|e| e.to_string())?;
    app.player.set_rating(st.liked, st.disliked);
    Ok(st)
}

#[tauri::command]
async fn subscribe_artist(
    app: State<'_, Arc<App>>,
    id: String,
    on: bool,
) -> Result<(), String> {
    app.ytm.subscribe(&id, on).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn playlist_create(app: State<'_, Arc<App>>, title: String) -> Result<String, String> {
    app.ytm.playlist_create(&title).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn playlist_add(
    app: State<'_, Arc<App>>,
    playlist_id: String,
    video_id: String,
) -> Result<(), String> {
    app.ytm
        .playlist_add(&playlist_id, &video_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn playlist_remove(
    app: State<'_, Arc<App>>,
    playlist_id: String,
    video_id: String,
) -> Result<(), String> {
    app.ytm
        .playlist_remove(&playlist_id, &video_id, None)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn playlist_rename(
    app: State<'_, Arc<App>>,
    playlist_id: String,
    title: String,
) -> Result<(), String> {
    app.ytm
        .playlist_rename(&playlist_id, &title)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn library_save(
    app: State<'_, Arc<App>>,
    id: String,
    save: bool,
) -> Result<(), String> {
    app.ytm.library_save(&id, save).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_library(app: State<'_, Arc<App>>) -> Result<Library, String> {
    if is_mock() {
        return Ok(mock_library());
    }
    app.ytm.library().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_history(app: State<'_, Arc<App>>) -> Result<Vec<Track>, String> {
    app.ytm.history().await.map_err(|e| e.to_string())
}

#[tauri::command]
fn auth_status(app: State<Arc<App>>) -> AuthStatus {
    app.auth.lock().clone()
}

#[tauri::command]
async fn open_login(app: State<'_, Arc<App>>, handle: AppHandle) -> Result<(), String> {
    if is_mock() {
        return Ok(());
    }
    let app_ref = app.inner().clone();
    login::open_login(handle, move |header| {
        let app_ref = app_ref.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        tauri::async_runtime::spawn(async move {
            let r = apply_session(&app_ref, header)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(r);
        });
        match rx.recv_timeout(std::time::Duration::from_secs(25)) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("timed out applying session".into()),
        }
    })
    .map_err(|e| e.to_string())
}

async fn apply_session(app: &App, header: String) -> anyhow::Result<()> {
    crate::diag::log(format!(
        "apply_session: cookies={:?} complete={}",
        crate::auth::header_cookie_names(&header),
        crate::auth::session_complete_from_header(&header)
    ));
    let kind = app.store.save(&header)?;
    crate::diag::log(format!("apply_session: stored as {kind:?}"));
    app.ytm.set_cookie_header(Some(header)).await?;
    let account = app.ytm.account().await.ok().flatten();
    crate::diag::log(format!(
        "apply_session: account={}",
        account.as_ref().map(|a| a.name.as_str()).unwrap_or("(none)")
    ));
    let mut a = app.auth.lock();
    a.signed_in = true;
    a.mock = false;
    a.account = account;
    a.store = Some(match kind {
        auth::StoreKind::Keyring => "keyring".into(),
        auth::StoreKind::EncryptedFile => "encrypted_file".into(),
    });
    Ok(())
}

#[tauri::command]
fn diagnostics(app: State<Arc<App>>) -> String {
    let auth = app.auth.lock().clone();
    let cookie = app.ytm.cookie_header();
    let names = cookie
        .as_deref()
        .map(crate::auth::header_cookie_names)
        .unwrap_or_default();
    let ready = cookie
        .as_deref()
        .is_some_and(crate::auth::session_ready_from_header);
    let complete = cookie
        .as_deref()
        .is_some_and(crate::auth::session_complete_from_header);
    let mut out = String::new();
    out.push_str(&format!(
        "tarn diagnostics\nsigned_in={} mock={} store={:?}\naccount={} handle={}\ncookie_names={:?}\nsession_ready={} session_complete={}\n\n",
        auth.signed_in,
        auth.mock,
        auth.store,
        auth.account.as_ref().map(|a| a.name.as_str()).unwrap_or("-"),
        auth.account.as_ref().and_then(|a| a.handle.clone()).unwrap_or_default(),
        names,
        ready,
        complete,
    ));
    out.push_str(&crate::diag::dump());
    out
}

#[tauri::command]
async fn sign_out(app: State<'_, Arc<App>>) -> Result<(), String> {
    let _ = app.store.clear();
    let _ = app.ytm.set_cookie_header(None).await;
    *app.auth.lock() = AuthStatus::default();
    Ok(())
}

#[tauri::command]
fn get_settings(app: State<Arc<App>>) -> Settings {
    app.settings.lock().clone()
}

#[tauri::command]
fn set_settings(app: State<Arc<App>>, settings: Settings) {
    let mut settings = settings;
    // Keep the one-shot flag so a later load does not flip the user off again.
    settings.notifications_migrated = true;
    app.ytm.apply_settings(&settings);
    *app.notify.lock() = settings.notifications;
    app.player.set_autoplay(settings.autoplay);
    app.player.set_crossfade(
        settings.crossfade,
        settings.crossfade_seconds,
        settings.album_continuity,
    );
    save_settings(app.store.data_dir(), &settings);
    *app.settings.lock() = settings;
}


#[tauri::command]
fn remember_ui(app: State<Arc<App>>, ui: persist::UiMem) {
    *app.ui.lock() = ui;
}

#[tauri::command]
fn saved_session(app: State<Arc<App>>) -> Option<persist::SavedSession> {
    let mut sess = persist::load(app.store.data_dir());
    if let Some(s) = sess.as_mut() {
        let ui = app.ui.lock();
        if s.view.is_none() {
            s.view = ui.view.clone();
            s.search_q = ui.search_q.clone();
            s.scroll = ui.scroll;
        }
    }
    sess
}

#[tauri::command]
fn save_session_now(handle: AppHandle) {
    persist::save(&handle);
}

#[tauri::command]
fn cache_stats(app: State<Arc<App>>) -> (u64, u64) {
    (app.ytm.cache.used_bytes(), app.ytm.cache.max_bytes())
}

#[tauri::command]
async fn extract_cookie_names(handle: AppHandle) -> Result<Vec<String>, String> {
    login::extract_public_cookie_names(handle)
        .await
        .map_err(|e| e.to_string())
}


fn main_hwnd(tauri_app: &tauri::App) -> crate::mpris::WindowHandle {
    #[cfg(windows)]
    {
        use tauri::Manager;
        let Some(win) = tauri_app.get_webview_window("main") else {
            eprintln!("media controls: no main window for HWND");
            return None;
        };
        match win.hwnd() {
            Ok(h) => Some(h.0 as usize),
            Err(e) => {
                eprintln!("media controls: hwnd: {e}");
                None
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = tauri_app;
        None
    }
}


fn maybe_blend(app: &Arc<App>) {
    let Some((track, index, style)) = app.player.blend_due() else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let id = track.id.clone();
        let result = async {
            let stream = app.ytm.stream(&id).await.map_err(|e| e.to_string())?;
            let next_dur = stream.duration;
            let audio = app
                .ytm
                .start_audio(&id, &stream)
                .await
                .map_err(|e| e.to_string())?;
            app.player
                .play_stream(stream, audio, 0.0, style)
                .map_err(|e| e.to_string())?;
            app.player.adopt_track(track, index);
            // Set the new track's duration only AFTER adopt, so we never
            // shrink the outgoing track's duration mid-play (that made
            // the same song look like it restarted).
            if let Some(d) = next_dur {
                app.player.set_duration(d);
            }
            let (q, _) = app.player.queue_and_index();
            prefetch_ahead(&app, &q, index);
            Ok::<(), String>(())
        }
        .await;
        if let Err(e) = result {
            eprintln!("blend: {e}");
            app.player.clear_blending();
        }
    });
}

fn emit_loop(handle: AppHandle, app: Arc<App>) {
    std::thread::spawn(move || {
        let mut last_save = std::time::Instant::now();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if let Some(header) = app.ytm.harvest_cookies() {
                if let Err(e) = app.store.save(&header) {
                    eprintln!("session persist (rotated cookies): {e}");
                }
            }
            maybe_blend(&app);
            if let Some((track, queue, index, pos, play)) = app.player.take_recover() {
                let app2 = app.clone();
                let handle2 = handle.clone();
                tauri::async_runtime::spawn(async move {
                    eprintln!(
                        "recover: re-resolving {} at {:.2}s (play={play})",
                        track.id, pos
                    );
                    app2.ytm.invalidate(&track.id);
                    let r = start_playback_at(&app2, track, queue, index, pos, !play).await;
                    app2.player.recover_finished();
                    if let Err(e) = r {
                        eprintln!("recover failed: {e}; not advancing");
                    }
                    let _ = handle2.emit("player", app2.player.snapshot());
                });
            }
            if app.player.take_ended() {
                let app2 = app.clone();
                let handle2 = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = advance(&app2, false).await;
                    let _ = handle2.emit("player", app2.player.snapshot());
                });
            }
            if let Some(msg) = app.pending_toast.lock().take() {
                let _ = handle.emit("toast", msg);
            }
            let st = app.player.snapshot();
            let _ = handle.emit("player", st);
            if last_save.elapsed() >= std::time::Duration::from_secs(3) {
                persist::save(&handle);
                last_save = std::time::Instant::now();
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    crate::win::init();
    // WebKitGTK compositing often blurs glyphs at fractional DPI.
    if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }
    let app = match App::new() {
        Ok(a) => Arc::new(a),
        Err(e) => {
            eprintln!("failed to start: {e}");
            std::process::exit(1);
        }
    };

    let app_for_setup = app.clone();
    let mut builder = tauri::Builder::default().manage(app);
    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_process::init());
    }
    builder
        .setup(move |tauri_app| {
            let handle = tauri_app.handle().clone();
            let app = app_for_setup.clone();
            if let Some(win) = tauri_app.get_webview_window("main") {
                if let Some(sess) = persist::load(app.store.data_dir()) {
                    if let Some(g) = sess.window.as_ref() {
                        persist::apply_window(&win, g);
                    }
                    {
                        let mut ui = app.ui.lock();
                        *ui = persist::UiMem {
                            view: sess.view,
                            search_q: sess.search_q,
                            scroll: sess.scroll,
                        };
                    }
                }
                let _ = win.show();
                let _ = win.set_focus();
            }
            let app_restore = app.clone();
            tauri::async_runtime::spawn(async move {
                restore_session(&app_restore).await;
                restore_playback(&app_restore).await;
            });
            tauri::async_runtime::spawn(async {
                crate::ytdlp::maybe_update().await;
            });
            emit_loop(handle.clone(), app.clone());
            let hwnd = main_hwnd(tauri_app);
            mpris::start(app.clone(), handle.clone(), hwnd);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            search_tracks,
            search_suggestions,
            search_all,
            get_home,
            get_explore,
            get_genre,
            get_charts,
            get_new_releases,
            get_artist,
            get_album,
            get_playlist,
            get_lyrics,
            get_related,
            start_radio,
            start_artist_radio,
            play,
            pause,
            resume,
            toggle,
            seek,
            set_volume,
            get_state,
            set_shuffle,
            set_repeat,
            cycle_repeat,
            queue_add,
            queue_remove,
            queue_move,
            next,
            prev,
            like_track,
            subscribe_artist,
            playlist_create,
            playlist_add,
            playlist_remove,
            playlist_rename,
            library_save,
            get_library,
            get_history,
            auth_status,
            open_login,
            sign_out,
            diagnostics,
            get_settings,
            set_settings,
            extract_cookie_names,
            prefetch,
            cache_stats,
            remember_ui,
            saved_session,
            save_session_now,
        ])
        .build(tauri::generate_context!())
        .expect("tarn failed to start")
        .run(|handle, event| {
            use tauri::{RunEvent, WindowEvent};
            match event {
                RunEvent::WindowEvent { label, event, .. } if label == "main" => {
                    if matches!(
                        event,
                        WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
                    ) {
                        persist::save(handle);
                    }
                }
                RunEvent::ExitRequested { .. } | RunEvent::Exit => {
                    persist::save(handle);
                }
                _ => {}
            }
        });
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "tarn-settings-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn fresh_profile_notifications_off() {
        let dir = tmp_dir("fresh");
        let s = load_settings(&dir);
        assert!(!s.notifications);
        assert!(s.notifications_migrated);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn old_default_on_migrates_once() {
        let dir = tmp_dir("old-on");
        // A settings.json written by a build that defaulted notifications to on,
        // without the migration flag.
        fs::write(
            dir.join("settings.json"),
            r#"{
                "quality": "high",
                "language": "en",
                "region": "US",
                "notifications": true,
                "autoplay": true,
                "cache_max_mb": 1024,
                "crossfade": true,
                "crossfade_seconds": 4.0,
                "album_continuity": true
            }"#,
        )
        .unwrap();
        let s = load_settings(&dir);
        assert!(!s.notifications, "old default on must flip to off");
        assert!(s.notifications_migrated);
        // User turns them back on; a later load must not flip again.
        let mut on = s.clone();
        on.notifications = true;
        save_settings(&dir, &on);
        let again = load_settings(&dir);
        assert!(again.notifications, "must not re-migrate after the one-shot");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn already_off_still_gets_the_flag() {
        let dir = tmp_dir("old-off");
        fs::write(
            dir.join("settings.json"),
            r#"{
                "quality": "high",
                "language": "en",
                "region": "US",
                "notifications": false,
                "autoplay": true,
                "cache_max_mb": 1024,
                "crossfade": true,
                "crossfade_seconds": 4.0,
                "album_continuity": true
            }"#,
        )
        .unwrap();
        let s = load_settings(&dir);
        assert!(!s.notifications);
        assert!(s.notifications_migrated);
        let _ = fs::remove_dir_all(dir);
    }
}
