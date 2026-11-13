mod auth;
mod color;
mod innertube;
mod login;
mod mpris;
mod models;
mod player;
mod ytm;

use std::sync::Arc;

use models::{
    ArtistPage, AuthStatus, Collection, Explore, GenrePage, Home, Library, LikeState, Lyrics,
    PlayerState, Related, SearchResults, Settings, Track,
};
use parking_lot::Mutex;
use player::{NextChoice, Player};
use tauri::{AppHandle, Emitter, Listener, Manager, State};
use ytm::Ytm;

use crate::auth::SessionStore;

pub struct App {
    ytm: Ytm,
    pub player: Player,
    http: reqwest::Client,
    store: SessionStore,
    settings: Mutex<Settings>,
    auth: Mutex<AuthStatus>,
    notify: Mutex<bool>,
}

impl App {
    fn new() -> anyhow::Result<Self> {
        let store = SessionStore::new()?;
        let settings = load_settings(store.data_dir());
        let ytm = Ytm::new()?;
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
                }),
                store: Some("mock".into()),
            }
        } else {
            AuthStatus::default()
        };
        Ok(Self {
            ytm,
            player: Player::start(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(12))
                .build()?,
            store,
            notify: Mutex::new(settings.notifications),
            settings: Mutex::new(settings),
            auth: Mutex::new(auth),
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
    None
}

fn load_settings(dir: &std::path::Path) -> Settings {
    let p = dir.join("settings.json");
    if let Ok(s) = std::fs::read_to_string(p) {
        if let Ok(v) = serde_json::from_str::<Settings>(&s) {
            return v;
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
    let tracks = app.ytm.radio(&id).await.map_err(|e| e.to_string())?;
    if tracks.is_empty() {
        return Err("No radio tracks".into());
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

async fn start_playback(
    app: &App,
    track: Track,
    queue: Vec<Track>,
    index: usize,
) -> Result<(), String> {
    let id = track.id.clone();
    let cover = track.cover.clone();
    if let Some(d) = track.duration {
        app.player.set_duration(d as f64);
    }
    app.player.set_rating(false, false);
    app.player.set_loading(track, queue, index);

    if let Some(url) = cover {
        let http = app.http.clone();
        if let Ok(resp) = http.get(&url).send().await {
            if let Ok(bytes) = resp.bytes().await {
                if let Some(rgb) = color::accent_from_bytes(&bytes) {
                    app.player.set_accent(Some(rgb));
                }
            }
        }
    }

    match app.ytm.stream(&id).await {
        Ok(stream) => {
            if let Some(d) = stream.duration {
                app.player.set_duration(d);
            }
            app.player.play_stream(stream, 0.0).map_err(|e| e.to_string())
        }
        Err(e) => {
            let msg = short_err(&e.to_string());
            app.player.set_error(msg.clone());
            Err(msg)
        }
    }
}

fn short_err(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() || s.len() > 160 {
        "Can't play this track".into()
    } else {
        s.to_string()
    }
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
}

#[tauri::command]
fn queue_remove(app: State<Arc<App>>, index: usize) {
    app.player.remove_at(index);
}

#[tauri::command]
fn queue_move(app: State<Arc<App>>, from: usize, to: usize) {
    app.player.move_item(from, to);
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

async fn advance(app: &App, user: bool) -> Result<(), String> {
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
            match app.ytm.radio(&id).await {
                Ok(more) => {
                    let existing: std::collections::HashSet<_> =
                        queue.iter().map(|t| t.id.clone()).collect();
                    for t in more {
                        if !existing.contains(&t.id) {
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
                Err(e) => {
                    eprintln!("autoplay radio: {e}");
                    Ok(())
                }
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
        tauri::async_runtime::spawn(async move {
            if let Err(e) = apply_session(&app_ref, header).await {
                eprintln!("apply session: {e}");
            }
        });
    })
    .map_err(|e| e.to_string())
}

async fn apply_session(app: &App, header: String) -> anyhow::Result<()> {
    let kind = app.store.save(&header)?;
    app.ytm.set_cookie_header(Some(header)).await?;
    let account = app.ytm.account().await.ok().flatten();
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
    app.ytm.apply_settings(&settings);
    *app.notify.lock() = settings.notifications;
    app.player.set_autoplay(settings.autoplay);
    save_settings(app.store.data_dir(), &settings);
    *app.settings.lock() = settings;
}

#[tauri::command]
async fn extract_cookie_names(handle: AppHandle) -> Result<Vec<String>, String> {
    login::extract_public_cookie_names(handle)
        .await
        .map_err(|e| e.to_string())
}

fn emit_loop(handle: AppHandle, app: Arc<App>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if app.player.take_ended() {
                let app2 = app.clone();
                let handle2 = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = advance(&app2, false).await;
                    let _ = handle2.emit("player", app2.player.snapshot());
                });
            }
            let st = app.player.snapshot();
            let _ = handle.emit("player", st);
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
    tauri::Builder::default()
        .manage(app)
        .setup(move |tauri_app| {
            let handle = tauri_app.handle().clone();
            let app = app_for_setup.clone();
            let app_restore = app.clone();
            tauri::async_runtime::spawn(async move {
                restore_session(&app_restore).await;
            });
            emit_loop(handle.clone(), app.clone());
            mpris::start(app.clone(), handle.clone());
            // Media keys from MPRIS come back as "media-cmd"
            let app_cmd = app.clone();
            handle.listen("media-cmd", move |ev| {
                let p = ev.payload();
                let app_cmd = app_cmd.clone();
                if p.contains("next") {
                    tauri::async_runtime::spawn(async move {
                        let _ = advance(&app_cmd, true).await;
                    });
                } else if p.contains("prev") {
                    tauri::async_runtime::spawn(async move {
                        match app_cmd.player.prev_choice() {
                            NextChoice::SeekZero => app_cmd.player.seek(0.0),
                            NextChoice::Play(i) => {
                                let (q, _) = app_cmd.player.queue_and_index();
                                if let Some(t) = q.get(i).cloned() {
                                    let _ = start_playback(&app_cmd, t, q, i).await;
                                }
                            }
                            _ => {}
                        }
                    });
                }
            });
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
            get_settings,
            set_settings,
            extract_cookie_names,
        ])
        .run(tauri::generate_context!())
        .expect("tarn failed to start");
}
