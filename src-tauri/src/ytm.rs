use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use rustypipe::client::{ClientType, RustyPipe};
use rustypipe::model::{
    AlbumItem, AlbumType, ArtistItem, AudioCodec, AudioFormat, MusicItem, MusicPlaylistItem,
    Thumbnail, TrackItem, TrackType,
};
use rustypipe::model::richtext::ToPlaintext;
use serde_json::Value;
use tokio::process::Command;

use crate::auth;
use crate::innertube::{self, InnerTube};
use crate::models::{
    AccountInfo, ArtistCard, ArtistPage, Collection, Explore, GenrePage, Home, Library, LikeState,
    Lyrics, PlaylistCard, Related, SearchResults, Settings, Shelf, ShelfItem, Track,
};

const STREAM_TTL: Duration = Duration::from_secs(5 * 60 * 60);

struct CachedStream {
    stream: Stream,
    at: Instant,
}

pub struct Ytm {
    rp: RustyPipe,
    http: reqwest::Client,
    lang: Mutex<String>,
    country: Mutex<String>,
    quality: Mutex<String>,
    cookie: Mutex<Option<String>>,
    cookies_file: PathBuf,
    streams: Mutex<std::collections::HashMap<String, CachedStream>>,
    bodies: Mutex<std::collections::HashMap<String, std::sync::Arc<crate::decode::GrowingBody>>>,
    pub cache: std::sync::Arc<crate::cache::AudioCache>,
    radio: Mutex<Option<rustypipe::model::paginator::Paginator<TrackItem>>>,
}

impl Ytm {
    pub fn new() -> Result<Self> {
        let dir = cache_dir();
        std::fs::create_dir_all(&dir).ok();
        let data = dirs::data_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("tarn");
        std::fs::create_dir_all(&data).ok();
        let cookies_file = data.join("yt-cookies.txt");
        let rp = RustyPipe::builder()
            .storage_dir(&dir)
            .build()
            .map_err(|e| anyhow!("rustypipe init: {e}"))?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(user_agent())
            .build()?;
        Ok(Self {
            rp,
            http,
            lang: Mutex::new("en".into()),
            country: Mutex::new("US".into()),
            quality: Mutex::new("high".into()),
            cookie: Mutex::new(None),
            cookies_file,
            streams: Mutex::new(std::collections::HashMap::new()),
            bodies: Mutex::new(std::collections::HashMap::new()),
            cache: std::sync::Arc::new(crate::cache::AudioCache::open()),
            radio: Mutex::new(None),
        })
    }

    pub fn apply_settings(&self, s: &Settings) {
        *self.lang.lock().unwrap() = s.language.clone();
        *self.country.lock().unwrap() = s.region.clone();
        *self.quality.lock().unwrap() = s.quality.clone();
        self.cache.set_max_mb(s.cache_max_mb);
    }

    pub fn cookies_file(&self) -> &PathBuf {
        &self.cookies_file
    }

    pub fn cookie_header(&self) -> Option<String> {
        self.cookie.lock().unwrap().clone()
    }

    pub async fn set_cookie_header(&self, header: Option<String>) -> Result<()> {
        match &header {
            Some(h) if !h.is_empty() => {
                let netscape = auth::netscape_from_header(h);
                auth::write_netscape_file(&self.cookies_file, &netscape)?;
                // rustypipe may write cache; never log the header.
                if let Err(e) = self.rp.user_auth_set_cookie(h.clone()).await {
                    eprintln!("rustypipe cookie: {e}");
                }
                // Tighten rustypipe cache files if they appeared.
                tighten_cache(&cache_dir());
                *self.cookie.lock().unwrap() = Some(h.clone());
            }
            _ => {
                let _ = self.rp.user_auth_remove_cookie().await;
                auth::remove_file_quiet(&self.cookies_file);
                *self.cookie.lock().unwrap() = None;
            }
        }
        Ok(())
    }

    fn it(&self) -> InnerTube {
        InnerTube::new(
            self.http.clone(),
            self.cookie_header(),
            self.lang.lock().unwrap().clone(),
            self.country.lock().unwrap().clone(),
        )
    }

    fn quality(&self) -> String {
        self.quality.lock().unwrap().clone()
    }

    pub async fn search_tracks(&self, query: &str) -> Result<Vec<Track>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        match self.rp.query().music_search_tracks(q).await {
            Ok(res) => {
                let tracks: Vec<Track> = res.items.items.iter().map(track_from).collect();
                if tracks.is_empty() {
                    let vids = self.rp.query().music_search_videos(q).await;
                    if let Ok(v) = vids {
                        return Ok(v.items.items.iter().map(track_from).collect());
                    }
                }
                Ok(tracks)
            }
            Err(e) => {
                eprintln!("rustypipe search failed: {e}");
                search_ytdlp(q).await
            }
        }
    }

    pub async fn search_suggestions(&self, query: &str) -> Result<Vec<String>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        match self.rp.query().music_search_suggestion(q).await {
            Ok(s) => Ok(s.terms.into_iter().take(8).collect()),
            Err(_) => Ok(Vec::new()),
        }
    }

    pub async fn search_all(&self, query: &str) -> Result<SearchResults> {
        let q = query.trim().to_string();
        if q.is_empty() {
            return Ok(SearchResults::default());
        }
        let q_main = self.rp.query();
        let q_songs = self.rp.query();
        let q_videos = self.rp.query();
        let q_albums = self.rp.query();
        let q_artists = self.rp.query();
        let q_pl = self.rp.query();
        let q_comm = self.rp.query();
        let (main, songs, videos, albums, artists, playlists, community) = tokio::join!(
            q_main.music_search_main(&q),
            q_songs.music_search_tracks(&q),
            q_videos.music_search_videos(&q),
            q_albums.music_search_albums(&q),
            q_artists.music_search_artists(&q),
            q_pl.music_search_playlists(&q, false),
            q_comm.music_search_playlists(&q, true),
        );
        let mut top = None;
        if let Ok(m) = main {
            if let Some(item) = m.items.items.first() {
                top = Some(music_item_to_shelf(item));
            }
        }
        let songs: Vec<Track> = songs
            .map(|r| r.items.items.iter().map(track_from).collect())
            .unwrap_or_default();
        let videos: Vec<Track> = videos
            .map(|r| {
                r.items
                    .items
                    .iter()
                    .map(|t| {
                        let mut tr = track_from(t);
                        tr.is_video = true;
                        tr
                    })
                    .collect()
            })
            .unwrap_or_default();
        let albums: Vec<PlaylistCard> = albums
            .map(|r| r.items.items.iter().map(album_from).collect())
            .unwrap_or_default();
        let artists: Vec<ArtistCard> = artists
            .map(|r| r.items.items.iter().map(artist_from).collect())
            .unwrap_or_default();
        let playlists: Vec<PlaylistCard> = playlists
            .map(|r| r.items.items.iter().map(playlist_from).collect())
            .unwrap_or_default();
        let community: Vec<PlaylistCard> = community
            .map(|r| r.items.items.iter().map(playlist_from).collect())
            .unwrap_or_default();
        if top.is_none() {
            top = artists
                .first()
                .filter(|a| a.name.eq_ignore_ascii_case(&q))
                .map(|a| ShelfItem {
                    kind: "artist".into(),
                    id: a.id.clone(),
                    title: a.name.clone(),
                    cover: a.cover.clone(),
                    ..Default::default()
                })
                .or_else(|| {
                    songs.first().map(|t| ShelfItem {
                        kind: "track".into(),
                        id: t.id.clone(),
                        title: t.title.clone(),
                        subtitle: Some(t.artists.join(", ")),
                        cover: t.cover.clone(),
                        artists: t.artists.clone(),
                        artist_ids: t.artist_ids.clone(),
                        album: t.album.clone(),
                        album_id: t.album_id.clone(),
                        duration: t.duration,
                        is_video: t.is_video,
                        ..Default::default()
                    })
                });
        }
        Ok(SearchResults {
            query: q,
            top,
            songs,
            videos,
            albums,
            artists,
            playlists,
            community,
        })
    }

    pub async fn home(&self, params: Option<&str>) -> Result<Home> {
        let mut home = match self.it().home(params).await {
            Ok(h) if !h.shelves.is_empty() => h,
            Ok(h) => h,
            Err(e) => {
                eprintln!("home browse: {e}");
                Home::default()
            }
        };
        // Always enrich with charts / new releases when those shelves are missing,
        // so a logged-out session still feels full.
        let has = |h: &Home, needle: &str| {
            h.shelves
                .iter()
                .any(|s| s.title.to_lowercase().contains(needle))
        };
        if !has(&home, "top") && !has(&home, "chart") && !has(&home, "trending") {
            match self.rp.query().music_charts(None).await {
                Ok(c) => {
                    let top: Vec<ShelfItem> = c.top_tracks.iter().map(track_item).collect();
                    let trend: Vec<ShelfItem> = c.trending_tracks.iter().map(track_item).collect();
                    let artists: Vec<ShelfItem> = c.artists.iter().map(artist_item).collect();
                    let lists: Vec<ShelfItem> = c.playlists.iter().map(playlist_item).collect();
                    if !top.is_empty() {
                        home.shelves.push(Shelf {
                            title: "Top songs".into(),
                            kind: "tracks".into(),
                            items: top,
                            ..Default::default()
                        });
                    }
                    if !trend.is_empty() {
                        home.shelves.push(Shelf {
                            title: "Trending".into(),
                            kind: "tracks".into(),
                            items: trend,
                            ..Default::default()
                        });
                    }
                    if !lists.is_empty() {
                        home.shelves.push(Shelf {
                            title: "Top playlists".into(),
                            kind: "playlists".into(),
                            items: lists,
                            ..Default::default()
                        });
                    }
                    if !artists.is_empty() {
                        home.shelves.push(Shelf {
                            title: "Artists".into(),
                            kind: "artists".into(),
                            items: artists,
                            ..Default::default()
                        });
                    }
                }
                Err(e) => eprintln!("charts: {e}"),
            }
        }
        if !has(&home, "new album") && !has(&home, "new release") && !has(&home, "albums & singles")
        {
            if let Ok(albums) = self.rp.query().music_new_albums().await {
                let items: Vec<ShelfItem> = albums.iter().map(album_item).take(24).collect();
                if !items.is_empty() {
                    home.shelves.push(Shelf {
                        title: "New albums & singles".into(),
                        kind: "albums".into(),
                        items,
                        ..Default::default()
                    });
                }
            }
        }
        if !has(&home, "new music video") && !has(&home, "new video") {
            if let Ok(vids) = self.rp.query().music_new_videos().await {
                let items: Vec<ShelfItem> = vids
                    .iter()
                    .map(|t| {
                        let mut i = track_item(t);
                        i.kind = "video".into();
                        i.is_video = true;
                        i
                    })
                    .take(24)
                    .collect();
                if !items.is_empty() {
                    home.shelves.push(Shelf {
                        title: "New music videos".into(),
                        kind: "videos".into(),
                        items,
                        ..Default::default()
                    });
                }
            }
        }
        if home.chips.is_empty() {
            if let Ok(genres) = self.rp.query().music_genres().await {
                home.chips = genres
                    .into_iter()
                    .filter(|g| g.is_mood)
                    .map(|g| crate::models::MoodChip {
                        title: g.name,
                        params: Some(g.id),
                        browse_id: Some("FEmusic_moods_and_genres_category".into()),
                        color: Some(argb_to_hex(g.color)),
                        selected: false,
                    })
                    .collect();
            }
        }
        home.signed_in = self.cookie_header().is_some();
        Ok(home)
    }

    pub async fn explore(&self) -> Result<Explore> {
        let mut ex = match self.it().explore().await {
            Ok(e) if !e.shelves.is_empty() || !e.moods.is_empty() => e,
            Ok(e) => e,
            Err(e) => {
                eprintln!("explore browse: {e}");
                Explore::default()
            }
        };
        if ex.shelves.is_empty() {
            if let Ok(albums) = self.rp.query().music_new_albums().await {
                ex.shelves.push(Shelf {
                    title: "New albums & singles".into(),
                    kind: "albums".into(),
                    items: albums.iter().map(album_item).take(24).collect(),
                    ..Default::default()
                });
            }
            if let Ok(vids) = self.rp.query().music_new_videos().await {
                ex.shelves.push(Shelf {
                    title: "New music videos".into(),
                    kind: "videos".into(),
                    items: vids
                        .iter()
                        .map(|t| {
                            let mut i = track_item(t);
                            i.kind = "video".into();
                            i.is_video = true;
                            i
                        })
                        .take(24)
                        .collect(),
                    ..Default::default()
                });
            }
            if let Ok(c) = self.rp.query().music_charts(None).await {
                let trend: Vec<ShelfItem> = c.trending_tracks.iter().map(track_item).collect();
                if !trend.is_empty() {
                    ex.shelves.push(Shelf {
                        title: "Trending".into(),
                        kind: "tracks".into(),
                        items: trend,
                        ..Default::default()
                    });
                }
            }
        }
        if ex.moods.is_empty() {
            if let Ok(genres) = self.rp.query().music_genres().await {
                ex.moods = genres
                    .into_iter()
                    .map(|g| ShelfItem {
                        kind: "mood".into(),
                        id: g.id.clone(),
                        title: g.name,
                        color: Some(argb_to_hex(g.color)),
                        params: Some(g.id),
                        browse_id: Some("FEmusic_moods_and_genres_category".into()),
                        ..Default::default()
                    })
                    .collect();
            }
        }
        Ok(ex)
    }

    pub async fn genre(&self, id: &str) -> Result<GenrePage> {
        let g = self.rp.query().music_genre(id).await.map_err(|e| anyhow!("{e}"))?;
        let sections = g
            .sections
            .into_iter()
            .map(|s| Shelf {
                id: s.subgenre_id,
                title: s.name,
                kind: "playlists".into(),
                items: s.playlists.iter().map(playlist_item).collect(),
                ..Default::default()
            })
            .filter(|s| !s.items.is_empty())
            .collect();
        Ok(GenrePage {
            id: g.id,
            name: g.name,
            sections,
        })
    }

    pub async fn charts_page(&self) -> Result<Home> {
        match self.it().charts().await {
            Ok(h) if !h.shelves.is_empty() => return Ok(h),
            Err(e) => eprintln!("charts browse: {e}"),
            _ => {}
        }
        let mut home = Home::default();
        if let Ok(c) = self.rp.query().music_charts(None).await {
            let top: Vec<ShelfItem> = c.top_tracks.iter().map(track_item).collect();
            let trend: Vec<ShelfItem> = c.trending_tracks.iter().map(track_item).collect();
            if !top.is_empty() {
                home.shelves.push(Shelf {
                    title: "Top songs".into(),
                    kind: "tracks".into(),
                    items: top,
                    ..Default::default()
                });
            }
            if !trend.is_empty() {
                home.shelves.push(Shelf {
                    title: "Trending".into(),
                    kind: "tracks".into(),
                    items: trend,
                    ..Default::default()
                });
            }
            if !c.playlists.is_empty() {
                home.shelves.push(Shelf {
                    title: "Top playlists".into(),
                    kind: "playlists".into(),
                    items: c.playlists.iter().map(playlist_item).collect(),
                    ..Default::default()
                });
            }
            if !c.artists.is_empty() {
                home.shelves.push(Shelf {
                    title: "Artists".into(),
                    kind: "artists".into(),
                    items: c.artists.iter().map(artist_item).collect(),
                    ..Default::default()
                });
            }
        }
        Ok(home)
    }

    pub async fn new_releases(&self) -> Result<Home> {
        match self.it().new_releases().await {
            Ok(h) if !h.shelves.is_empty() => return Ok(h),
            Err(e) => eprintln!("new releases: {e}"),
            _ => {}
        }
        let mut home = Home::default();
        if let Ok(albums) = self.rp.query().music_new_albums().await {
            home.shelves.push(Shelf {
                title: "New albums & singles".into(),
                kind: "albums".into(),
                items: albums.iter().map(album_item).collect(),
                ..Default::default()
            });
        }
        if let Ok(vids) = self.rp.query().music_new_videos().await {
            home.shelves.push(Shelf {
                title: "New music videos".into(),
                kind: "videos".into(),
                items: vids
                    .iter()
                    .map(|t| {
                        let mut i = track_item(t);
                        i.kind = "video".into();
                        i.is_video = true;
                        i
                    })
                    .collect(),
                ..Default::default()
            });
        }
        Ok(home)
    }

    pub async fn artist(&self, id: &str) -> Result<ArtistPage> {
        let a = self
            .rp
            .query()
            .music_artist(id, true)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        let mut albums = Vec::new();
        let mut singles = Vec::new();
        for al in &a.albums {
            let c = album_from(al);
            match al.album_type {
                AlbumType::Single | AlbumType::Ep => singles.push(c),
                _ => albums.push(c),
            }
        }
        let mut videos = Vec::new();
        if let Some(pid) = &a.videos_playlist_id {
            if let Ok(pl) = self.rp.query().music_playlist(pid).await {
                videos = pl.tracks.items.iter().map(track_from).take(40).collect();
                for v in &mut videos {
                    v.is_video = true;
                }
            }
        }
        let cover = best_thumb(&a.header_image)
            .or_else(|| a.tracks.first().and_then(|t| best_thumb(&t.cover)))
            .or_else(|| a.albums.first().and_then(|t| best_thumb(&t.cover)));
        let mut tracks: Vec<Track> = a.tracks.iter().map(track_from).collect();
        self.enrich_durations(&mut tracks, a.tracks_playlist_id.as_deref(), Some(&a.id))
            .await;
        self.enrich_durations(&mut videos, a.videos_playlist_id.as_deref(), None)
            .await;
        Ok(ArtistPage {
            id: a.id,
            name: a.name,
            header: best_thumb_wide(&a.header_image),
            cover,
            description: a.description,
            subscribers: a.subscriber_count,
            tracks,
            albums,
            singles,
            videos,
            playlists: a.playlists.iter().map(playlist_from).collect(),
            similar: a.similar_artists.iter().map(artist_from).collect(),
            radio_id: a.radio_id,
            tracks_playlist_id: a.tracks_playlist_id,
            videos_playlist_id: a.videos_playlist_id,
            subscribed: false,
        })
    }

    /// rustypipe's artist page often omits track length. Fill from the artist's
    /// tracks playlist, an InnerTube browse, then per-track music_details.
    async fn enrich_durations(
        &self,
        tracks: &mut [Track],
        playlist_id: Option<&str>,
        browse_id: Option<&str>,
    ) {
        fn needs(t: &Track) -> bool {
            t.duration.is_none() || t.duration == Some(0)
        }
        if !tracks.iter().any(needs) {
            return;
        }
        if let Some(pid) = playlist_id {
            if let Ok(pl) = self.rp.query().music_playlist(pid).await {
                let map: HashMap<String, Track> = pl
                    .tracks
                    .items
                    .iter()
                    .map(|t| {
                        let tr = track_from(t);
                        (tr.id.clone(), tr)
                    })
                    .collect();
                for t in tracks.iter_mut() {
                    if let Some(f) = map.get(&t.id) {
                        if needs(t) {
                            t.duration = f.duration;
                        }
                        if t.album.is_none() {
                            t.album = f.album.clone();
                            t.album_id = f.album_id.clone();
                        }
                    }
                }
            }
        }
        if !tracks.iter().any(needs) {
            return;
        }
        if let Some(bid) = browse_id {
            if let Ok(body) = self.it().browse(bid, None).await {
                let durs = innertube::durations_by_video(&body);
                for t in tracks.iter_mut() {
                    if needs(t) {
                        if let Some(&d) = durs.get(&t.id) {
                            if d > 0 {
                                t.duration = Some(d);
                            }
                        }
                    }
                }
            }
        }
        if !tracks.iter().any(needs) {
            return;
        }
        let missing: Vec<String> = tracks
            .iter()
            .filter(|t| needs(t))
            .map(|t| t.id.clone())
            .take(12)
            .collect();
        if missing.is_empty() {
            return;
        }
        // Sequential is a last resort (few tracks). music_details is reliable.
        for id in missing {
            if let Ok(d) = self.rp.query().music_details(&id).await {
                if let Some(sec) = d.track.duration.map(normalize_duration) {
                    if sec > 0 {
                        if let Some(t) = tracks.iter_mut().find(|t| t.id == id) {
                            t.duration = Some(sec);
                            if t.album.is_none() {
                                let full = track_from(&d.track);
                                t.album = full.album;
                                t.album_id = full.album_id;
                            }
                        }
                    }
                }
            }
        }
    }

    pub async fn album(&self, id: &str) -> Result<Collection> {
        let a = self
            .rp
            .query()
            .music_album(id)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        Ok(Collection {
            kind: "album".into(),
            id: a.id,
            playlist_id: a.playlist_id,
            name: a.name,
            cover: best_thumb(&a.cover),
            artists: a.artists.iter().map(|x| x.name.clone()).collect(),
            artist_ids: a
                .artists
                .iter()
                .filter_map(|x| x.id.clone())
                .collect(),
            author: a.artists.first().map(|x| x.name.clone()),
            year: a.year,
            description: a.description.as_ref().map(|r| r.to_plaintext_yt_host("www.youtube.com")),
            track_count: Some(a.track_count as u64),
            album_type: Some(album_type_str(a.album_type)),
            tracks: a.tracks.iter().map(track_from).collect(),
            related: a.variants.iter().map(album_from).collect(),
            owned: false,
            saved: false,
        })
    }

    pub async fn playlist(&self, id: &str) -> Result<Collection> {
        let p = self
            .rp
            .query()
            .music_playlist(id)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        Ok(Collection {
            kind: "playlist".into(),
            id: p.id,
            playlist_id: None,
            name: p.name,
            cover: best_thumb(&p.thumbnail),
            artists: Vec::new(),
            artist_ids: Vec::new(),
            author: p.channel.as_ref().map(|c| c.name.clone()),
            year: None,
            description: p.description.as_ref().map(|r| r.to_plaintext_yt_host("www.youtube.com")),
            track_count: p.track_count,
            album_type: None,
            tracks: p.tracks.items.iter().map(track_from).collect(),
            related: p.related_playlists.items.iter().map(playlist_from).collect(),
            owned: false,
            saved: false,
        })
    }

    pub async fn radio(&self, video_id: &str) -> Result<Vec<Track>> {
        let r = match self.rp.query().music_radio_track(video_id).await {
            Ok(r) if !r.items.is_empty() => r,
            other => {
                if let Err(e) = &other {
                    eprintln!("music_radio_track: {e}");
                }
                self.rp
                    .query()
                    .music_radio(&format!("RDAMVM{video_id}"))
                    .await
                    .map_err(|e| anyhow!("{e}"))?
            }
        };
        let tracks = dedup_tracks(r.items.iter().map(track_from).collect());
        *self.radio.lock().unwrap() = Some(r);
        Ok(tracks)
    }

    /// Next page of the current radio, unique tracks only.
    pub async fn radio_continue(&self) -> Result<Vec<Track>> {
        let page = self.radio.lock().unwrap().clone();
        let Some(p) = page else {
            return Ok(Vec::new());
        };
        if p.is_exhausted() {
            return Ok(Vec::new());
        }
        match p.next(&self.rp.query()).await {
            Ok(Some(n)) => {
                let tracks = dedup_tracks(n.items.iter().map(track_from).collect());
                *self.radio.lock().unwrap() = Some(n);
                Ok(tracks)
            }
            Ok(None) => {
                *self.radio.lock().unwrap() = None;
                Ok(Vec::new())
            }
            Err(e) => Err(anyhow!("{e}")),
        }
    }

    pub async fn radio_playlist(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let r = self
            .rp
            .query()
            .music_radio_playlist(playlist_id)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        Ok(r.items.iter().map(track_from).collect())
    }

    pub async fn radio_id(&self, radio_id: &str) -> Result<Vec<Track>> {
        let r = self
            .rp
            .query()
            .music_radio(radio_id)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        Ok(r.items.iter().map(track_from).collect())
    }

    pub async fn lyrics(&self, video_id: &str) -> Result<Lyrics> {
        let details = self
            .rp
            .query()
            .music_details(video_id)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        let Some(lid) = details.lyrics_id.clone() else {
            return Ok(Lyrics::default());
        };
        if let Ok(Some(timed)) = self.it().timed_lyrics(&lid).await {
            if !timed.lines.is_empty() {
                return Ok(timed);
            }
        }
        match self.rp.query().music_lyrics(&lid).await {
            Ok(l) => Ok(innertube::lyrics_from_body(&l.body, &l.footer)),
            Err(e) => Err(anyhow!("{e}")),
        }
    }

    pub async fn related(&self, video_id: &str) -> Result<Related> {
        let details = self
            .rp
            .query()
            .music_details(video_id)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        let Some(rid) = details.related_id else {
            return Ok(Related::default());
        };
        let r = self
            .rp
            .query()
            .music_related(rid)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        Ok(Related {
            tracks: dedup_tracks(r.tracks.iter().map(track_from).collect()),
            versions: dedup_tracks(r.other_versions.iter().map(track_from).collect()),
            albums: r.albums.iter().map(album_from).collect(),
            artists: r.artists.iter().map(artist_from).collect(),
            playlists: r.playlists.iter().map(playlist_from).collect(),
        })
    }

    pub async fn account(&self) -> Result<Option<AccountInfo>> {
        self.it().account_menu().await
    }

    pub async fn like(&self, video_id: &str, action: &str) -> Result<LikeState> {
        self.it().like(video_id, action).await
    }

    pub async fn subscribe(&self, channel_id: &str, on: bool) -> Result<()> {
        self.it().subscribe(channel_id, on).await
    }

    pub async fn playlist_create(&self, title: &str) -> Result<String> {
        self.it().playlist_create(title).await
    }

    pub async fn playlist_add(&self, playlist_id: &str, video_id: &str) -> Result<()> {
        self.it().playlist_add(playlist_id, video_id).await
    }

    pub async fn playlist_remove(
        &self,
        playlist_id: &str,
        video_id: &str,
        set_video_id: Option<&str>,
    ) -> Result<()> {
        self.it()
            .playlist_remove(playlist_id, video_id, set_video_id)
            .await
    }

    pub async fn playlist_rename(&self, playlist_id: &str, title: &str) -> Result<()> {
        self.it().playlist_rename(playlist_id, title).await
    }

    pub async fn library_save(&self, id: &str, save: bool) -> Result<()> {
        self.it().library_save(id, save).await
    }

    pub async fn library(&self) -> Result<Library> {
        if self.cookie_header().is_none() {
            return Ok(Library {
                signed_in: false,
                ..Default::default()
            });
        }
        let account = self.account().await.ok().flatten();
        let mut lib = Library {
            signed_in: true,
            mock: false,
            account,
            ..Default::default()
        };
        if let Ok(p) = self.rp.query().music_saved_playlists().await {
            lib.playlists = p.items.iter().map(playlist_from).collect();
        }
        if let Ok(a) = self.rp.query().music_saved_albums().await {
            lib.albums = a.items.iter().map(album_from).collect();
        }
        if let Ok(a) = self.rp.query().music_saved_artists().await {
            lib.artists = a.items.iter().map(artist_from).collect();
        }
        if let Ok(liked) = self.rp.query().music_liked_tracks().await {
            lib.liked = liked.tracks.items.iter().map(track_from).collect();
        } else if let Ok(t) = self.rp.query().music_saved_tracks().await {
            lib.liked = t.items.iter().map(track_from).collect();
        }
        if let Ok(h) = self.rp.query().music_history().await {
            lib.recents = h
                .items
                .iter()
                .map(|hi| track_from(&hi.item))
                .collect();
        }
        Ok(lib)
    }

    pub async fn history(&self) -> Result<Vec<Track>> {
        let h = self
            .rp
            .query()
            .music_history()
            .await
            .map_err(|e| anyhow!("{e}"))?;
        Ok(h.items.iter().map(|hi| track_from(&hi.item)).collect())
    }

    /// Resolve a playable audio stream URL.
    ///
    /// rustypipe (iOS/TV clients, no PO token) is tried first so play can
    /// start in-process without spawning yt-dlp. The sidecar is only used
    /// if rustypipe fails, and never on Android/iOS. Resolved URLs are
    /// cached for a few hours (YouTube's googlevideo links last ~6 h).
    pub async fn stream(&self, video_id: &str) -> Result<Stream> {
        if let Some(s) = self.cached_stream(video_id) {
            return Ok(s);
        }
        let s = self.resolve_stream(video_id).await?;
        self.put_stream(video_id, s.clone());
        Ok(s)
    }

    fn cached_stream(&self, id: &str) -> Option<Stream> {
        let mut g = self.streams.lock().unwrap();
        if let Some(c) = g.get(id) {
            if c.at.elapsed() < STREAM_TTL {
                return Some(c.stream.clone());
            }
        }
        g.remove(id);
        None
    }

    fn put_stream(&self, id: &str, stream: Stream) {
        self.streams.lock().unwrap().insert(
            id.to_string(),
            CachedStream {
                stream,
                at: Instant::now(),
            },
        );
    }

    fn evict_stream(&self, id: &str) {
        self.streams.lock().unwrap().remove(id);
    }

    async fn resolve_stream(&self, video_id: &str) -> Result<Stream> {
        match rustypipe_stream(&self.rp, video_id).await {
            Ok(s) => return Ok(s),
            Err(e) => eprintln!("rustypipe stream: {e}"),
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let cookies = if self.cookies_file.exists() {
                Some(self.cookies_file.clone())
            } else {
                None
            };
            let q = self.quality();
            return ytdlp_stream(video_id, cookies.as_deref(), &q).await;
        }
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            Err(anyhow!("no playable stream"))
        }
    }

    /// Start a progressive download and return once enough of the file is
    /// present to probe and begin decoding (see `READY_BYTES`). The rest
    /// continues in the background.
    pub async fn start_audio(
        &self,
        video_id: &str,
        stream: &Stream,
    ) -> Result<std::sync::Arc<crate::decode::GrowingBody>> {
        if !video_id.is_empty() {
            if let Some(bytes) = self.cache.read_complete(video_id) {
                let body = crate::decode::GrowingBody::from_complete(bytes);
                self.bodies
                    .lock()
                    .unwrap()
                    .insert(stream.url.clone(), body.clone());
                return Ok(body);
            }
        }
        if let Some(existing) = self.existing_body(&stream.url) {
            existing.wait_ready().await?;
            return Ok(existing);
        }
        let body = self.spawn_download(video_id, stream);
        match body.wait_ready().await {
            Ok(()) => Ok(body),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("403") {
                    self.evict_stream(video_id);
                    self.bodies.lock().unwrap().remove(&stream.url);
                    let stream = self.resolve_stream(video_id).await?;
                    self.put_stream(video_id, stream.clone());
                    let body = self.spawn_download(video_id, &stream);
                    body.wait_ready().await?;
                    return Ok(body);
                }
                Err(e)
            }
        }
    }

    fn existing_body(
        &self,
        url: &str,
    ) -> Option<std::sync::Arc<crate::decode::GrowingBody>> {
        let b = self.bodies.lock().unwrap().get(url).cloned()?;
        if b.error().is_some() {
            return None;
        }
        Some(b)
    }

    fn spawn_download(
        &self,
        video_id: &str,
        stream: &Stream,
    ) -> std::sync::Arc<crate::decode::GrowingBody> {
        if let Some(existing) = self.existing_body(&stream.url) {
            return existing;
        }
        let body = crate::decode::GrowingBody::new();
        // Resume from a partial on-disk download if we have one.
        if !video_id.is_empty() {
            if let Some((path, _)) = self.cache.partial(video_id) {
                if let Ok(bytes) = std::fs::read(&path) {
                    body.append(&bytes);
                }
            }
        }
        {
            let mut g = self.bodies.lock().unwrap();
            g.insert(stream.url.clone(), body.clone());
            if g.len() > 10 {
                let stale: Vec<String> = g
                    .iter()
                    .filter(|(_, b)| b.is_done())
                    .map(|(k, _)| k.clone())
                    .take(g.len() - 8)
                    .collect();
                for k in stale {
                    g.remove(&k);
                }
            }
        }
        let stream = stream.clone();
        let body2 = body.clone();
        let cache = self.cache.clone();
        let id = video_id.to_string();
        tokio::spawn(async move {
            let c = if id.is_empty() {
                None
            } else {
                Some((cache, id))
            };
            if let Err(e) = download_into(&stream, &body2, c).await {
                body2.fail(e.to_string());
            } else {
                body2.finish();
            }
        });
        body
    }

    /// Resolve the stream and fully download it into the disk cache.
    pub async fn prefetch(&self, video_id: &str) {
        if self.cache.has_complete(video_id) {
            return;
        }
        let Ok(stream) = self.stream(video_id).await else {
            return;
        };
        if let Some(existing) = self.existing_body(&stream.url) {
            let _ = existing.wait_done().await;
            return;
        }
        let body = self.spawn_download(video_id, &stream);
        let _ = body.wait_done().await;
    }

    /// Wait for a complete download (diagnostics).
    pub async fn fetch_audio(&self, stream: &Stream) -> Result<std::sync::Arc<Vec<u8>>> {
        let body = self.start_audio("", stream).await?;
        body.wait_done().await?;
        Ok(std::sync::Arc::new(body.snapshot()))
    }
}

#[derive(Debug, Clone)]
pub struct Stream {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub duration: Option<f64>,
    pub source: &'static str,
    pub mime: Option<String>,
}

fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("tarn")
}

fn tighten_cache(dir: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_file() {
                    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
}

pub fn track_from(t: &TrackItem) -> Track {
    Track {
        id: t.id.clone(),
        title: t.name.clone(),
        artists: t.artists.iter().map(|a| a.name.clone()).collect(),
        artist_ids: t.artists.iter().filter_map(|a| a.id.clone()).collect(),
        album: t.album.as_ref().map(|a| a.name.clone()),
        album_id: t.album.as_ref().map(|a| a.id.clone()),
        duration: t.duration.map(normalize_duration),
        cover: best_thumb(&t.cover),
        is_video: matches!(t.track_type, TrackType::Video),
    }
}

fn artist_from(a: &ArtistItem) -> ArtistCard {
    ArtistCard {
        id: a.id.clone(),
        name: a.name.clone(),
        cover: best_thumb(&a.avatar),
        subscribers: a.subscriber_count,
    }
}

fn playlist_from(p: &MusicPlaylistItem) -> PlaylistCard {
    PlaylistCard {
        id: p.id.clone(),
        name: p.name.clone(),
        cover: best_thumb(&p.thumbnail),
        track_count: p.track_count,
        subtitle: p.channel.as_ref().map(|c| c.name.clone()),
        author: p.channel.as_ref().map(|c| c.name.clone()),
        from_ytm: Some(p.from_ytm),
        ..Default::default()
    }
}

fn album_from(a: &AlbumItem) -> PlaylistCard {
    PlaylistCard {
        id: a.id.clone(),
        name: a.name.clone(),
        cover: best_thumb(&a.cover),
        track_count: None,
        subtitle: Some(
            a.artists
                .iter()
                .map(|x| x.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        year: a.year,
        album_type: Some(album_type_str(a.album_type)),
        ..Default::default()
    }
}

fn track_item(t: &TrackItem) -> ShelfItem {
    let tr = track_from(t);
    ShelfItem {
        kind: if tr.is_video { "video" } else { "track" }.into(),
        id: tr.id,
        title: tr.title,
        subtitle: Some(tr.artists.join(", ")),
        cover: tr.cover,
        artists: tr.artists,
        artist_ids: tr.artist_ids,
        album: tr.album,
        album_id: tr.album_id,
        duration: tr.duration,
        is_video: tr.is_video,
        ..Default::default()
    }
}

fn artist_item(a: &ArtistItem) -> ShelfItem {
    ShelfItem {
        kind: "artist".into(),
        id: a.id.clone(),
        title: a.name.clone(),
        cover: best_thumb(&a.avatar),
        ..Default::default()
    }
}

fn playlist_item(p: &MusicPlaylistItem) -> ShelfItem {
    ShelfItem {
        kind: "playlist".into(),
        id: p.id.clone(),
        title: p.name.clone(),
        subtitle: p.channel.as_ref().map(|c| c.name.clone()),
        cover: best_thumb(&p.thumbnail),
        track_count: p.track_count,
        ..Default::default()
    }
}

fn album_item(a: &AlbumItem) -> ShelfItem {
    ShelfItem {
        kind: "album".into(),
        id: a.id.clone(),
        title: a.name.clone(),
        subtitle: Some(
            a.artists
                .iter()
                .map(|x| x.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        cover: best_thumb(&a.cover),
        year: a.year,
        album_type: Some(album_type_str(a.album_type)),
        artists: a.artists.iter().map(|x| x.name.clone()).collect(),
        artist_ids: a.artists.iter().filter_map(|x| x.id.clone()).collect(),
        ..Default::default()
    }
}

fn music_item_to_shelf(i: &MusicItem) -> ShelfItem {
    match i {
        MusicItem::Track(t) => track_item(t),
        MusicItem::Album(a) => album_item(a),
        MusicItem::Artist(a) => artist_item(a),
        MusicItem::Playlist(p) => playlist_item(p),
        MusicItem::User(u) => ShelfItem {
            kind: "artist".into(),
            id: u.id.clone(),
            title: u.name.clone(),
            cover: best_thumb(&u.avatar),
            ..Default::default()
        },
    }
}

fn album_type_str(t: AlbumType) -> String {
    match t {
        AlbumType::Ep => "EP",
        AlbumType::Single => "Single",
        AlbumType::Audiobook => "Audiobook",
        AlbumType::Show => "Show",
        _ => "Album",
    }
    .into()
}

fn argb_to_hex(c: u32) -> String {
    format!("#{:06X}", c & 0x00FF_FFFF)
}

fn best_thumb(thumbs: &[Thumbnail]) -> Option<String> {
    let t = thumbs.iter().max_by_key(|t| t.width.max(t.height))?;
    Some(upgrade_cover(&t.url))
}

fn best_thumb_wide(thumbs: &[Thumbnail]) -> Option<String> {
    let t = thumbs.iter().max_by_key(|t| t.width)?;
    // banners look better a bit wider
    let u = &t.url;
    if u.contains("googleusercontent.com") || u.contains("ggpht.com") {
        if let Some(i) = u.rfind('=') {
            return Some(format!("{}=w1280-h400-p-l90-rj", &u[..i]));
        }
    }
    Some(upgrade_cover(u))
}

pub fn upgrade_cover(url: &str) -> String {
    let u = url.to_string();
    if u.contains("googleusercontent.com") || u.contains("ggpht.com") {
        if let Some(i) = u.rfind('=') {
            return format!("{}=w512-h512-l90-rj", &u[..i]);
        }
    }
    if u.contains("i.ytimg.com") && u.contains("/default.jpg") {
        return u.replace("/default.jpg", "/hqdefault.jpg");
    }
    u
}

fn normalize_duration(d: u32) -> u32 {
    if d > 10_000 {
        d / 1000
    } else {
        d
    }
}

fn dedup_tracks(tracks: Vec<Track>) -> Vec<Track> {
    let mut seen = std::collections::HashSet::new();
    tracks
        .into_iter()
        .filter(|t| !t.id.is_empty() && seen.insert(t.id.clone()))
        .collect()
}

pub fn seed_first(mut tracks: Vec<Track>, seed: Option<Track>, id: &str) -> Vec<Track> {
    tracks = dedup_tracks(tracks);
    if let Some(pos) = tracks.iter().position(|t| t.id == id) {
        if pos != 0 {
            let t = tracks.remove(pos);
            tracks.insert(0, t);
        }
    } else if let Some(s) = seed {
        if s.id == id {
            tracks.insert(0, s);
        }
    }
    tracks
}

async fn download_into(
    stream: &Stream,
    body: &crate::decode::GrowingBody,
    cache: Option<(std::sync::Arc<crate::cache::AudioCache>, String)>,
) -> Result<()> {
    let mut last_err = None;
    for attempt in 0..6u32 {
        let offset = body.len() as u64;
        if let Some((cache, id)) = &cache {
            let _ = cache.write_part(id, &body.snapshot());
        }
        match download_range(stream, body, offset, cache.as_ref()).await {
            Ok(()) => {
                if let Some((cache, id)) = &cache {
                    if let Err(e) = cache.commit(id, &body.snapshot()) {
                        eprintln!("cache commit: {e}");
                    }
                }
                return Ok(());
            }
            Err(e) => {
                last_err = Some(e);
                let wait = Duration::from_millis(400 * (1u64 << attempt.min(4)));
                eprintln!(
                    "audio download retry {}: {}; backing off {wait:?}",
                    attempt + 1,
                    last_err.as_ref().unwrap()
                );
                tokio::time::sleep(wait).await;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("audio download failed")))
}

async fn download_range(
    stream: &Stream,
    body: &crate::decode::GrowingBody,
    offset: u64,
    cache: Option<&(std::sync::Arc<crate::cache::AudioCache>, String)>,
) -> Result<()> {
    // googlevideo stalls on HTTP/2 and throttles without a Range request.
    let client = reqwest::Client::builder()
        .user_agent(user_agent())
        .http1_only()
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(180))
        .build()?;
    let mut req = client.get(&stream.url);
    for (k, v) in &stream.headers {
        if k.eq_ignore_ascii_case("range") {
            continue;
        }
        req = req.header(k.as_str(), v.as_str());
    }
    req = req.header("Range", format!("bytes={offset}-"));
    let resp = req.send().await.context("audio download")?;
    let status = resp.status();
    if status.as_u16() == 416 {
        // Already have the whole file.
        if body.len() >= 64 {
            return Ok(());
        }
        return Err(anyhow!("audio download: HTTP 416"));
    }
    if !status.is_success() {
        return Err(anyhow!("audio download: HTTP {status}"));
    }
    if let Some(cr) = resp.headers().get(reqwest::header::CONTENT_RANGE) {
        if let Ok(s) = cr.to_str() {
            if let Some(total) = s.rsplit('/').next() {
                if let Ok(n) = total.parse::<u64>() {
                    body.set_content_len(n);
                }
            }
        }
    } else if let Some(len) = resp.content_length() {
        body.set_content_len(len.saturating_add(offset));
    }
    let mut resp = resp;
    let mut n = 0usize;
    while let Some(chunk) = resp.chunk().await.context("audio body")? {
        n += chunk.len();
        body.append(&chunk);
        if let Some((cache, id)) = cache {
            let _ = cache.append_part(id, &chunk);
        }
    }
    if body.len() < 64 {
        return Err(anyhow!("audio download too small ({} bytes)", body.len()));
    }
    eprintln!(
        "audio: +{n} bytes (total {}) from {} ({})",
        body.len(),
        stream.source,
        stream.mime.as_deref().unwrap_or("?")
    );
    Ok(())
}

async fn rustypipe_stream(rp: &RustyPipe, id: &str) -> Result<Stream> {
    // iOS and TV clients return unobfuscated URLs and do not need a PO token.
    let player = rp
        .query()
        .player_from_clients(id, &[ClientType::Ios, ClientType::Tv])
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let stream = pick_audio(&player.audio_streams).ok_or_else(|| {
        anyhow!(
            "no audio streams (client={:?}, videos={})",
            player.client_type,
            player.video_streams.len()
        )
    })?;
    Ok(Stream {
        url: stream.url.clone(),
        headers: vec![("User-Agent".into(), user_agent())],
        duration: stream.duration_ms.map(|ms| ms as f64 / 1000.0),
        source: "rustypipe",
        mime: Some(stream.mime.clone()),
    })
}

fn pick_audio(streams: &[rustypipe::model::AudioStream]) -> Option<&rustypipe::model::AudioStream> {
    // Prefer AAC/m4a: rodio+symphonia decodes it in-process. Opus/webm is a
    // weaker fallback because we do not ship a native opus decoder.
    let usable = |s: &&rustypipe::model::AudioStream| s.drm_systems.is_empty() && !s.url.is_empty();
    let aac = |s: &rustypipe::model::AudioStream| {
        matches!(s.codec, AudioCodec::Mp4a)
            || matches!(s.format, AudioFormat::M4a)
            || s.mime.to_ascii_lowercase().contains("mp4")
            || s.mime.to_ascii_lowercase().contains("aac")
    };
    streams
        .iter()
        .filter(usable)
        .max_by_key(|s| (aac(s), s.average_bitrate.max(s.bitrate)))
}

pub fn user_agent() -> String {
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36".into()
}

fn format_for(quality: &str) -> &'static str {
    // Always prefer m4a/AAC so the in-process decoder (symphonia) can handle it.
    match quality {
        "low" => "worstaudio[ext=m4a]/worstaudio[acodec=aac]/bestaudio[ext=m4a]/bestaudio",
        "medium" => "bestaudio[abr<=140][ext=m4a]/bestaudio[ext=m4a]/bestaudio",
        _ => "bestaudio[ext=m4a]/bestaudio[acodec=aac]/bestaudio",
    }
}

async fn ytdlp_stream(
    id: &str,
    cookies: Option<&std::path::Path>,
    quality: &str,
) -> Result<Stream> {
    let bin = crate::ytdlp::ensure()
        .await
        .context("yt-dlp sidecar is not available")?;
    let url = format!("https://www.youtube.com/watch?v={id}");
    let mut cmd = Command::new(&bin);
    crate::ytdlp::silence_tokio(&mut cmd);
    cmd.arg("--no-warnings").arg("--no-playlist").arg("-f").arg(format_for(quality));
    if let Some(p) = cookies {
        cmd.arg("--cookies").arg(p);
    }
    cmd.arg("-j").arg("--").arg(&url);
    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("run yt-dlp at {}", bin.display()))?;
    if !out.status.success() {
        return Err(anyhow!(
            "yt-dlp: {}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("failed")
        ));
    }
    let v: Value = serde_json::from_slice(&out.stdout).context("yt-dlp json")?;
    let stream_url = v
        .get("url")
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow!("yt-dlp returned no url"))?
        .to_string();
    let mut headers = Vec::new();
    if let Some(obj) = v.get("http_headers").and_then(|h| h.as_object()) {
        for (k, val) in obj {
            if let Some(s) = val.as_str() {
                headers.push((k.clone(), s.to_string()));
            }
        }
    }
    if !headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
    {
        headers.push(("User-Agent".into(), user_agent()));
    }
    let duration = v.get("duration").and_then(|d| d.as_f64());
    let mime = v.get("ext").and_then(|x| x.as_str()).map(|s| s.to_string());
    Ok(Stream {
        url: stream_url,
        headers,
        duration,
        source: "yt-dlp",
        mime,
    })
}

async fn search_ytdlp(query: &str) -> Result<Vec<Track>> {
    let bin = crate::ytdlp::ensure()
        .await
        .context("yt-dlp sidecar is not available")?;
    let q = format!("ytsearch12:{query}");
    let mut cmd = Command::new(&bin);
    crate::ytdlp::silence_tokio(&mut cmd);
    let out = cmd
        .args(["--no-warnings", "--flat-playlist", "-j", "--", &q])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("run yt-dlp at {}", bin.display()))?;
    if !out.status.success() {
        return Err(anyhow!(
            "search failed: {}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("?")
        ));
    }
    let mut tracks = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        let title = v
            .get("title")
            .and_then(|x| x.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let artist = v
            .get("uploader")
            .or_else(|| v.get("channel"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let duration = v.get("duration").and_then(|d| d.as_f64()).map(|d| d as u32);
        let cover = v
            .get("thumbnails")
            .and_then(|t| t.as_array())
            .and_then(|a| a.last())
            .and_then(|t| t.get("url"))
            .and_then(|u| u.as_str())
            .map(upgrade_cover);
        tracks.push(Track {
            id,
            title,
            artists: if artist.is_empty() {
                Vec::new()
            } else {
                vec![artist]
            },
            duration,
            cover,
            ..Default::default()
        });
    }
    Ok(tracks)
}

// UserItem fields — compile will tell us if name/id/avatar differ.
