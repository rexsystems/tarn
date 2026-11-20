mod auth;
mod diag;
mod cache;
mod offline;
mod color;
pub mod decode;
mod eq;
mod fx;
mod viz;
mod output;
mod innertube;
mod player_client;
mod login;
mod mix;
mod transition;
mod stats;
mod jumplist;
mod analysis;
mod lrclib;
mod bgvideo;
mod key;
mod autodj;
mod sponsorblock;
#[cfg(desktop)]
mod mpris;
#[cfg(desktop)]
mod discord;
#[cfg(desktop)]
mod tray;
#[cfg(windows)]
mod thumbbar;
#[cfg(target_os = "macos")]
mod dock;
#[cfg(desktop)]
mod remote;
mod jam;
// Phones: system media session / Discord presence are desktop-only for now.
#[cfg(target_os = "android")]
mod android_media;
#[cfg(target_os = "ios")]
mod ios_media;
#[cfg(target_os = "ios")]
mod ios_bridge;
#[cfg(mobile)]
mod mpris {
    pub type WindowHandle = Option<usize>;
    pub fn start(app: std::sync::Arc<crate::App>, handle: tauri::AppHandle, _: WindowHandle) {
        // Android: the Kotlin foreground service polls this for its notification.
        #[cfg(target_os = "android")]
        {
            let _ = handle;
            crate::android_media::start(app);
        }
        // iOS: lock-screen Now Playing + remote commands.
        #[cfg(target_os = "ios")]
        crate::ios_bridge::start(app, handle);
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let _ = (app, handle);
    }
}
#[cfg(mobile)]
mod discord {
    pub fn start(_: std::sync::Arc<crate::App>) {}
}
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

#[derive(serde::Serialize, Clone, Debug)]
pub struct NetState {
    /// The user switched offline mode on.
    pub mode: bool,
    /// The connection is down (detected).
    pub net_down: bool,
    /// Effective: mode || net_down.
    pub offline: bool,
}

pub struct App {
    ytm: std::sync::Arc<Ytm>,
    pub player: Player,
    http: reqwest::Client,
    store: SessionStore,
    settings: Mutex<Settings>,
    auth: Mutex<AuthStatus>,
    notify: Mutex<bool>,
    discord: Mutex<bool>,
    pub ui: Mutex<persist::UiMem>,
    pub pending_toast: Mutex<Option<String>>,
    skip_fails: std::sync::atomic::AtomicU32,
    /// Serialises library loads; callers that waited reuse the fresh result.
    library_slot: tokio::sync::Mutex<(u64, Option<Library>)>,
    library_calls: std::sync::atomic::AtomicU64,
    /// Serialises default Home loads (no chip params) so refresh spam shares one fetch.
    home_slot: tokio::sync::Mutex<(u64, Option<Home>)>,
    home_calls: std::sync::atomic::AtomicU64,
    /// Set when `restore_session` finishes (ok or empty) so Home doesn't race cookies.
    session_ready: std::sync::atomic::AtomicBool,
    /// Seed id currently being fetched for autoplay queue growth.
    autoplay_fetching: Mutex<Option<String>>,
    /// Video ids known to be in Liked Music (from library + like toggles).
    liked_ids: Mutex<std::collections::HashSet<String>>,
    /// Last library liked track list - used to open LM without rustypipe.
    liked_tracks: Mutex<Vec<Track>>,
    pub offline: std::sync::Arc<offline::Offline>,
    /// True while music.youtube.com is unreachable (checked in the background).
    net_down: std::sync::atomic::AtomicBool,
    /// Earliest time a failed blend may be retried while offline.
    blend_retry: Mutex<Option<std::time::Instant>>,
    /// Local listening history for the Stats page.
    pub stats: stats::Stats,
    /// Recently played tracks for the Windows taskbar Jump List.
    pub recent: jumplist::Recent,
    /// Per-track loudness / beat-grid cache.
    pub analysis: std::sync::Arc<analysis::Analyzer>,
    /// SponsorBlock "music_offtopic" segments (used when the setting is on).
    sponsorblock: std::sync::Arc<sponsorblock::SponsorBlock>,
}

impl App {
    fn new() -> anyhow::Result<Self> {
        let store = SessionStore::new()?;
        let settings = load_settings(store.data_dir());
        let ytm = std::sync::Arc::new(Ytm::new()?);
        ytm.apply_settings(&settings);
        ytm.set_offline(settings.offline_mode);
        let offline = offline::Offline::open(&ytm.cache);
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
            settings.crossfade_manual,
        );
        player.set_eq(settings.eq_bands, settings.eq_enabled);
        player.set_fx(settings.speed, settings.speed_keep_pitch, settings.reverb);
        output::set_prefs(settings.output_device.clone(), settings.output_auto_switch);
        if settings.output_device.is_some() {
            player.set_output_device(settings.output_device.clone());
        }
        player.set_transition(settings.transition());
        let analysis = analysis::Analyzer::open(store.data_dir());
        player.set_analyzer(analysis.clone());
        {
            let p = player.clone();
            analysis.on_done(Box::new(move |id, _| p.analysis_ready(id)));
        }
        player.set_normalize(settings.normalize, settings.normalize_target);
        player.set_auto_dj(settings.auto_dj);
        player.set_waveform(settings.seek_waveform);
        if let Some(sess) = persist::load(store.data_dir()) {
            // Volume / shuffle / repeat / queue metadata - audio is armed later.
            player.set_volume(sess.volume);
            player.set_shuffle(sess.shuffle);
            player.set_repeat(&sess.repeat);
        }
        let stats = stats::Stats::open(store.data_dir(), settings.listening_stats);
        let recent = jumplist::Recent::open(store.data_dir());
        // First run / empty file: fill from the local listen log.
        recent.seed_from_listens(&stats.listens(0));
        Ok(Self {
            ytm,
            player,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(12))
                .build()?,
            store,
            notify: Mutex::new(settings.notifications),
            discord: Mutex::new(settings.discord_presence),
            settings: Mutex::new(settings),
            auth: Mutex::new(auth),
            ui: Mutex::new(persist::UiMem::default()),
            pending_toast: Mutex::new(None),
            skip_fails: std::sync::atomic::AtomicU32::new(0),
            library_slot: tokio::sync::Mutex::new((0, None)),
            library_calls: std::sync::atomic::AtomicU64::new(0),
            home_slot: tokio::sync::Mutex::new((0, None)),
            home_calls: std::sync::atomic::AtomicU64::new(0),
            session_ready: std::sync::atomic::AtomicBool::new(is_mock()),
            autoplay_fetching: Mutex::new(None),
            liked_ids: Mutex::new(std::collections::HashSet::new()),
            liked_tracks: Mutex::new(Vec::new()),
            offline,
            net_down: std::sync::atomic::AtomicBool::new(false),
            blend_retry: Mutex::new(None),
            stats,
            recent,
            analysis,
            sponsorblock: sponsorblock::SponsorBlock::new(),
        })
    }

    /// Offline for any reason: the user's toggle or a dead connection.
    pub fn is_offline(&self) -> bool {
        self.ytm.is_offline()
    }

    /// Recompute the effective offline flag. Returns true if it changed.
    fn refresh_offline(&self) -> bool {
        let mode = self.settings.lock().offline_mode;
        let down = self.net_down.load(std::sync::atomic::Ordering::Relaxed);
        let was = self.ytm.is_offline();
        let now = mode || down;
        self.ytm.set_offline(now);
        if was && !now {
            self.offline.requeue_missing(&self.ytm.cache);
        }
        was != now
    }

    fn net_state(&self) -> NetState {
        NetState {
            mode: self.settings.lock().offline_mode,
            net_down: self.net_down.load(std::sync::atomic::Ordering::Relaxed),
            offline: self.ytm.is_offline(),
        }
    }

    pub fn notifications_enabled(&self) -> bool {
        *self.notify.lock()
    }

    pub fn discord_enabled(&self) -> bool {
        *self.discord.lock()
    }

    pub fn remember_liked(&self, ids: impl IntoIterator<Item = String>) {
        let mut g = self.liked_ids.lock();
        for id in ids {
            if !id.is_empty() {
                g.insert(id);
            }
        }
    }

    /// Replace the liked-id set from a full Liked Music snapshot.
    /// Insert-only refresh left unliked ids sticky for the whole session.
    /// An empty replace is ignored so a failed/partial library fetch cannot
    /// wipe every heart mid-play (that made likes flash then disappear).
    pub fn remember_liked_tracks(&self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }
        let mut g = self.liked_ids.lock();
        g.clear();
        for t in &tracks {
            if !t.id.is_empty() {
                g.insert(t.id.clone());
            }
        }
        drop(g);
        *self.liked_tracks.lock() = tracks;
    }

    pub fn clear_liked(&self) {
        self.liked_ids.lock().clear();
        self.liked_tracks.lock().clear();
    }

    pub fn liked_music_snapshot(&self) -> Option<Collection> {
        let tracks = self.liked_tracks.lock().clone();
        if tracks.is_empty() {
            return None;
        }
        let n = tracks.len() as u64;
        Some(Collection {
            kind: "playlist".into(),
            id: "LM".into(),
            playlist_id: Some("LM".into()),
            name: "Liked Music".into(),
            cover: tracks.first().and_then(|t| t.cover.clone()),
            artists: Vec::new(),
            artist_ids: Vec::new(),
            author: None,
            year: None,
            description: None,
            track_count: Some(n),
            album_type: None,
            tracks,
            related: Vec::new(),
            owned: true,
            saved: true,
        })
    }

    pub fn set_liked_id(&self, id: &str, liked: bool) {
        if id.is_empty() {
            return;
        }
        let mut g = self.liked_ids.lock();
        if liked {
            g.insert(id.to_string());
        } else {
            g.remove(id);
        }
    }

    pub fn is_liked_id(&self, id: &str) -> bool {
        !id.is_empty() && self.liked_ids.lock().contains(id)
    }

    pub fn apply_known_rating(&self, id: &str) {
        let liked = self.is_liked_id(id);
        self.player.set_rating(liked, false);
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
        app.session_ready
            .store(true, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    match app.store.load() {
        Ok(Some((header, kind))) => {
            if auth::session_ready_from_header(&header) {
                if let Err(e) = app.ytm.set_cookie_header(Some(header)).await {
                    eprintln!("restore session: {e}");
                } else {
                    let account = app.ytm.account().await.ok().flatten();
                    if account.is_none() {
                        eprintln!("restore session: account menu looks logged out; clearing");
                        let _ = app.ytm.set_cookie_header(None).await;
                        let _ = app.store.clear();
                    } else {
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
            }
        }
        Ok(None) => {}
        Err(e) => eprintln!("session load: {e}"),
    }
    app.session_ready
        .store(true, std::sync::atomic::Ordering::SeqCst);
}

async fn wait_session_ready(app: &App) {
    for _ in 0..100 {
        if app
            .session_ready
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
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
    handle: tauri::AppHandle,
    params: Option<String>,
) -> Result<Home, String> {
    // Chip filters are one-shot; default Home is single-flight + disk-cached.
    if params.is_some() {
        wait_session_ready(&app).await;
        return app
            .ytm
            .home(params.as_deref(), |_| {})
            .await
            .map_err(|e| e.to_string());
    }

    // Don't race session restore - that was painting a logged-out 3-shelf Home
    // over the personalized cache (Listen again / New releases / …).
    wait_session_ready(&app).await;
    for _ in 0..30 {
        let signed = app.auth.lock().signed_in;
        let cookies = app.ytm.cookie_header().is_some();
        if cookies || !signed {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    }

    let ticket = app
        .home_calls
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut slot = app.home_slot.lock().await;
    let home = match &slot.1 {
        Some(h) if slot.0 > ticket && home_feed_ok(h) => h.clone(),
        _ => {
            let handle2 = handle.clone();
            let h = app
                .ytm
                .home(None, move |partial| {
                    if home_feed_ok(partial) {
                        let _ = handle2.emit("home-partial", partial.clone());
                    }
                })
                .await
                .map_err(|e| e.to_string())?;
            slot.0 = app
                .home_calls
                .load(std::sync::atomic::Ordering::SeqCst);
            slot.1 = Some(h.clone());
            save_home_cache(&app, &h);
            h
        }
    };
    // Mix-art placeholders need a few `next` lookups - do them after the UI
    // already has Home, then push a partial with real covers.
    if home.signed_in {
        let app2 = app.inner().clone();
        let handle2 = handle.clone();
        let mut home2 = home.clone();
        tauri::async_runtime::spawn(async move {
            app2.ytm.fill_mix_covers(&mut home2).await;
            if home_feed_ok(&home2) {
                {
                    let mut slot = app2.home_slot.lock().await;
                    if let Some(cur) = slot.1.as_mut() {
                        // Keep shelf structure; only patch covers we resolved.
                        for (i, sh) in home2.shelves.iter().enumerate() {
                            if let Some(dst) = cur.shelves.get_mut(i) {
                                for (j, it) in sh.items.iter().enumerate() {
                                    if let Some(slot_it) = dst.items.get_mut(j) {
                                        if slot_it.id == it.id {
                                            if let Some(c) = it.cover.clone() {
                                                slot_it.cover = Some(c);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        home2 = cur.clone();
                    }
                }
                save_home_cache(&app2, &home2);
                let _ = handle2.emit("home-partial", home2);
            }
        });
    }
    Ok(home)
}

/// Personalized Home is usually fat; a 3-shelf Throwback/Quick picks blob is
/// the logged-out race we must not treat as a successful refresh.
pub(crate) fn home_feed_ok(h: &Home) -> bool {
    if h.shelves.is_empty() {
        return false;
    }
    if h.shelves.len() >= 6 {
        return true;
    }
    h.shelves.iter().any(|s| {
        let t = s.title.to_lowercase();
        t.contains("listen again")
            || t.contains("mixed for you")
            || t.contains("forgotten")
            || t.contains("covers and remix")
            || t.contains("daily discover")
            || t.contains("new release")
    })
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct HomeCacheFile {
    account: Option<String>,
    home: Home,
}

fn home_cache_path(app: &App) -> std::path::PathBuf {
    app.store.data_dir().join("home-cache.json")
}

fn save_home_cache(app: &App, home: &Home) {
    if !home_feed_ok(home) {
        crate::diag::log(format!(
            "home cache: skip thin feed ({} shelves)",
            home.shelves.len()
        ));
        return;
    }
    let account = app.auth.lock().account.as_ref().map(|a| a.name.clone());
    // Don't persist a logged-out snapshot over a signed-in account.
    if account.is_some() && !home.signed_in {
        return;
    }
    // Keep a richer disk snapshot if this live result is thinner.
    if let Ok(bytes) = std::fs::read(home_cache_path(app)) {
        if let Ok(prev) = serde_json::from_slice::<HomeCacheFile>(&bytes) {
            if prev.home.shelves.len() > home.shelves.len().saturating_add(2) {
                crate::diag::log(format!(
                    "home cache: keep richer snapshot ({} > {} shelves)",
                    prev.home.shelves.len(),
                    home.shelves.len()
                ));
                return;
            }
        }
    }
    let path = home_cache_path(app);
    let file = HomeCacheFile {
        account,
        home: home.clone(),
    };
    if let Ok(json) = serde_json::to_vec(&file) {
        std::thread::spawn(move || {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        });
    }
}

/// Instant Home paint while the live feed refreshes in the background.
#[tauri::command]
fn get_home_cached(app: State<'_, Arc<App>>) -> Option<Home> {
    let bytes = std::fs::read(home_cache_path(&app)).ok()?;
    let file: HomeCacheFile = serde_json::from_slice(&bytes).ok()?;
    if file.home.shelves.is_empty() {
        return None;
    }
    let auth = app.auth.lock();
    let name = auth.account.as_ref().map(|a| a.name.clone());
    let signed = auth.signed_in;
    drop(auth);
    if signed {
        match (name.as_deref(), file.account.as_deref()) {
            // Same account.
            (Some(n), Some(a)) if n == a => Some(file.home),
            // Different named account - never flash the wrong feed.
            (Some(_), Some(_)) => None,
            // Account name not restored yet, or cache written before name landed:
            // still paint the last signed-in snapshot so Home isn't blank.
            (_, _) if file.home.signed_in => Some(file.home),
            _ => None,
        }
    } else if file.account.is_none() && !file.home.signed_in {
        Some(file.home)
    } else {
        None
    }
}

#[tauri::command]
fn clear_home_cache(app: State<'_, Arc<App>>) -> Result<(), String> {
    let path = home_cache_path(&app);
    let _ = std::fs::remove_file(path);
    if let Ok(mut slot) = app.home_slot.try_lock() {
        slot.1 = None;
    }
    Ok(())
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

/// Lightweight artist portrait for Stats (and similar) - no full page payload.
#[tauri::command]
async fn get_artist_cover(app: State<'_, Arc<App>>, id: String) -> Result<Option<String>, String> {
    app.ytm.artist_cover(&id).await.map_err(|e| e.to_string())
}

/// Session restore runs in the background after launch. A page restored from
/// the last session asks before it finishes, so the request goes out without
/// the account: Liked Music comes back as a 100-track public list and private
/// playlists fail or come back partial. Wait for the cookies (up to 10s) when
/// a saved session exists; a signed-out user does not wait.
async fn wait_for_session(app: &App) {
    if app.ytm.cookie_header().is_some() || !matches!(app.store.load(), Ok(Some(_))) {
        return;
    }
    for _ in 0..100 {
        if app.ytm.cookie_header().is_some() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tauri::command]
async fn get_album(app: State<'_, Arc<App>>, id: String) -> Result<Collection, String> {
    wait_for_session(&app).await;
    app.ytm.album(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_playlist(app: State<'_, Arc<App>>, id: String) -> Result<Collection, String> {
    let bare = id.strip_prefix("VL").unwrap_or(&id);
    if bare == "LM" {
        if let Some(c) = app.liked_music_snapshot() {
            return Ok(c);
        }
        wait_for_session(&app).await;
        // The full library load holds this lock while it runs; wait for it
        // instead of paging Liked Music a second time in parallel, which has
        // returned only the first 100 tracks.
        // If nobody has started loading it yet (this page was asked first,
        // e.g. restored from the last session), load it here and share it.
        let ticket = app
            .library_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut slot = app.library_slot.lock().await;
        let lib = match &slot.1 {
            Some(l) if slot.0 > ticket => Some(l.clone()),
            _ => match app.ytm.library().await {
                Ok(l) => {
                    if l.signed_in {
                        slot.0 = app
                            .library_calls
                            .load(std::sync::atomic::Ordering::SeqCst);
                        slot.1 = Some(l.clone());
                    }
                    Some(l)
                }
                Err(_) => None,
            },
        };
        drop(slot);
        if let Some(lib) = lib {
            if lib.signed_in && !lib.liked.is_empty() {
                app.remember_liked_tracks(lib.liked);
                if let Some(c) = app.liked_music_snapshot() {
                    return Ok(c);
                }
            }
        }
    }
    wait_for_session(&app).await;
    app.ytm.playlist(&id).await.map_err(|e| e.to_string())
}

/// First page of a playlist for an instant paint; `get_playlist` follows with
/// the whole list. Liked Music is already cached whole, so it is not split.
#[tauri::command]
async fn get_playlist_head(app: State<'_, Arc<App>>, id: String) -> Result<Collection, String> {
    let bare = id.strip_prefix("VL").unwrap_or(&id);
    if bare == "LM" {
        return get_playlist(app, id).await;
    }
    wait_for_session(&app).await;
    app.ytm.playlist_head(&id).await.map_err(|e| e.to_string())
}

/// Music video to show behind the fullscreen player, if the song has one.
#[tauri::command]
async fn bg_video(
    app: State<'_, Arc<App>>,
    id: String,
    is_video: bool,
) -> Result<Option<bgvideo::BgVideo>, String> {
    bgvideo::resolve(&app.ytm, &id, is_video)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn bg_video_cache_size() -> u64 {
    bgvideo::cache_bytes()
}

#[tauri::command]
fn bg_video_cache_clear() {
    bgvideo::clear_cache();
}

#[tauri::command]
async fn get_lyrics(app: State<'_, Arc<App>>, id: String) -> Result<Lyrics, String> {
    let yt = app.ytm.lyrics(&id).await.map_err(|e| e.to_string());
    if matches!(&yt, Ok(l) if l.synced) || !app.settings.lock().lyrics_lrclib {
        return yt;
    }
    // No timed lyrics from YouTube Music: try LRCLIB for the track we know.
    let track = {
        let (queue, _) = app.player.queue_and_index();
        queue.into_iter().find(|t| t.id == id)
    };
    if let Some(track) = track {
        if let Some(l) = lrclib::lyrics(&track).await {
            let yt_empty = !matches!(&yt, Ok(y) if !y.lines.is_empty());
            if l.synced || yt_empty {
                return Ok(l);
            }
        }
    }
    yt
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
    // Explicit click: never silently substitute another song on failure.
    start_playback_at(app, track, queue, index, 0.0, false, true, false).await
}

/// Jump List / second-instance `--play <id>`: look up metadata we stored, then play.
fn play_from_jumplist(handle: AppHandle, app: Arc<App>, id: String) {
    tauri::async_runtime::spawn(async move {
        let mut track = app.recent.get(&id).unwrap_or_else(|| Track {
            id: id.clone(),
            title: id.clone(),
            ..Default::default()
        });
        track.ensure_cover();
        let queue = vec![track.clone()];
        if let Err(e) = start_playback(&app, track, queue, 0).await {
            eprintln!("jumplist play {id}: {e}");
            *app.pending_toast.lock() = Some(format!("Couldn't play from Jump List: {e}"));
        }
        let _ = handle.emit("player", app.player.snapshot());
    });
}

pub(crate) async fn start_playback_at(
    app: &App,
    track: Track,
    queue: Vec<Track>,
    index: usize,
    start: f64,
    paused: bool,
    // True for Next/Prev/clicking a track; false for natural end advance.
    manual: bool,
    // Auto-advance on failure (queue play / next). Never for mid-track
    // recover or session restore - that was skipping while listening.
    skip_on_fail: bool,
) -> Result<(), String> {
    let id = track.id.clone();
    // Offline: only downloaded songs can play. Hop over the rest when moving
    // through the queue; refuse an explicit click with a clear reason.
    if app.is_offline() && !app.ytm.cache.has_complete(&id) {
        if skip_on_fail {
            let next = queue
                .iter()
                .enumerate()
                .skip(index + 1)
                .find(|(_, t)| app.ytm.cache.has_complete(&t.id))
                .map(|(i, t)| (i, t.clone()));
            if let Some((i, t)) = next {
                return Box::pin(start_playback_at(
                    app, t, queue, i, start, paused, manual, skip_on_fail,
                ))
                .await;
            }
            *app.pending_toast.lock() = Some("No more downloaded songs in the queue".into());
            return Err("Offline - nothing else downloaded".into());
        }
        let msg = "Not downloaded - available when you're back online".to_string();
        app.player.set_error(msg.clone());
        *app.pending_toast.lock() = Some(format!("{} isn't downloaded", track.title));
        return Err(msg);
    }
    let mut track = track;
    track.ensure_cover();
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
        if app.player.should_crossfade(snap.track.as_ref(), &track, manual) {
            crate::player::PlayStyle::Crossfade(app.player.crossfade_secs())
        } else {
            crate::player::PlayStyle::Cut
        }
    } else {
        crate::player::PlayStyle::Fresh
    };
    if manual {
        // After the decision so a resolve retry cannot trip the rapid window.
        app.player.note_manual_nav();
    }
    diag::log(format!(
        "play: {:?}{} {} → {}",
        style,
        if manual { " (manual)" } else { "" },
        snap.track.as_ref().map(|t| t.id.as_str()).unwrap_or("-"),
        track.id,
    ));
    // Overlap styles: keep the outgoing UI/clock until the mixer starts the
    // fade (same idea as natural end-blend). Swapping early made the seek bar
    // jump to the end of the next song while the old one was still audible.
    let defer_ui = matches!(
        style,
        crate::player::PlayStyle::Crossfade(_) | crate::player::PlayStyle::Cut
    ) && snap.playing
        && snap.track.is_some();
    // Heart follows the audible/UI track. During crossfade/cut the outgoing
    // title stays on screen, so stash the next rating and apply it on adopt.
    // For Fresh/Armed, apply *after* prepare so we don't briefly keep the
    // previous track's heart (prepare leaves liked untouched).
    let known_liked = app.is_liked_id(&id);
    if defer_ui {
        app.player.set_pending_rating(known_liked, false);
    }
    let index = if defer_ui {
        app.player.begin_manual_blend(queue.clone(), index)
    } else {
        if let Some(d) = track.duration {
            app.player.set_duration(d as f64);
        }
        app.player
            .prepare(track.clone(), queue.clone(), index, start, true);
        app.apply_known_rating(&id);
        index
    };
    let gen = app.player.play_gen();

    // Cover/accent is decorative - never block the play path on it.
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
            if app.player.play_gen() != gen {
                return Ok(());
            }
            let next_dur = stream.duration;
            if !defer_ui {
                if let Some(d) = next_dur {
                    app.player.set_duration(d);
                    app.player.fill_track_duration(&id, d);
                }
            } else if let Some(d) = next_dur {
                // Don't touch the outgoing duration; stash on the body so the
                // first snapshot after adopt is right.
                app.player.fill_track_duration(&id, d);
            }
            match app.ytm.start_audio(&id, &stream).await {
                Ok(audio) => {
                    if app.player.play_gen() != gen {
                        return Ok(());
                    }
                    if defer_ui {
                        if let Some(d) = next_dur {
                            audio.set_duration(d);
                            track.duration = Some(d.round().max(0.0) as u32);
                        }
                        app.player.queue_adopt(track.clone(), index);
                    }
                    let r = app
                        .player
                        .play_stream(stream, audio, start, style)
                        .map_err(|e| e.to_string());
                    if app.player.play_gen() != gen {
                        // A newer play() won the race - don't prefetch for us.
                        return Ok(());
                    }
                    if r.is_ok() {
                        app.skip_fails.store(0, std::sync::atomic::Ordering::Relaxed);
                        if defer_ui {
                            if let Some(d) = next_dur {
                                // If adopt already flushed, keep duration in sync.
                                app.player.set_duration(d);
                            }
                        }
                    } else if defer_ui {
                        app.player.clear_blending();
                    }
                    prefetch_ahead(app, &queue, index);
                    r
                }
                Err(e) => {
                    if app.player.play_gen() != gen {
                        return Ok(());
                    }
                    if defer_ui {
                        app.player.clear_blending();
                    }
                    let msg = short_err(&e.to_string());
                    playback_failed(app, &track, msg, skip_on_fail).await
                }
            }
        }
        Err(e) => {
            if app.player.play_gen() != gen {
                return Ok(());
            }
            if defer_ui {
                app.player.clear_blending();
            }
            let msg = short_err(&e.to_string());
            playback_failed(app, &track, msg, skip_on_fail).await
        }
    }
}

/// After every client and yt-dlp path has failed, optionally skip to the
/// next track. Mid-track recover / session restore must not auto-advance
/// (a 403→re-resolve miss used to skip while the user was still listening).
/// Stop after three consecutive skips when advancing through the queue.
async fn playback_failed(
    app: &App,
    track: &Track,
    msg: String,
    skip_on_fail: bool,
) -> Result<(), String> {
    let title = track.title.clone();
    if !skip_on_fail {
        eprintln!("playback failed (hold): {msg} - {title}");
        app.player.set_error(msg.clone());
        *app.pending_toast.lock() = Some(format!("Couldn't play {title}"));
        return Err(msg);
    }
    let n = app
        .skip_fails
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        + 1;
    let failed_id = track.id.clone();
    eprintln!("playback failed ({n}): {msg} - {title}");
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
    let resume = app.settings.lock().resume_on_start;
    // paused=true arms silently; paused=false starts playback immediately.
    if let Err(e) = start_playback_at(app, track, queue, idx, pos, !resume, false, false).await {
        eprintln!("restore playback: {e}");
    }
}

/// Similar tracks for when the queue runs out (autoplay).
async fn fetch_autoplay_more(ytm: &crate::ytm::Ytm, seed_id: &str) -> Vec<Track> {
    if seed_id.is_empty() {
        return Vec::new();
    }
    match ytm.radio_continue().await {
        Ok(m) if !m.is_empty() => return m,
        Ok(_) => {}
        Err(e) => eprintln!("autoplay radio_continue: {e}"),
    }
    match ytm.radio(seed_id).await {
        Ok(m) if !m.is_empty() => return m,
        Ok(_) => {}
        Err(e) => eprintln!("autoplay radio: {e}"),
    }
    match ytm.related(seed_id).await {
        Ok(r) if !r.tracks.is_empty() => r.tracks,
        Ok(_) => Vec::new(),
        Err(e) => {
            eprintln!("autoplay related: {e}");
            Vec::new()
        }
    }
}

fn prefetch_ahead(app: &App, queue: &[Track], index: usize) {
    if app.is_offline() {
        return;
    }
    let ids: Vec<String> = queue
        .iter()
        .skip(index + 1)
        .take(3)
        .map(|t| t.id.clone())
        .filter(|id| !id.is_empty())
        .collect();
    let remaining = queue.len().saturating_sub(index + 1);
    let seed = queue
        .get(index)
        .map(|t| t.id.clone())
        .unwrap_or_default();
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
            analyze_prefetched(&ytm, &player, id);
        }
        // Grow the queue before the last song ends so Up next / blend see it.
        if remaining < 3 && player.snapshot().autoplay {
            let more = fetch_autoplay_more(&ytm, &seed).await;
            if !more.is_empty() && player.snapshot().autoplay {
                player.append_unique(more.clone());
                for t in more.iter().take(3) {
                    ytm.prefetch(&t.id).await;
                }
            }
        }
    });
}

/// Measure a fully cached track in the background (loudness / beat grid)
/// so it is ready before it plays. No-op unless a feature needs it.
pub(crate) fn analyze_prefetched(ytm: &std::sync::Arc<Ytm>, player: &Player, id: &str) {
    if !player.wants_analysis() || !ytm.cache.has_complete(id) {
        return;
    }
    let (ytm, player, id) = (ytm.clone(), player.clone(), id.to_string());
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(bytes) = ytm.cache.read_complete(&id) {
            player.analyze(&id, crate::decode::GrowingBody::from_complete(bytes));
        }
    });
}

/// Phone remote: start the LAN control page and return its URL + QR code.
#[cfg(desktop)]
#[tauri::command]
async fn remote_start(app: State<'_, Arc<App>>, handle: AppHandle) -> Result<remote::RemoteInfo, String> {
    let info = remote::start(app.inner().clone(), handle).await;
    if info.running {
        let mut s = app.settings.lock().clone();
        if !s.phone_remote {
            s.phone_remote = true;
            save_settings(app.store.data_dir(), &s);
            *app.settings.lock() = s;
        }
    }
    Ok(info)
}

#[cfg(desktop)]
#[tauri::command]
fn remote_stop(app: State<'_, Arc<App>>) {
    remote::stop();
    let mut s = app.settings.lock().clone();
    if s.phone_remote {
        s.phone_remote = false;
        save_settings(app.store.data_dir(), &s);
        *app.settings.lock() = s;
    }
}

#[cfg(desktop)]
#[tauri::command]
fn remote_status() -> remote::RemoteInfo {
    remote::info()
}

/// Listen together: host a session on this network, or on a self-hosted relay
/// when `server` is given.
#[cfg(desktop)]
#[tauri::command]
async fn jam_host(
    app: State<'_, Arc<App>>,
    server: Option<String>,
    password: Option<String>,
    name: Option<String>,
) -> Result<jam::JamInfo, String> {
    let relay = server.map(|s| (s, password.unwrap_or_default()));
    Ok(jam::host(app.inner().clone(), relay, &name.unwrap_or_default()).await)
}

/// Listen together: follow a host by its join code.
#[cfg(desktop)]
#[tauri::command]
fn jam_join(app: State<Arc<App>>, code: String, name: Option<String>) -> jam::JamInfo {
    jam::join(app.inner().clone(), &code, &name.unwrap_or_default())
}

#[cfg(desktop)]
#[tauri::command]
fn jam_leave() {
    jam::stop();
}

#[cfg(desktop)]
#[tauri::command]
fn jam_status() -> jam::JamInfo {
    jam::info()
}

/// A pending invite from a tarn://join link, if any.
#[tauri::command]
fn jam_take_invite() -> Option<String> {
    #[cfg(desktop)]
    {
        jam::take_invite()
    }
    #[cfg(not(desktop))]
    {
        None
    }
}

/// Guest: add a song to the host's queue.
#[cfg(desktop)]
#[tauri::command]
async fn jam_add(track: Track) -> Result<(), String> {
    jam::add_to_host(track).await
}

#[cfg(mobile)]
#[tauri::command]
fn remote_start() -> Result<(), String> {
    Err("The phone remote runs on the desktop app".into())
}

#[cfg(mobile)]
#[tauri::command]
fn remote_stop() {}

#[cfg(mobile)]
#[tauri::command]
fn remote_status() -> serde_json::Value {
    serde_json::json!({ "running": false, "url": null, "qr_svg": null, "error": null })
}

#[cfg(mobile)]
#[tauri::command]
fn jam_host(server: Option<String>, password: Option<String>, name: Option<String>) -> Result<(), String> {
    let _ = (server, password, name);
    Err("Listen together runs on the desktop app".into())
}

#[cfg(mobile)]
#[tauri::command]
fn jam_join(code: String, name: Option<String>) -> Result<(), String> {
    let _ = (code, name);
    Err("Listen together runs on the desktop app".into())
}

#[cfg(mobile)]
#[tauri::command]
fn jam_leave() {}

#[cfg(mobile)]
#[tauri::command]
fn jam_status() -> serde_json::Value {
    serde_json::json!({ "mode": "off", "code": null, "guests": 0, "following": null, "rtt_ms": null, "error": null })
}

#[cfg(mobile)]
#[tauri::command]
fn jam_add(track: Track) -> Result<(), String> {
    let _ = track;
    Err("Listen together runs on the desktop app".into())
}

/// Cached beat grids of a track (None until it has been analysed).
#[tauri::command]
fn track_beat(app: State<Arc<App>>, id: String) -> Option<(Option<analysis::Grid>, Option<analysis::Grid>, f32)> {
    app.analysis.get(&id).map(|a| (a.head, a.tail, a.duration_s))
}

/// Waveform overview of a track for the seek bar (None until analysed).
#[tauri::command]
fn track_wave(app: State<Arc<App>>, id: String) -> Option<Vec<u8>> {
    if !app.player.wants_analysis() {
        return None;
    }
    if let Some(a) = app.analysis.get(&id) {
        return (!a.wave.is_empty()).then_some(a.wave);
    }
    // Played before the feature was on: measure it from the cache now.
    if !app.analysis.busy(&id) {
        analyze_prefetched(&app.ytm, &app.player, &id);
    }
    None
}

/// Turn the visualizer audio tap on while the fullscreen player shows it.
#[tauri::command]
fn set_visualizer(on: bool) {
    viz::set_on(on);
}

/// Current output spectrum: `bands` levels 0..=1 (low to high).
#[tauri::command]
fn viz_frame(bands: usize) -> Vec<f32> {
    if !viz::on() {
        return Vec::new();
    }
    viz::spectrum(bands)
}

/// Output devices for the settings picker.
#[tauri::command]
fn output_devices() -> output::Devices {
    output::list()
}

/// Cached tempo for each of `ids` that has been analysed (smart playlists).
#[tauri::command]
fn track_bpms(app: State<Arc<App>>, ids: Vec<String>) -> std::collections::HashMap<String, f64> {
    app.analysis.bpms(&ids)
}

#[tauri::command]
fn analysis_count(app: State<Arc<App>>) -> usize {
    app.analysis.len()
}

#[tauri::command]
fn clear_analysis(app: State<Arc<App>>) {
    app.analysis.clear();
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
    if jam::is_guest() {
        return;
    }
    app.player.remove_at(index);
    let (q, i) = app.player.queue_and_index();
    prefetch_ahead(&app, &q, i);
}

#[tauri::command]
fn queue_clear_upcoming(app: State<Arc<App>>) {
    if jam::is_guest() {
        return;
    }
    app.player.clear_upcoming();
}

#[tauri::command]
fn queue_move(app: State<Arc<App>>, from: usize, to: usize) {
    if jam::is_guest() {
        return;
    }
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
    if jam::is_guest() {
        return Ok(());
    }
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
    // In someone else's Listen along the host picks the songs: a guest's own
    // skip or end-of-track must not fight the follow loop.
    if jam::is_guest() {
        return Ok(());
    }
    {
        let snap = app.player.snapshot();
        diag::log(format!(
            "advance: user={user} from {:?} at {:.1}/{:.1}s",
            snap.track.as_ref().map(|t| t.id.as_str()),
            snap.position,
            snap.duration
        ));
    }
    match app.player.next_choice(user) {
        NextChoice::Play(i) => {
            let (queue, _) = app.player.queue_and_index();
            if let Some(t) = queue.get(i).cloned() {
                start_playback_at(app, t, queue, i, 0.0, false, user, true).await
            } else {
                Ok(())
            }
        }
        NextChoice::Autoplay => {
            if app.is_offline() {
                *app.pending_toast.lock() = Some("You're offline - no more downloaded songs".into());
                return Ok(());
            }
            let (queue, index) = app.player.queue_and_index();
            let id = queue.get(index).map(|t| t.id.clone());
            let Some(id) = id else {
                return Ok(());
            };
            let more = fetch_autoplay_more(&app.ytm, &id).await;
            if more.is_empty() {
                eprintln!("autoplay: no similar tracks for {id}");
                *app.pending_toast.lock() = Some("Couldn't find more to play".into());
                return Ok(());
            }
            app.player.append_unique(more);
            let (queue, index) = app.player.queue_and_index();
            let i = index + 1;
            if let Some(t) = queue.get(i).cloned() {
                // Autoplay continuation is a natural advance, not a user skip.
                start_playback_at(app, t, queue, i, 0.0, false, user, true).await
            } else {
                eprintln!("autoplay: appended tracks but none after index {index}");
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
async fn like_track(
    app: State<'_, Arc<App>>,
    id: String,
    action: String,
    track: Option<Track>,
) -> Result<LikeState, String> {
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
        app.set_liked_id(&id, st.liked);
        if st.liked {
            if let Some(t) = track.clone() {
                let mut g = app.liked_tracks.lock();
                if !g.iter().any(|x| x.id == t.id) {
                    g.insert(0, t);
                }
            }
        } else {
            app.liked_tracks.lock().retain(|t| t.id != id);
        }
        patch_library_liked(&app, &id, st.liked, track.as_ref()).await;
        return Ok(st);
    }
    let st = app
        .ytm
        .like(&id, &action)
        .await
        .map_err(|e| e.to_string())?;
    app.player.set_rating(st.liked, st.disliked);
    app.set_liked_id(&id, st.liked);
    if st.liked {
        // Show the new like in the Liked Music list right away instead of
        // waiting for the next library reload (the list is a cached snapshot).
        if let Some(t) = track.clone() {
            let mut g = app.liked_tracks.lock();
            if !g.iter().any(|x| x.id == t.id) {
                g.insert(0, t);
            }
        }
    } else {
        app.liked_tracks.lock().retain(|t| t.id != id);
    }
    // Keep the in-memory library slot + disk cache in sync so a later paint
    // (or the player "known liked" fallback) does not revive a removed like.
    patch_library_liked(&app, &id, st.liked, track.as_ref()).await;
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

fn library_cache_path(app: &App) -> std::path::PathBuf {
    app.store.data_dir().join("library-cache.json")
}

fn save_library_cache(app: &App, lib: &Library) {
    if !lib.signed_in || (lib.liked.is_empty() && lib.playlists.is_empty()) {
        return;
    }
    let path = library_cache_path(app);
    if let Ok(json) = serde_json::to_vec(lib) {
        std::thread::spawn(move || {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        });
    }
}

async fn patch_library_liked(app: &App, id: &str, liked: bool, track: Option<&Track>) {
    let mut slot = app.library_slot.lock().await;
    let Some(lib) = slot.1.as_mut() else {
        return;
    };
    if liked {
        if let Some(t) = track {
            if !lib.liked.iter().any(|x| x.id == t.id) {
                lib.liked.insert(0, t.clone());
            }
        }
    } else {
        lib.liked.retain(|t| t.id != id);
    }
    save_library_cache(app, lib);
}

/// Last full library from disk, for an instant sidebar / Liked list while
/// the fresh one loads. Prefer the matching account; if auth has no name
/// yet (session still settling), still return a signed-in cache so the
/// playlist sidebar is not skeletons for seconds.
#[tauri::command]
fn get_library_cached(app: State<'_, Arc<App>>) -> Option<Library> {
    let auth = app.auth.lock().clone();
    if !auth.signed_in {
        return None;
    }
    let bytes = std::fs::read(library_cache_path(&app)).ok()?;
    let lib: Library = serde_json::from_slice(&bytes).ok()?;
    if !lib.signed_in {
        return None;
    }
    if let Some(name) = auth.account.as_ref().map(|a| a.name.as_str()) {
        if let Some(saved) = lib.account.as_ref().map(|a| a.name.as_str()) {
            if saved != name {
                return None;
            }
        }
    }
    if lib.playlists.is_empty() && lib.liked.is_empty() {
        return None;
    }
    // Seed hearts before the full library round-trip finishes.
    if !lib.liked.is_empty() {
        app.remember_liked_tracks(lib.liked.clone());
        if let Some(id) = app.player.snapshot().track.as_ref().map(|t| t.id.clone()) {
            app.apply_known_rating(&id);
        }
    }
    Some(lib)
}

#[tauri::command]
async fn get_library_lite(app: State<'_, Arc<App>>) -> Result<Library, String> {
    if is_mock() {
        return Ok(mock_library());
    }
    app.ytm.library_lite().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_library(app: State<'_, Arc<App>>) -> Result<Library, String> {
    if is_mock() {
        return Ok(mock_library());
    }
    // Session restore runs in the background; a very early library call can
    // see signed_in with cookies not applied yet (or the reverse). Wait briefly.
    for _ in 0..30 {
        let signed = app.auth.lock().signed_in;
        let cookies = app.ytm.cookie_header().is_some();
        if cookies || !signed {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    }
    // The UI asked twice at startup, which downloaded the 1000+ liked tracks
    // twice in parallel and starved playback. Join the in-flight load.
    let ticket = app
        .library_calls
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut slot = app.library_slot.lock().await;
    let lib = match &slot.1 {
        Some(l) if slot.0 > ticket => l.clone(),
        _ => {
            let l = app.ytm.library().await.map_err(|e| e.to_string())?;
            slot.0 = app
                .library_calls
                .load(std::sync::atomic::Ordering::SeqCst);
            slot.1 = Some(l.clone());
            l
        }
    };
    drop(slot);
    save_library_cache(&app, &lib);
    app.remember_liked_tracks(lib.liked.clone());
    if let Some(id) = app.player.snapshot().track.as_ref().map(|t| t.id.clone()) {
        app.apply_known_rating(&id);
    }
    Ok(lib)
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
        Box::pin(async move {
            apply_session(&app_ref, header)
                .await
                .map_err(|e| e.to_string())
        })
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
    // account_menu can 200 with only the Premium CTA when SAPISIDHASH used
    // the wrong-domain cookie. Keep the login window open and retry.
    let Some(account) = account else {
        let _ = app.ytm.set_cookie_header(None).await;
        anyhow::bail!("account menu looks logged out; retrying with fresher cookies");
    };
    let mut a = app.auth.lock();
    a.signed_in = true;
    a.mock = false;
    a.account = Some(account);
    a.store = Some(match kind {
        auth::StoreKind::Keyring => "keyring".into(),
        auth::StoreKind::EncryptedFile => "encrypted_file".into(),
    });
    Ok(())
}

/// Lets the frontend leave a line in the diagnostics (layout metrics on phones).
#[tauri::command]
fn diag_note(msg: String) {
    crate::diag::log(format!("ui: {}", msg.chars().take(400).collect::<String>()));
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
    app.clear_liked();
    app.player.set_rating(false, false);
    Ok(())
}

#[tauri::command]
fn get_settings(app: State<Arc<App>>) -> Settings {
    app.settings.lock().clone()
}

#[tauri::command]
fn set_settings(app: State<Arc<App>>, handle: AppHandle, settings: Settings) {
    let mut settings = settings;
    // Keep the one-shot flag so a later load does not flip the user off again.
    settings.notifications_migrated = true;
    // Phone remote on/off is owned by remote_start / remote_stop so a stale
    // frontend settings blob cannot wipe the remembered preference.
    settings.phone_remote = app.settings.lock().phone_remote;
    app.ytm.apply_settings(&settings);
    *app.notify.lock() = settings.notifications;
    *app.discord.lock() = settings.discord_presence;
    app.player.set_autoplay(settings.autoplay);
    app.player.set_crossfade(
        settings.crossfade,
        settings.crossfade_seconds,
        settings.album_continuity,
        settings.crossfade_manual,
    );
    let mut bands = settings.eq_bands;
    for g in bands.iter_mut() {
        *g = g.clamp(-12.0, 12.0);
    }
    settings.eq_bands = bands;
    app.player.set_eq(settings.eq_bands, settings.eq_enabled);
    app.player.set_fx(settings.speed, settings.speed_keep_pitch, settings.reverb);
    {
        let before = output::prefs().0;
        output::set_prefs(settings.output_device.clone(), settings.output_auto_switch);
        let now = output::prefs().0;
        if before != now {
            app.player.set_output_device(now);
        }
    }
    app.player.set_transition(settings.transition());
    app.stats.set_enabled(settings.listening_stats);
    #[cfg(desktop)]
    if app.settings.lock().global_hotkeys != settings.global_hotkeys {
        mpris::set_user_hotkeys(settings.global_hotkeys.clone());
    }
    #[cfg(desktop)]
    tray::apply(&handle, settings.tray_icon);
    app.player
        .set_normalize(settings.normalize, settings.normalize_target);
    app.player.set_auto_dj(settings.auto_dj);
    app.player.set_waveform(settings.seek_waveform);
    save_settings(app.store.data_dir(), &settings);
    *app.settings.lock() = settings;
    if app.refresh_offline() {
        let _ = handle.emit("net", app.net_state());
    }
}



// ---------------------------------------------------------------- offline

#[tauri::command]
fn net_state(app: State<Arc<App>>) -> NetState {
    app.net_state()
}

#[tauri::command]
fn set_offline_mode(app: State<Arc<App>>, handle: AppHandle, on: bool) {
    {
        let mut s = app.settings.lock();
        s.offline_mode = on;
        save_settings(app.store.data_dir(), &s);
    }
    app.refresh_offline();
    let _ = handle.emit("net", app.net_state());
}

#[tauri::command]
fn offline_status(app: State<Arc<App>>) -> offline::Status {
    app.offline.status(&app.ytm.cache)
}

#[tauri::command]
fn offline_download(
    app: State<Arc<App>>,
    handle: AppHandle,
    source: offline::Source,
    tracks: Vec<Track>,
) {
    let id = source.id.clone();
    app.offline.add_source(&app.ytm.cache, source, tracks);
    let _ = handle.emit("offline", ());
    let off = app.offline.clone();
    let http = app.http.clone();
    tauri::async_runtime::spawn(async move {
        off.fetch_source_covers(&http, &id).await;
        let _ = handle.emit("offline", ());
    });
}

#[tauri::command]
fn offline_remove(app: State<Arc<App>>, handle: AppHandle, id: String) {
    app.offline.remove_source(&app.ytm.cache, &id);
    let _ = handle.emit("offline", ());
}

#[tauri::command]
fn offline_cancel(app: State<Arc<App>>, handle: AppHandle, id: Option<String>) {
    match id {
        Some(id) => app.offline.cancel_source(&app.ytm.cache, &id),
        None => app.offline.cancel_all(&app.ytm.cache),
    }
    let _ = handle.emit("offline", ());
}

#[tauri::command]
fn offline_remove_all(app: State<Arc<App>>, handle: AppHandle) {
    app.offline.remove_all(&app.ytm.cache);
    let _ = handle.emit("offline", ());
}

#[tauri::command]
fn offline_retry(app: State<Arc<App>>, handle: AppHandle) {
    app.offline.retry_failed(&app.ytm.cache);
    let _ = handle.emit("offline", ());
}

#[tauri::command]
fn offline_pause(app: State<Arc<App>>, handle: AppHandle, on: bool) {
    app.offline.set_paused(on);
    let _ = handle.emit("offline", ());
}

#[tauri::command]
fn offline_collection(app: State<Arc<App>>, id: String) -> Option<Collection> {
    app.offline.collection(&app.ytm.cache, &id)
}

#[tauri::command]
fn offline_tracks(app: State<Arc<App>>) -> Vec<Track> {
    app.offline.all_tracks(&app.ytm.cache)
}

/// Background reachability check; flips the effective offline flag.
fn net_loop(handle: AppHandle, app: Arc<App>) {
    tauri::async_runtime::spawn(async move {
        let mut misses = 0u8;
        loop {
            let ok = tokio::time::timeout(
                std::time::Duration::from_secs(4),
                tokio::net::TcpStream::connect(("music.youtube.com", 443)),
            )
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false);
            let down_now = app.net_down.load(std::sync::atomic::Ordering::Relaxed);
            if ok {
                misses = 0;
                if down_now {
                    app.net_down.store(false, std::sync::atomic::Ordering::Relaxed);
                    app.refresh_offline();
                    let _ = handle.emit("net", app.net_state());
                }
            } else {
                misses = misses.saturating_add(1);
                // Two misses in a row, so one dropped packet doesn't flip us.
                if misses >= 2 && !down_now {
                    app.net_down.store(true, std::sync::atomic::Ordering::Relaxed);
                    app.refresh_offline();
                    let _ = handle.emit("net", app.net_state());
                }
            }
            let wait = if app.net_down.load(std::sync::atomic::Ordering::Relaxed) { 4 } else { 10 };
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
        }
    });
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

/// Listens recorded for the Stats page since `since` (unix seconds).
#[tauri::command]
async fn get_listens(app: State<'_, Arc<App>>, since: u64) -> Result<Vec<stats::Listen>, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || app.stats.listens(since))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_listens(app: State<Arc<App>>) {
    app.stats.clear();
}

/// Global hotkeys that could not be registered ("action: reason").
#[tauri::command]
fn hotkey_errors() -> Vec<String> {
    #[cfg(desktop)]
    {
        mpris::hotkey_errors()
    }
    #[cfg(mobile)]
    {
        Vec::new()
    }
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
    if let Some(at) = *app.blend_retry.lock() {
        if std::time::Instant::now() < at {
            return;
        }
    }
    let Some((track, index, style)) = app.player.blend_due() else {
        return;
    };
    {
        let snap = app.player.snapshot();
        diag::log(format!(
            "blend: due, {:?} at {:.1}/{:.1}s -> {} ({})",
            snap.track.as_ref().map(|t| t.id.as_str()),
            snap.position,
            snap.duration,
            track.id,
            track.title
        ));
    }
    let gen = app.player.play_gen();
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
            // User started a different playlist/queue while we were resolving.
            if app.player.play_gen() != gen {
                eprintln!("blend: aborted (queue replaced during resolve)");
                app.player.clear_blending();
                return Ok(());
            }
            let still = app.player.queue_and_index().0.get(index).map(|t| t.id.as_str())
                == Some(id.as_str());
            if !still {
                eprintln!("blend: aborted (index no longer matches {id})");
                app.player.clear_blending();
                return Ok(());
            }
            // Last chance before touching the mixer - a newer play() may
            // have already prepared a different queue.
            if app.player.play_gen() != gen {
                eprintln!("blend: aborted (queue replaced before stream)");
                app.player.clear_blending();
                return Ok(());
            }
            // Queue the UI swap before the mixer command so XfReady can
            // adopt as soon as audio actually starts - never before.
            // Heart must flip with the title, not while the outgoing song shows.
            app.player
                .set_pending_rating(app.is_liked_id(&id), false);
            app.player.queue_adopt(track, index);
            if let Some(d) = next_dur {
                // Stash on the body so the first snapshot after adopt is right.
                audio.set_duration(d);
            }
            app.player
                .play_stream(stream, audio, 0.0, style)
                .map_err(|e| e.to_string())?;
            if app.player.play_gen() != gen {
                eprintln!("blend: aborted after stream (queue replaced)");
                app.player.clear_blending();
                return Ok(());
            }
            if let Some(d) = next_dur {
                // If adopt already flushed, keep duration in sync.
                app.player.set_duration(d);
                app.player.fill_track_duration(&id, d);
            }
            let (q, _) = app.player.queue_and_index();
            prefetch_ahead(&app, &q, index);
            Ok::<(), String>(())
        }
        .await;
        if let Err(e) = result {
            eprintln!("blend: {e}");
            app.player.clear_blending();
            // Offline and the next song isn't downloaded: don't hammer it every
            // tick - the normal end-of-track advance will skip ahead.
            if app.is_offline() {
                *app.blend_retry.lock() =
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(4));
            }
        }
    });
}

fn ensure_autoplay_buffer(app: &Arc<App>) {
    if app.is_offline() {
        return;
    }
    let snap = app.player.snapshot();
    if !snap.autoplay || !snap.playing {
        return;
    }
    let remaining = snap.queue.len().saturating_sub(snap.index + 1);
    if remaining > 0 {
        return;
    }
    if snap.duration <= 1.0 || snap.position < (snap.duration - 30.0).max(0.0) {
        return;
    }
    let Some(seed) = snap.track.as_ref().map(|t| t.id.clone()) else {
        return;
    };
    {
        let mut slot = app.autoplay_fetching.lock();
        if slot.as_deref() == Some(seed.as_str()) {
            return;
        }
        *slot = Some(seed.clone());
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let more = fetch_autoplay_more(&app.ytm, &seed).await;
        if !more.is_empty() && app.player.snapshot().autoplay {
            let n = more.len();
            app.player.append_unique(more.clone());
            eprintln!("autoplay: prefilled +{n} before end of {seed}");
            for t in more.iter().take(3) {
                app.ytm.prefetch(&t.id).await;
            }
        }
        let mut slot = app.autoplay_fetching.lock();
        if slot.as_deref() == Some(seed.as_str()) {
            *slot = None;
        }
    });
}

/// Cheap fingerprint of the queue (order, ids and the fields the UI shows).
fn queue_signature(queue: &[Track]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    queue.len().hash(&mut h);
    for t in queue {
        t.id.hash(&mut h);
        t.title.hash(&mut h);
        t.cover.hash(&mut h);
        t.duration.hash(&mut h);
    }
    h.finish()
}

/// Skip a music video's off-topic intro / outro when SponsorBlock is on.
fn sponsorblock_tick(app: &Arc<App>, handle: &AppHandle) {
    if !app.settings.lock().sponsorblock || app.is_offline() {
        return;
    }
    let st = app.player.snapshot();
    let Some(track) = st.track.as_ref() else { return };
    if !st.playing || st.loading {
        return;
    }
    match app.sponsorblock.check(&app.http, &track.id, st.position, st.duration) {
        Some(sponsorblock::Action::Seek(to)) => {
            diag::log(format!("sponsorblock: skip {:.1}s -> {to:.1}s", st.position));
            app.player.seek(to);
            let _ = handle.emit("toast", "Skipped non-music section");
        }
        Some(sponsorblock::Action::Next) => {
            diag::log("sponsorblock: skip off-topic outro");
            let app2 = app.clone();
            let handle2 = handle.clone();
            tauri::async_runtime::spawn(async move {
                let _ = advance(&app2, false).await;
                let _ = handle2.emit("player", app2.player.snapshot());
            });
        }
        None => {}
    }
}

fn emit_loop(handle: AppHandle, app: Arc<App>) {
    std::thread::spawn(move || {
        let mut last_save = std::time::Instant::now();
        // The queue can be thousands of tracks: send it when it changes (and
        // now and then, to resync), not with every 200 ms position update.
        let mut last_queue_sig: Option<u64> = None;
        let mut last_queue_sent = std::time::Instant::now();
        loop {
            // Keep a steady 200 ms tick even during audio crossfade. A 50 ms
            // emit storm was thrashing WebView2 (dual-layer morph + tray).
            std::thread::sleep(std::time::Duration::from_millis(200));
            if let Some(header) = app.ytm.harvest_cookies() {
                if let Err(e) = app.store.save(&header) {
                    eprintln!("session persist (rotated cookies): {e}");
                }
            }
            maybe_blend(&app);
            ensure_autoplay_buffer(&app);
            if let Some((track, queue, index, pos, play)) = app.player.take_recover() {
                let app2 = app.clone();
                let handle2 = handle.clone();
                tauri::async_runtime::spawn(async move {
                    eprintln!(
                        "recover: re-resolving {} at {:.2}s (play={play})",
                        track.id, pos
                    );
                    app2.ytm.invalidate(&track.id);
                    // Never auto-skip on recover failure - stay on this track.
                    let r = start_playback_at(
                        &app2, track, queue, index, pos, !play, false, false,
                    )
                    .await;
                    app2.player.recover_finished();
                    if let Err(e) = r {
                        eprintln!("recover failed: {e}; holding track (no skip)");
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
            sponsorblock_tick(&app, &handle);
            if let Some(msg) = app.pending_toast.lock().take() {
                let _ = handle.emit("toast", msg);
            }
            let mut st = app.player.snapshot();
            app.stats.tick(&st);
            if let Some(t) = st.track.as_ref() {
                if app.recent.note(t) {
                    let list = app.recent.list();
                    std::thread::spawn(move || jumplist::publish(&list));
                }
            }
            let sig = queue_signature(&st.queue);
            if last_queue_sig == Some(sig)
                && last_queue_sent.elapsed() < std::time::Duration::from_secs(10)
            {
                st.queue = Vec::new();
                st.queue_same = true;
            } else {
                last_queue_sig = Some(sig);
                last_queue_sent = std::time::Instant::now();
            }
            #[cfg(desktop)]
            tray::update(&handle, &st);
            #[cfg(windows)]
            thumbbar::update(&st);
            #[cfg(target_os = "macos")]
            dock::update(&handle, &st);
            let _ = handle.emit("player", st);
            if last_save.elapsed() >= std::time::Duration::from_secs(3) {
                persist::save(&handle);
                last_save = std::time::Instant::now();
            }
        }
    });
}

/// Android has no HOME / XDG dirs, so `dirs::data_dir()` and friends resolve to
/// read-only paths ("failed to start: Read-only file system"). Point them at
/// the app's private storage before anything touches the disk.
#[cfg(target_os = "android")]
fn init_android_dirs() {
    let pkg = std::fs::read("/proc/self/cmdline")
        .ok()
        .and_then(|b| {
            let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
            String::from_utf8(b[..end].to_vec()).ok()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "app.tarn.desktop".to_string());
    let base = std::path::PathBuf::from("/data/data").join(&pkg);
    let files = base.join("files");
    let cache = base.join("cache");
    for d in [&files, &cache, &files.join("config")] {
        let _ = std::fs::create_dir_all(d);
    }
    std::env::set_var("HOME", &files);
    std::env::set_var("XDG_DATA_HOME", &files);
    std::env::set_var("XDG_CONFIG_HOME", files.join("config"));
    std::env::set_var("XDG_CACHE_HOME", &cache);
    std::env::set_var("XDG_RUNTIME_DIR", &cache);
    std::env::set_var("TMPDIR", &cache);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "android")]
    init_android_dirs();
    // iOS: must be set before the audio output opens, or the OS silences us
    // in the background / when the screen locks.
    #[cfg(target_os = "ios")]
    if let Err(e) = crate::ios_media::setup_audio_session() {
        eprintln!("ios audio session: {e}");
    }
    crate::win::init();
    // WebKitGTK compositing often blurs glyphs at fractional DPI on X11, but
    // on Wayland forcing it off causes black flicker. Keep it off only on X11.
    if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
        let on_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let value = if on_wayland { "0" } else { "1" };
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", value);
    }
    let app = match App::new() {
        Ok(a) => Arc::new(a),
        Err(e) => {
            eprintln!("failed to start: {e}");
            std::process::exit(1);
        }
    };

    let launch_play = jumplist::play_id_from_args(
        &std::env::args().collect::<Vec<_>>(),
    );
    let app_for_setup = app.clone();
    let mut builder = tauri::Builder::default().manage(app);
    #[cfg(desktop)]
    {
        builder = builder
            // Must be the first plugin: a second launch hands over to the
            // running instance (brings its window back, even from the tray)
            // and exits instead of opening another copy.
            .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
                tray::show_window(app);
                if let Some(id) = jumplist::play_id_from_args(&argv) {
                    if let Some(state) = app.try_state::<Arc<App>>() {
                        play_from_jumplist(app.clone(), state.inner().clone(), id);
                    }
                }
            }))
            .plugin(tauri_plugin_deep_link::init())
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_process::init());
    }
    builder
        .register_uri_scheme_protocol("tarnvideo", |_ctx, req| bgvideo::serve(&req))
        .register_uri_scheme_protocol("tarncover", |_ctx, req| {
            let name = req
                .uri()
                .path()
                .trim_start_matches('/')
                .to_string();
            let ok = name.len() == 16 && name.chars().all(|c| c.is_ascii_hexdigit());
            let path = offline::root_dir().join("covers").join(&name);
            match ok.then(|| std::fs::read(path).ok()).flatten() {
                Some(bytes) => tauri::http::Response::builder()
                    .header("Content-Type", offline::sniff_image(&bytes))
                    .header("Cache-Control", "max-age=86400")
                    .header("Access-Control-Allow-Origin", "*")
                    .body(bytes)
                    .unwrap(),
                None => tauri::http::Response::builder()
                    .status(404)
                    .body(Vec::new())
                    .unwrap(),
            }
        })
        .setup(move |tauri_app| {
            let handle = tauri_app.handle().clone();
            let app = app_for_setup.clone();
            #[cfg(desktop)]
            {
                use tauri::Emitter;
                use tauri_plugin_deep_link::DeepLinkExt;
                // tarn://join?code=... from the jam server's invite page.
                #[cfg(any(target_os = "linux", windows))]
                let _ = tauri_app.deep_link().register_all();
                let h = handle.clone();
                tauri_app.deep_link().on_open_url(move |ev| {
                    for u in ev.urls() {
                        if let Some(code) = jam::code_from_deep_link(u.as_str()) {
                            jam::set_invite(code);
                            tray::show_window(&h);
                            let _ = h.emit("jam-invite", ());
                        }
                    }
                });
            }
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
            let handle_restore = handle.clone();
            let cold_play = launch_play.clone();
            tauri::async_runtime::spawn(async move {
                restore_session(&app_restore).await;
                // Frontend often mounts before restore finishes and would keep
                // signed_in=false forever without this - library/playlists stay empty
                // until a manual reload.
                if app_restore.auth.lock().signed_in {
                    let _ = handle_restore.emit(
                        "auth",
                        serde_json::json!({ "signed_in": true, "restored": true }),
                    );
                }
                // Jump List / CLI `--play` wins over the previous session.
                if let Some(id) = cold_play {
                    play_from_jumplist(handle_restore.clone(), app_restore, id);
                } else {
                    restore_playback(&app_restore).await;
                }
            });
            app.ytm.prewarm_players();
            tauri::async_runtime::spawn(async {
                crate::ytdlp::maybe_update().await;
            });
            // Populate the taskbar Jump List from disk (or seeded listens).
            {
                let list = app.recent.list();
                if !list.is_empty() {
                    std::thread::spawn(move || jumplist::publish(&list));
                }
            }
            emit_loop(handle.clone(), app.clone());
            autodj::start(app.clone());
            output::watch(app.player.clone());
            net_loop(handle.clone(), app.clone());
            {
                let h = handle.clone();
                let emit: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                    let _ = h.emit("offline", ());
                });
                app.offline.spawn_workers(
                    app.ytm.clone(),
                    app.player.clone(),
                    app.http.clone(),
                    emit,
                );
            }
            #[cfg(desktop)]
            tray::apply(&handle, app.settings.lock().tray_icon);
            #[cfg(desktop)]
            if app.settings.lock().phone_remote {
                let app2 = app.clone();
                let h = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = remote::start(app2, h).await;
                });
            }
            let hwnd = main_hwnd(tauri_app);
            mpris::start(app.clone(), handle.clone(), hwnd);
            #[cfg(windows)]
            thumbbar::start(app.clone(), handle.clone(), hwnd);
            #[cfg(target_os = "macos")]
            dock::start(app.clone(), &handle);
            discord::start(app.clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bg_video,
            bg_video_cache_size,
            bg_video_cache_clear,
            search_tracks,
            search_suggestions,
            search_all,
            get_home,
            get_home_cached,
            clear_home_cache,
            get_explore,
            get_genre,
            get_charts,
            get_new_releases,
            get_artist,
            get_artist_cover,
            get_album,
            get_playlist,
            get_playlist_head,
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
            queue_clear_upcoming,
            net_state,
            set_offline_mode,
            offline_status,
            offline_download,
            offline_remove,
            offline_remove_all,
            offline_cancel,
            offline_retry,
            offline_pause,
            offline_collection,
            offline_tracks,
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
            get_library_lite,
            get_library_cached,
            get_history,
            auth_status,
            open_login,
            sign_out,
            diagnostics,
            diag_note,
            get_settings,
            set_settings,
            extract_cookie_names,
            prefetch,
            analysis_count,
            track_beat,
            track_wave,
            set_visualizer,
            viz_frame,
            output_devices,
            track_bpms,
            remote_start,
            remote_stop,
            remote_status,
            jam_host,
            jam_join,
            jam_leave,
            jam_status,
            jam_add,
            jam_take_invite,
            clear_analysis,
            cache_stats,
            get_listens,
            clear_listens,
            hotkey_errors,
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
                    // Close to tray: hide instead of quitting.
                    #[cfg(desktop)]
                    if let WindowEvent::CloseRequested { api, .. } = &event {
                        let to_tray = handle
                            .try_state::<Arc<App>>()
                            .is_some_and(|a| a.settings.lock().close_to_tray);
                        if to_tray && tray::active(handle) {
                            api.prevent_close();
                            persist::save(handle);
                            if let Some(w) = handle.get_webview_window("main") {
                                let _ = w.hide();
                            }
                            return;
                        }
                    }
                    if matches!(
                        event,
                        WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
                    ) {
                        persist::save(handle);
                    }
                }
                RunEvent::ExitRequested { .. } | RunEvent::Exit => {
                    persist::save(handle);
                    if let Some(app) = handle.try_state::<Arc<App>>() {
                        app.stats.flush();
                    }
                }
                // macOS: clicking the Dock icon brings a hidden window back.
                #[cfg(target_os = "macos")]
                RunEvent::Reopen { .. } => tray::show_window(handle),
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
    fn old_settings_get_neutral_transition_defaults() {
        let dir = tmp_dir("xf-defaults");
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
        let t = s.transition();
        // Missing keys must reproduce the classic equal-power blend.
        assert_eq!(t.shape, crate::transition::XfShape::equal_power(4.0));
        assert!(!t.smart && !t.bpm_match);
        assert!(s.crossfade_manual);
        // New keys round-trip.
        let mut c = s.clone();
        c.crossfade_curve_in = "s_curve".into();
        c.crossfade_filter = "sweep".into();
        c.bpm_match = true;
        c.bpm_max_change = 4.0;
        save_settings(&dir, &c);
        let again = load_settings(&dir);
        let t = again.transition();
        assert_eq!(t.shape.curve_in, crate::transition::Curve::Smooth);
        assert_eq!(t.shape.filter, crate::transition::FilterSweep::Sweep);
        assert!(t.bpm_match);
        assert!((t.bpm_max_pct - 4.0).abs() < 1e-6);
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
