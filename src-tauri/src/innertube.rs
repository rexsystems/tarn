#![allow(clippy::collapsible_if, clippy::redundant_closure)]
//! Raw InnerTube helpers: public browse (Home, Explore), likes, lyrics, mutations.
//! Cookie values are never logged.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::auth::{sapisid_from_header, sapisidhash_now};
use crate::models::{
    AccountInfo, ArtistCard, Explore, Home, LikeState, LyricLine, Lyrics, MoodChip, PlaylistCard,
    Shelf, ShelfItem, Track,
};
use crate::ytm::user_agent;

const ORIGIN: &str = "https://music.youtube.com";
const CLIENT_NAME: &str = "WEB_REMIX";
// Keep this on a known-good WEB_REMIX build. Made-up "today" versions
// get HTTP 400 on some library browses.
const CLIENT_VERSION: &str = "1.20260516.01.00";
/// Timed lyrics only show up on the Android Music client; WEB_REMIX returns
/// plain text (or an older untimed shape) for the same browse id.
const ANDROID_MUSIC_NAME: &str = "ANDROID_MUSIC";
// Known-good for timed lyrics (ytmusicapi / ytmusic-rs). Newer builds 400.
const ANDROID_MUSIC_VERSION: &str = "7.21.50";
const ANDROID_MUSIC_CLIENT_ID: &str = "21";

#[derive(Clone)]
pub struct InnerTube {
    http: reqwest::Client,
    cookie: Option<String>,
    lang: String,
    region: String,
    page_id: Option<String>,
}

impl InnerTube {
    pub fn new(http: reqwest::Client, cookie: Option<String>, lang: String, region: String) -> Self {
        Self {
            http,
            cookie,
            lang,
            region,
            page_id: None,
        }
    }

    pub fn with_page_id(mut self, page_id: Option<String>) -> Self {
        self.page_id = page_id.filter(|s| !s.is_empty());
        self
    }

    pub fn has_cookie(&self) -> bool {
        self.cookie.as_ref().is_some_and(|c| !c.is_empty())
    }

    fn context(&self) -> Value {
        let mut ctx = json!({
            "client": {
                "clientName": CLIENT_NAME,
                "clientVersion": CLIENT_VERSION,
                "hl": self.lang,
                "gl": self.region.to_uppercase(),
            }
        });
        if let Some(pid) = &self.page_id {
            ctx["user"] = json!({ "onBehalfOfUser": pid });
        }
        ctx
    }

    /// POST with retries on transient failures (network errors, 429, 5xx).
    /// A single 503 used to abort a whole playlist / liked-songs pagination.
    pub async fn post(&self, endpoint: &str, body: Value) -> Result<Value> {
        let mut last = None;
        for attempt in 0..4u64 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(400 * attempt)).await;
            }
            match self.post_once(endpoint, body.clone()).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    let m = e.to_string();
                    let transient = m.contains("HTTP 5")
                        || m.contains("HTTP 429")
                        || m.contains("error sending request")
                        || m.contains("timed out")
                        || m.contains("connection")
                        || m.contains("innertube json");
                    if !transient {
                        return Err(e);
                    }
                    last = Some(e);
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("innertube {endpoint}: failed")))
    }

    async fn post_once(&self, endpoint: &str, mut body: Value) -> Result<Value> {
        if body.get("context").is_none() {
            body["context"] = self.context();
        }
        let url = format!("https://music.youtube.com/youtubei/v1/{endpoint}?prettyPrint=false");
        let mut req = self
            .http
            .post(&url)
            .header("User-Agent", user_agent())
            .header("Content-Type", "application/json")
            .header("Origin", ORIGIN)
            .header("Referer", "https://music.youtube.com/")
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .header("X-Origin", ORIGIN);
        if let Some(c) = &self.cookie {
            req = req.header("Cookie", c);
            if let Some(sap) = sapisid_from_header(c) {
                req = req.header("Authorization", sapisidhash_now(&sap));
            }
            // WEB_REMIX signed-in calls need AuthUser (default 0) and,
            // for a brand account, X-Goog-PageId (set by the caller on context).
            req = req.header("X-Goog-AuthUser", "0");
            if let Some(v) = crate::auth::header_value(c, "VISITOR_INFO1_LIVE") {
                req = req.header("X-Goog-Visitor-Id", v);
            }
            if let Some(pid) = &self.page_id {
                req = req.header("X-Goog-PageId", pid);
            }
        }
        let resp = req.json(&body).send().await?;
        let status = resp.status();
        // Persist rotating SIDCC / PSIDCC from Set-Cookie (values not logged).
        let set_cookies: Vec<String> = resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
            .collect();
        let text = resp.text().await.unwrap_or_default();
        let browse = body
            .get("browseId")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        crate::diag::log(format!(
            "innertube {endpoint}{browse_bit}: HTTP {status} bytes={} set-cookie={}",
            text.len(),
            set_cookies
                .iter()
                .filter_map(|s| s.split('=').next())
                .collect::<Vec<_>>()
                .join(","),
            browse_bit = if browse.is_empty() {
                String::new()
            } else {
                format!(" {browse}")
            },
        ));
        if !set_cookies.is_empty() {
            crate::auth::note_set_cookies(&set_cookies);
        }
        if !status.is_success() {
            let reason = crate::diag::yt_error_reason(&text);
            crate::diag::soft_fail(
                "innertube",
                format!("{endpoint}{browse_bit} HTTP {status} reason={reason}", browse_bit = if browse.is_empty() { String::new() } else { format!(" {browse}") }),
            );
            crate::diag::debug(format!(
                "innertube {endpoint} raw body ({}b): {}",
                text.len(),
                text.chars().take(400).collect::<String>()
            ));
            // A transient 401/403 is NOT a sign-out. Callers log and retry.
            return Err(anyhow!("innertube {endpoint}: HTTP {status} ({reason})"));
        }
        // Detect logged-out responses that still return 200.
        if text.contains("LOGIN_REQUIRED") || text.contains("loggedOut") {
            crate::diag::log(format!(
                "innertube {endpoint}: body looks logged-out (LOGIN_REQUIRED)"
            ));
        }
        serde_json::from_str(&text).map_err(|e| anyhow!("innertube json: {e}"))
    }

    pub async fn browse(&self, browse_id: &str, params: Option<&str>) -> Result<Value> {
        let mut body = json!({ "browseId": browse_id });
        if let Some(p) = params {
            if !p.is_empty() {
                body["params"] = json!(p);
            }
        }
        self.post("browse", body).await
    }

    pub async fn browse_continuation(&self, token: &str) -> Result<Value> {
        self.post("browse", json!({ "continuation": token })).await
    }

    /// The other half of a song/music-video pair (YouTube Music's Song/Video
    /// switch) and how their timelines line up, if YouTube links one.
    pub async fn counterpart(
        &self,
        video_id: &str,
    ) -> Result<Option<(String, Vec<crate::bgvideo::VideoSeg>)>> {
        let v = self
            .post(
                "next",
                json!({
                    "videoId": video_id,
                    "enablePersistentPlaylistPanel": true,
                    "isAudioOnly": true,
                }),
            )
            .await?;
        Ok(parse_counterpart(&v, video_id))
    }

    /// Tracks of an auto mix / radio playlist (RDTMAK..., RDMM..., RDCLAK...).
    /// YouTube Music plays these through `next`, not `browse`.
    pub async fn mix_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let v = self
            .post(
                "next",
                json!({
                    "playlistId": playlist_id,
                    "enablePersistentPlaylistPanel": true,
                    "isAudioOnly": true,
                    "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
                }),
            )
            .await?;
        let mut out: Vec<Track> = Vec::new();
        walk(&v, &mut |key, val| {
            if key != "playlistPanelVideoRenderer" {
                return;
            }
            let Some(id) = val["videoId"].as_str().filter(|s| s.len() == 11) else {
                return;
            };
            let title = yt_text(&val["title"]).unwrap_or_default();
            let mut artists = Vec::new();
            let mut artist_ids = Vec::new();
            let mut album = None;
            let mut album_id = None;
            if let Some(runs) = val.pointer("/longBylineText/runs").and_then(|r| r.as_array()) {
                for r in runs {
                    let t = r["text"].as_str().unwrap_or("").trim();
                    let bid = r
                        .pointer("/navigationEndpoint/browseEndpoint/browseId")
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    if t.is_empty() || t == "•" || t == "&" || t == "," {
                        continue;
                    }
                    if bid.starts_with("UC") {
                        artists.push(t.to_string());
                        artist_ids.push(bid.to_string());
                    } else if bid.starts_with("MPRE") {
                        album = Some(t.to_string());
                        album_id = Some(bid.to_string());
                    }
                }
            }
            out.push(Track {
                id: id.to_string(),
                title,
                artists,
                artist_ids,
                album,
                album_id,
                duration: duration_of(&val["lengthText"]).or_else(|| {
                    yt_text(&val["lengthText"]).and_then(|t| parse_mmss(&t))
                }),
                cover: thumb_of(&val["thumbnail"]),
                is_video: false,
            });
        });
        Ok(dedup_tracks(out))
    }

    /// Browse Home. `on_partial` is called as soon as the first browse page
    /// lands (and again after each continuation page) so the UI can paint
    /// shelves while "show more" expansions still run.
    pub async fn home_progressive<F>(&self, params: Option<&str>, mut on_partial: F) -> Result<Home>
    where
        F: FnMut(&Home),
    {
        let v = self.browse("FEmusic_home", params).await?;
        let account_hdr = response_has_account(&v);
        let mut home = parse_home(&v);
        home.continuation = find_page_continuation(&v);
        home.signed_in = self.cookie.is_some();
        if !home.shelves.is_empty() {
            on_partial(&home);
        }
        // Follow continuation pages so Listen again / Quick picks / Mixed
        // for you and the rest of the personalized feed actually arrive.
        // Soft cap: enough for a full Home without chasing empty trailing pages.
        for n in 0..8 {
            if home.shelves.len() >= 16 {
                break;
            }
            let Some(tok) = home.continuation.clone() else {
                break;
            };
            match self.browse_continuation(&tok).await {
                Ok(more) => {
                    let extra = parse_home(&more);
                    crate::diag::log(format!(
                        "home continuation {}: +{} shelves (token {} chars)",
                        n + 1,
                        extra.shelves.len(),
                        tok.len()
                    ));
                    merge_home(&mut home, extra);
                    home.continuation = find_page_continuation(&more);
                    on_partial(&home);
                }
                Err(e) => {
                    crate::diag::log(format!("home continuation {}: {e}", n + 1));
                    break;
                }
            }
        }
        // Per-shelf "show more" tokens - fill Listen again / Quick picks / etc.
        // Cap parallelism: expanding every shelf at once hammered the network
        // on startup while Home was still settling.
        let mut set = tokio::task::JoinSet::new();
        let mut expand_n = 0usize;
        for (i, s) in home.shelves.iter().enumerate() {
            if s.continuation.is_none() {
                continue;
            }
            if expand_n >= 4 {
                break;
            }
            expand_n += 1;
            let it = self.clone();
            let mut shelf = s.clone();
            set.spawn(async move {
                it.expand_shelf(&mut shelf).await;
                (i, shelf)
            });
        }
        while let Some(done) = set.join_next().await {
            if let Ok((i, shelf)) = done {
                if let Some(slot) = home.shelves.get_mut(i) {
                    *slot = shelf;
                }
            }
        }
        home.signed_in = self.cookie.is_some();
        crate::diag::log(format!(
            "home: signed_in={} account_header={} hl={} gl={} page_id={} chips={} shelves={:?}",
            home.signed_in,
            account_hdr,
            self.lang,
            self.region,
            self.page_id.as_deref().map(|_| "set").unwrap_or("none"),
            home.chips.len(),
            home.shelves
                .iter()
                .map(|s| format!("{} ({})", s.title, s.items.len()))
                .collect::<Vec<_>>()
        ));
        if self.cookie.is_some() && !account_hdr {
            crate::diag::log(
                "home: cookie was sent but the response has no account header - may be logged out",
            );
        }
        Ok(home)
    }

    pub async fn explore(&self) -> Result<Explore> {
        let v = self.browse("FEmusic_explore", None).await?;
        let account_hdr = response_has_account(&v);
        let mut ex = parse_explore(&v);
        let mut tok = find_page_continuation(&v);
        for n in 0..6 {
            let Some(t) = tok.clone() else {
                break;
            };
            match self.browse_continuation(&t).await {
                Ok(more) => {
                    let extra = parse_explore(&more);
                    crate::diag::log(format!(
                        "explore continuation {}: +{} shelves",
                        n + 1,
                        extra.shelves.len()
                    ));
                    for s in extra.shelves {
                        if !ex
                            .shelves
                            .iter()
                            .any(|h| h.title == s.title && h.items.len() == s.items.len())
                        {
                            ex.shelves.push(s);
                        }
                    }
                    if ex.moods.is_empty() {
                        ex.moods = extra.moods;
                    }
                    if ex.links.is_empty() {
                        ex.links = extra.links;
                    }
                    tok = find_page_continuation(&more);
                }
                Err(e) => {
                    crate::diag::log(format!("explore continuation {}: {e}", n + 1));
                    break;
                }
            }
        }
        for s in &mut ex.shelves {
            self.expand_shelf(s).await;
        }
        crate::diag::log(format!(
            "explore: account_header={} shelves={:?}",
            account_hdr,
            ex.shelves
                .iter()
                .map(|s| format!("{} ({})", s.title, s.items.len()))
                .collect::<Vec<_>>()
        ));
        Ok(ex)
    }

    async fn expand_shelf(&self, shelf: &mut Shelf) {
        for n in 0..4 {
            let Some(tok) = shelf.continuation.take() else {
                return;
            };
            match self.browse_continuation(&tok).await {
                Ok(v) => {
                    let (items, next) = parse_continuation_items(&v);
                    let before = shelf.items.len();
                    for it in items {
                        if it.id.is_empty() || !shelf.items.iter().any(|x| x.id == it.id) {
                            shelf.items.push(it);
                        }
                    }
                    crate::diag::log(format!(
                        "shelf '{}' continuation {}: {} -> {} items",
                        shelf.title,
                        n + 1,
                        before,
                        shelf.items.len()
                    ));
                    shelf.continuation = next;
                    if shelf.items.len() == before {
                        break;
                    }
                }
                Err(e) => {
                    crate::diag::log(format!("shelf '{}' continuation: {e}", shelf.title));
                    break;
                }
            }
        }
    }

    pub async fn charts(&self) -> Result<Home> {
        let v = self.browse("FEmusic_charts", None).await?;
        Ok(parse_home(&v))
    }

    pub async fn new_releases(&self) -> Result<Home> {
        let v = self.browse("FEmusic_new_releases", None).await?;
        Ok(parse_home(&v))
    }

    pub async fn account_menu(&self) -> Result<Option<AccountInfo>> {
        if self.cookie.is_none() {
            return Ok(None);
        }
        let v = self.post("account/account_menu", json!({})).await?;
        Ok(parse_account(&v))
    }

    pub async fn like(&self, video_id: &str, action: &str) -> Result<LikeState> {
        // action: "like" | "dislike" | "removelike"
        let endpoint = match action {
            "dislike" => "like/dislike",
            "removelike" => "like/removelike",
            _ => "like/like",
        };
        let body = json!({ "target": { "videoId": video_id } });
        self.post(endpoint, body).await?;
        Ok(match action {
            "like" => LikeState {
                liked: true,
                disliked: false,
            },
            "dislike" => LikeState {
                liked: false,
                disliked: true,
            },
            _ => LikeState {
                liked: false,
                disliked: false,
            },
        })
    }

    pub async fn subscribe(&self, channel_id: &str, on: bool) -> Result<()> {
        let ep = if on {
            "subscription/subscribe"
        } else {
            "subscription/unsubscribe"
        };
        self.post(ep, json!({ "channelIds": [channel_id] })).await?;
        Ok(())
    }

    pub async fn playlist_create(&self, title: &str) -> Result<String> {
        let v = self
            .post("playlist/create", json!({ "title": title }))
            .await?;
        v["playlistId"]
            .as_str()
            .or_else(|| v["playlistId"].get("id").and_then(|x| x.as_str()))
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("playlist create: no id"))
    }

    pub async fn playlist_add(&self, playlist_id: &str, video_id: &str) -> Result<()> {
        self.post(
            "browse/edit_playlist",
            json!({
                "playlistId": playlist_id,
                "actions": [{ "action": "ACTION_ADD_VIDEO", "addedVideoId": video_id }]
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn playlist_remove(
        &self,
        playlist_id: &str,
        video_id: &str,
        set_video_id: Option<&str>,
    ) -> Result<()> {
        let mut action = json!({
            "action": "ACTION_REMOVE_VIDEO",
            "removedVideoId": video_id
        });
        if let Some(s) = set_video_id {
            action["setVideoId"] = json!(s);
        }
        self.post(
            "browse/edit_playlist",
            json!({ "playlistId": playlist_id, "actions": [action] }),
        )
        .await?;
        Ok(())
    }

    pub async fn playlist_rename(&self, playlist_id: &str, title: &str) -> Result<()> {
        self.post(
            "browse/edit_playlist",
            json!({
                "playlistId": playlist_id,
                "actions": [{ "action": "ACTION_SET_PLAYLIST_NAME", "playlistName": title }]
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn library_save(&self, id: &str, save: bool) -> Result<()> {
        let ep = if save { "like/like" } else { "like/removelike" };
        let body = if id.starts_with("UC") {
            json!({ "target": { "channelId": id } })
        } else {
            json!({ "target": { "playlistId": id } })
        };
        self.post(ep, body).await?;
        Ok(())
    }

    /// Timed lyrics if YouTube returns them; otherwise empty (caller falls back).
    pub async fn timed_lyrics(&self, lyrics_browse_id: &str) -> Result<Option<Lyrics>> {
        let v = self.browse_android_music(lyrics_browse_id).await?;
        Ok(parse_timed_lyrics(&v))
    }

    /// Same browse endpoint as [`browse`], but with an Android Music client
    /// so timed lyric models are included in the response.
    ///
    /// Important: do **not** attach WEB browser cookies / SAPISIDHASH here.
    /// Mixing WEB auth with ANDROID_MUSIC yields HTTP 400
    /// "Request contains an invalid argument" (same as ytmusicapi #835).
    async fn browse_android_music(&self, browse_id: &str) -> Result<Value> {
        let body = json!({
            "browseId": browse_id,
            "context": {
                "client": {
                    "clientName": ANDROID_MUSIC_NAME,
                    "clientVersion": ANDROID_MUSIC_VERSION,
                    "hl": self.lang,
                    "gl": self.region.to_uppercase(),
                    "androidSdkVersion": 34,
                    "osName": "Android",
                    "osVersion": "14",
                }
            }
        });
        let url = "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false";
        crate::diag::debug(format!(
            "lyrics android browse {browse_id}: no-cookie ANDROID_MUSIC {ANDROID_MUSIC_VERSION}"
        ));
        let resp = self
            .http
            .post(url)
            .header(
                "User-Agent",
                "com.google.android.apps.youtube.music/7.21.50 (Linux; U; Android 14; en_US)",
            )
            .header("Content-Type", "application/json")
            .header("X-YouTube-Client-Name", ANDROID_MUSIC_CLIENT_ID)
            .header("X-YouTube-Client-Version", ANDROID_MUSIC_VERSION)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        crate::diag::log(format!(
            "innertube browse(android) {browse_id}: HTTP {status} bytes={}",
            text.len()
        ));
        if !status.is_success() {
            let reason = crate::diag::yt_error_reason(&text);
            crate::diag::soft_fail(
                "lyrics",
                format!("android browse {browse_id} HTTP {status} reason={reason}"),
            );
            crate::diag::debug(format!(
                "lyrics android raw ({}b): {}",
                text.len(),
                text.chars().take(400).collect::<String>()
            ));
            return Err(anyhow!("innertube browse(android): HTTP {status} ({reason})"));
        }
        crate::diag::debug(format!(
            "lyrics android ok {browse_id}: timed={}",
            text.contains("timedLyrics")
        ));
        serde_json::from_str(&text).map_err(|e| anyhow!("innertube json: {e}"))
    }
}

pub fn parse_home(v: &Value) -> Home {
    let chips = parse_chips(v);
    let mut shelves = Vec::new();
    collect_shelves(v, &mut shelves);
    let continuation = find_continuation(v);
    Home {
        chips,
        shelves,
        continuation,
        signed_in: false,
    }
}

fn merge_home(home: &mut Home, extra: Home) {
    if home.chips.is_empty() {
        home.chips = extra.chips;
    }
    for s in extra.shelves {
        if !home
            .shelves
            .iter()
            .any(|h| h.title == s.title && h.items.len() == s.items.len())
        {
            home.shelves.push(s);
        }
    }
    home.continuation = extra.continuation;
}

pub fn parse_explore(v: &Value) -> Explore {
    let mut shelves = Vec::new();
    collect_shelves(v, &mut shelves);
    let mut moods = Vec::new();
    let mut links = Vec::new();
    // Mood tiles often live in a "Moods & genres" shelf, or as navigation buttons.
    walk(v, &mut |key, val| {
        if key == "musicNavigationButtonRenderer" {
            let title = yt_text(&val["buttonText"])
                .or_else(|| yt_text(&val["text"]))
                .unwrap_or_default();
            if title.is_empty() {
                return;
            }
            let (browse_id, params) = browse_of(val);
            let color = color_of(val);
            let item = MoodChip {
                title: title.clone(),
                params,
                browse_id,
                color: color.clone(),
                selected: false,
            };
            // Top three Explore shortcuts (New releases, Charts, Moods & genres) have icons and no solid color.
            if color.is_some() {
                moods.push(ShelfItem {
                    kind: "mood".into(),
                    id: item.browse_id.clone().or(item.params.clone()).unwrap_or_default(),
                    title,
                    subtitle: None,
                    cover: None,
                    color,
                    browse_id: item.browse_id.clone(),
                    params: item.params.clone(),
                    ..Default::default()
                });
            } else if matches!(
                item.title.as_str(),
                "New releases" | "Charts" | "Moods & genres" | "New release" | "Moods and genres"
            ) {
                links.push(item);
            }
        }
    });
    // Dedup moods (they also appear inside a shelf)
    if moods.is_empty() {
        if let Some(s) = shelves.iter().find(|s| s.title.to_lowercase().contains("mood")) {
            moods = s.items.clone();
        }
    }
    Explore {
        links,
        shelves,
        moods,
    }
}

fn collect_shelves(v: &Value, out: &mut Vec<Shelf>) {
    walk(v, &mut |key, val| {
        if key == "musicCarouselShelfRenderer" || key == "musicShelfRenderer" {
            if let Some(s) = parse_shelf(val) {
                if !s.items.is_empty() && !s.title.is_empty() {
                    // Avoid pushing the same shelf twice if walk hits nested copies - by title+len
                    if !out.iter().any(|x| x.title == s.title && x.items.len() == s.items.len()) {
                        out.push(s);
                    }
                }
            }
        }
    });
}

fn parse_shelf(val: &Value) -> Option<Shelf> {
    let header = &val["header"];
    let title = yt_text(&header["musicCarouselShelfBasicHeaderRenderer"]["title"])
        .or_else(|| yt_text(&header["musicShelfHeaderRenderer"]["title"]))
        .or_else(|| yt_text(&header["title"]))
        .or_else(|| yt_text(&val["title"]))
        .unwrap_or_default();
    let subtitle = yt_text(&header["musicCarouselShelfBasicHeaderRenderer"]["strapline"])
        .or_else(|| yt_text(&header["strapline"]));
    let contents = val["contents"]
        .as_array()
        .or_else(|| val["items"].as_array());
    let mut items = Vec::new();
    if let Some(arr) = contents {
        for c in arr {
            if let Some(it) = parse_item(c) {
                items.push(it);
            }
        }
    }
    if items.is_empty() {
        return None;
    }
    let kind = infer_shelf_kind(&items);
    let continuation = contents
        .and_then(|arr| continuation_token_from_list(arr))
        .or_else(|| find_continuation(val));
    Some(Shelf {
        id: val["shelfId"].as_str().map(|s| s.to_string()),
        title,
        subtitle,
        kind,
        items,
        continuation,
    })
}

fn infer_shelf_kind(items: &[ShelfItem]) -> String {
    let mut counts = [0u32; 6]; // track album playlist artist mood video
    for i in items {
        match i.kind.as_str() {
            "track" => counts[0] += 1,
            "album" => counts[1] += 1,
            "playlist" => counts[2] += 1,
            "artist" => counts[3] += 1,
            "mood" => counts[4] += 1,
            "video" => counts[5] += 1,
            _ => {}
        }
    }
    let labels = ["tracks", "albums", "playlists", "artists", "moods", "videos"];
    let (i, n) = counts.iter().enumerate().max_by_key(|(_, n)| *n).unwrap();
    if *n == 0 || items.len() as u32 > *n && *n < items.len() as u32 / 2 + 1 {
        if items.iter().any(|x| x.kind != items[0].kind) {
            return "mixed".into();
        }
    }
    labels[i].into()
}

fn parse_item(v: &Value) -> Option<ShelfItem> {
    if let Some(r) = v.get("musicTwoRowItemRenderer") {
        return parse_two_row(r);
    }
    if let Some(r) = v.get("musicResponsiveListItemRenderer") {
        return parse_responsive(r);
    }
    if let Some(r) = v.get("musicNavigationButtonRenderer") {
        let title = yt_text(&r["buttonText"])?;
        let (browse_id, params) = browse_of(r);
        return Some(ShelfItem {
            kind: "mood".into(),
            id: browse_id
                .clone()
                .or_else(|| params.clone())
                .unwrap_or_default(),
            title,
            color: color_of(r),
            browse_id,
            params,
            ..Default::default()
        });
    }
    if v.get("musicTwoRowItemRenderer").is_none()
        && (v.get("title").is_some() || v.get("playlistItemData").is_some())
    {
        return parse_two_row(v).or_else(|| parse_responsive(v));
    }
    None
}

fn primary_browse(r: &Value) -> (Option<String>, Option<String>, Option<String>) {
    // Prefer the item's own navigationEndpoint so subtitle artist links
    // don't win over an album/playlist browse.
    let nav = r.get("navigationEndpoint").unwrap_or(r);
    let be = nav.get("browseEndpoint").or_else(|| r.get("browseEndpoint"));
    if let Some(be) = be {
        let bid = be["browseId"].as_str().map(|s| s.to_string());
        let params = be["params"].as_str().map(|s| s.to_string());
        let page = be
            .pointer("/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        return (bid, params, page);
    }
    let (b, p) = browse_of(r);
    (b, p, page_type(r))
}

/// `playlistId` of the card's own `navigationEndpoint.watchPlaylistEndpoint`
/// (how auto mixes and some playlist cards open), without the leading "VL".
fn own_watch_playlist_id(r: &Value) -> Option<String> {
    r.pointer("/navigationEndpoint/watchPlaylistEndpoint/playlistId")
        .and_then(|x| x.as_str())
        .filter(|s| s.len() > 4)
        .map(|s| s.trim_start_matches("VL").to_string())
}

fn parse_two_row(r: &Value) -> Option<ShelfItem> {
    let title = yt_text(&r["title"])?;
    let subtitle = yt_text(&r["subtitle"]);
    let cover = thumb_of(r);
    let (browse_id, params, page) = primary_browse(r);
    // What the card itself opens. Everything else inside the renderer (the
    // play overlay, the "Start radio" / "Add to playlist" menu entries) also
    // carries videoIds and playlistIds that are NOT this card's identity.
    let own_playlist = own_watch_playlist_id(r);
    let is_collection = matches!(
        page.as_deref(),
        Some("MUSIC_PAGE_TYPE_PLAYLIST")
            | Some("MUSIC_PAGE_TYPE_ALBUM")
            | Some("MUSIC_PAGE_TYPE_ARTIST")
    ) && r
        .get("navigationEndpoint")
        .is_some_and(|n| n.get("browseEndpoint").is_some())
        || own_playlist.is_some();
    // A playlist/album/artist card must stay one, even though its play
    // overlay and menu contain videoIds (it used to become a single "track").
    let video_id = if is_collection { None } else { video_id_of(r) };
    let playlist_id = playlist_id_of(r);
    let (artists, artist_ids) = artists_of(r);
    let (kind, id) = if let Some(vid) = video_id.clone() {
        let k = if page.as_deref() == Some("MUSIC_PAGE_TYPE_VIDEO")
            || subtitle
                .as_deref()
                .map(|s| s.to_lowercase().contains("video"))
                .unwrap_or(false)
        {
            "video"
        } else {
            "track"
        };
        (k, vid)
    } else if page.as_deref() == Some("MUSIC_PAGE_TYPE_ARTIST")
        || browse_id
            .as_deref()
            .map(|s| s.starts_with("UC"))
            .unwrap_or(false)
    {
        ("artist", browse_id.clone().unwrap_or_default())
    } else if page.as_deref() == Some("MUSIC_PAGE_TYPE_ALBUM")
        || browse_id
            .as_deref()
            .map(|s| s.starts_with("MPRE") || s.starts_with("FEmusic_library_corpus_container"))
            .unwrap_or(false)
    {
        (
            "album",
            browse_id
                .clone()
                .or(playlist_id.clone())
                .unwrap_or_default(),
        )
    } else if page.as_deref() == Some("MUSIC_PAGE_TYPE_PLAYLIST")
        || own_playlist.is_some()
        || playlist_id.is_some()
    {
        // Prefer the card's own destination (its browse id / watch-playlist
        // id) over the first playlistId found anywhere, which is often the
        // radio (RDAMPL...) from the menu.
        let own_browse = if page.as_deref() == Some("MUSIC_PAGE_TYPE_PLAYLIST") {
            browse_id
                .as_ref()
                .map(|b| b.trim_start_matches("VL").to_string())
        } else {
            None
        };
        (
            "playlist",
            own_browse
                .or(own_playlist.clone())
                .or_else(|| playlist_id.clone())
                .or_else(|| {
                    browse_id
                        .as_ref()
                        .map(|b| b.trim_start_matches("VL").to_string())
                })
                .unwrap_or_default(),
        )
    } else {
        (
            "playlist",
            browse_id
                .clone()
                .or(playlist_id.clone())
                .unwrap_or_default(),
        )
    };
    if id.is_empty() && title.is_empty() {
        return None;
    }
    Some(ShelfItem {
        kind: kind.into(),
        id,
        title,
        subtitle,
        cover,
        artists,
        artist_ids,
        playlist_id,
        browse_id,
        params,
        is_video: kind == "video",
        ..Default::default()
    })
}

fn parse_responsive(r: &Value) -> Option<ShelfItem> {
    let mut cols: Vec<String> = Vec::new();
    if let Some(flex) = r["flexColumns"].as_array() {
        for c in flex {
            if let Some(t) = yt_text(&c["musicResponsiveListItemFlexColumnRenderer"]["text"])
                .or_else(|| yt_text(c))
            {
                cols.push(t);
            }
        }
    }
    let title = cols.first().cloned().or_else(|| yt_text(&r["title"]))?;
    let subtitle = cols.get(1).cloned();
    let album = cols.get(2).cloned();
    let cover = thumb_of(r);
    let video_id = video_id_of(r).or_else(|| {
        r["playlistItemData"]["videoId"]
            .as_str()
            .map(|s| s.to_string())
    });
    let (browse_id, params) = browse_of(r);
    let page = page_type(r);
    let (artists, artist_ids) = artists_of(r);
    let duration = duration_of(r);
    if let Some(vid) = video_id {
        let artists = if artists.is_empty() {
            subtitle
                .as_ref()
                .map(|s| {
                    s.split('•')
                        .next()
                        .unwrap_or(s)
                        .split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty() && !x.chars().all(|c| c.is_ascii_digit() || c == '.' || c == 'B' || c == 'M' || c == 'K' || c == ' ' || c == 'p' || c == 'l' || c == 'a' || c == 'y' || c == 's'))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            artists
        };
        return Some(ShelfItem {
            kind: "track".into(),
            id: vid,
            title,
            subtitle,
            cover,
            artists,
            artist_ids,
            album,
            duration,
            playlist_id: playlist_id_of(r),
            browse_id,
            params,
            ..Default::default()
        });
    }
    // artist / album / playlist row
    let (kind, id) = if page.as_deref() == Some("MUSIC_PAGE_TYPE_ARTIST")
        || browse_id.as_deref().is_some_and(|s| s.starts_with("UC"))
    {
        ("artist", browse_id.clone().unwrap_or_default())
    } else if page.as_deref() == Some("MUSIC_PAGE_TYPE_ALBUM") {
        ("album", browse_id.clone().unwrap_or_default())
    } else {
        // The card's play button carries a watch-playlist id (often an `RD…`
        // mix built from the playlist), which queues different songs than the
        // playlist page shows. The `VL…` browseId is the playlist itself, so
        // Play from Home and opening the card give the same tracks.
        (
            "playlist",
            browse_id
                .as_deref()
                .filter(|b| b.starts_with("VL"))
                .map(|b| b.trim_start_matches("VL").to_string())
                .or_else(|| playlist_id_of(r))
                .or_else(|| browse_id.as_ref().map(|b| b.trim_start_matches("VL").to_string()))
                .unwrap_or_default(),
        )
    };
    Some(ShelfItem {
        kind: kind.into(),
        id,
        title,
        subtitle,
        cover,
        artists,
        artist_ids,
        browse_id,
        params,
        ..Default::default()
    })
}

fn parse_chips(v: &Value) -> Vec<MoodChip> {
    let mut out = Vec::new();
    walk(v, &mut |key, val| {
        if key == "chipCloudChipRenderer" {
            let title = yt_text(&val["text"]).unwrap_or_default();
            if title.is_empty() {
                return;
            }
            let (browse_id, params) = browse_of(val);
            let selected = val["isSelected"].as_bool().unwrap_or(false);
            if !out.iter().any(|c: &MoodChip| c.title == title) {
                out.push(MoodChip {
                    title,
                    params,
                    browse_id,
                    color: None,
                    selected,
                });
            }
        }
    });
    out
}

fn is_promo_account_name(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    n.is_empty()
        || n.contains("premium")
        || n.contains("sign in")
        || n.contains("log in")
        || n.contains("upgrade")
        || n == "guest"
}

fn parse_account(v: &Value) -> Option<AccountInfo> {
    let mut name = None;
    let mut photo = None;
    let mut handle = None;
    let mut page_id = None;
    walk(v, &mut |key, val| {
        if key == "accountName" || key == "accountNameButtonRenderer" {
            if name.is_none() {
                if let Some(t) = yt_text(val) {
                    if !is_promo_account_name(&t) {
                        name = Some(t);
                    }
                }
            }
        }
        if page_id.is_none() && (key == "pageId" || key == "onBehalfOfUser") {
            if let Some(pid) = val.as_str() {
                if pid.len() >= 10 && pid.chars().all(|c| c.is_ascii_digit()) {
                    page_id = Some(pid.to_string());
                }
            }
        }
        if key == "accountPhoto" || key == "thumbnails" {
            if photo.is_none() {
                photo = thumb_of(val).or_else(|| {
                    val.as_array()
                        .and_then(|a| a.last())
                        .and_then(|t| t["url"].as_str().map(|s| s.to_string()))
                });
            }
        }
        if key == "channelHandle" || key == "accountNameSecondary" {
            if handle.is_none() {
                handle = yt_text(val);
            }
        }
    });
    // Fallback: first reasonably short runs text
    if name.is_none() {
        walk(v, &mut |key, val| {
            if name.is_none() && key == "runs" {
                if let Some(t) = yt_text(&json!({"runs": val.clone()})) {
                    if t.len() < 80 && !t.contains("http") && !is_promo_account_name(&t) {
                        name = Some(t);
                    }
                }
            }
        });
    }
    // Logged-out account_menu often only has the Premium CTA - treat as no account.
    let name = name.filter(|n| !is_promo_account_name(n))?;
    Some(AccountInfo {
        name,
        photo,
        handle,
        page_id,
    })
}

fn parse_timed_lyrics(v: &Value) -> Option<Lyrics> {
    let mut lines = Vec::new();
    walk(v, &mut |key, val| {
        if key == "timedLyricsData" {
            if let Some(arr) = val.as_array() {
                for ln in arr {
                    if let Some(line) = timed_lyric_line(ln) {
                        lines.push(line);
                    }
                }
            }
        }
    });
    // Prefer the Android Music path when walk somehow missed nested keys.
    if lines.is_empty() {
        if let Some(arr) = v
            .pointer(
                "/contents/elementRenderer/newElement/type/componentType/model/timedLyricsModel/lyricsData/timedLyricsData",
            )
            .and_then(|x| x.as_array())
        {
            for ln in arr {
                if let Some(line) = timed_lyric_line(ln) {
                    lines.push(line);
                }
            }
        }
    }
    if lines.is_empty() {
        return None;
    }
    // YouTube sometimes returns timedLyricsData with no cueRange - treat
    // as plain lyrics so we don't claim sync we don't have.
    let timed = lines.iter().filter(|l| l.start_ms.is_some()).count();
    let synced = timed > 0 && timed * 2 >= lines.len();
    Some(Lyrics {
        source: None,
        lines,
        synced,
    })
}

fn timed_lyric_line(ln: &Value) -> Option<LyricLine> {
    let text = ln["lyricLine"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| yt_text(&ln["lyricLine"]))
        .unwrap_or_default();
    if text.is_empty() {
        return None;
    }
    let cue = &ln["cueRange"];
    let start = cue["startTimeMilliseconds"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| cue["startTimeMilliseconds"].as_u64().map(|n| n as u32))
        .or_else(|| cue["startTimeMilliseconds"].as_i64().map(|n| n.max(0) as u32));
    Some(LyricLine {
        text,
        start_ms: start,
    })
}

pub fn lyrics_from_body(body: &str, footer: &str) -> Lyrics {
    // Support [mm:ss] / [mm:ss.xx] LRC if present.
    let mut lines = Vec::new();
    let mut synced = false;
    for raw in body.lines() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        if let Some(rest) = raw.strip_prefix('[') {
            if let Some((ts, text)) = rest.split_once(']') {
                if let Some(ms) = parse_ts(ts) {
                    synced = true;
                    lines.push(LyricLine {
                        text: text.trim().to_string(),
                        start_ms: Some(ms),
                    });
                    continue;
                }
            }
        }
        lines.push(LyricLine {
            text: raw.to_string(),
            start_ms: None,
        });
    }
    Lyrics {
        source: if footer.is_empty() {
            None
        } else {
            Some(footer.to_string())
        },
        lines,
        synced,
    }
}

fn parse_ts(s: &str) -> Option<u32> {
    let s = s.trim();
    let (m, rest) = s.split_once(':')?;
    let m: u32 = m.parse().ok()?;
    let (sec, frac) = if let Some((a, b)) = rest.split_once('.') {
        (a, b)
    } else if let Some((a, b)) = rest.split_once(',') {
        (a, b)
    } else {
        (rest, "0")
    };
    let sec: u32 = sec.parse().ok()?;
    let mut ms = frac.chars().take(3).collect::<String>();
    while ms.len() < 3 {
        ms.push('0');
    }
    let frac: u32 = ms.parse().unwrap_or(0);
    Some(m * 60_000 + sec * 1000 + frac)
}

// ---------- JSON walking ----------

pub fn durations_by_video(v: &Value) -> std::collections::HashMap<String, u32> {
    let mut map = std::collections::HashMap::new();
    fn visit(v: &Value, map: &mut std::collections::HashMap<String, u32>) {
        match v {
            Value::Object(m) => {
                // Only pair videoId + lengthText at list-item scope, not the whole page.
                if m.contains_key("playlistItemData")
                    || m.contains_key("flexColumns")
                    || m.contains_key("fixedColumns")
                {
                    if let (Some(id), Some(d)) = (video_id_of(v), duration_of(v)) {
                        map.entry(id).or_insert(d);
                    }
                }
                for val in m.values() {
                    visit(val, map);
                }
            }
            Value::Array(a) => {
                for val in a {
                    visit(val, map);
                }
            }
            _ => {}
        }
    }
    visit(v, &mut map);
    map
}

fn ms_of(v: &Value) -> Option<f64> {
    v.as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| v.as_f64())
}

/// Counterpart video id + segment map for `video_id` in a `next` response.
fn parse_counterpart(v: &Value, video_id: &str) -> Option<(String, Vec<crate::bgvideo::VideoSeg>)> {
    let mut found = None;
    walk(v, &mut |key, val| {
        if found.is_some() || key != "playlistPanelVideoWrapperRenderer" {
            return;
        }
        let primary = val
            .pointer("/primaryRenderer/playlistPanelVideoRenderer/videoId")
            .and_then(|x| x.as_str());
        if primary != Some(video_id) {
            return;
        }
        let Some(list) = val.get("counterpart").and_then(|x| x.as_array()) else {
            return;
        };
        for c in list {
            let Some(id) = c
                .pointer("/counterpartRenderer/playlistPanelVideoRenderer/videoId")
                .and_then(|x| x.as_str())
                .filter(|id| id.len() == 11 && *id != video_id)
            else {
                continue;
            };
            let mut segs = Vec::new();
            if let Some(arr) = c.pointer("/segmentMap/segment").and_then(|x| x.as_array()) {
                for g in arr {
                    let (Some(p), Some(q), Some(d)) = (
                        ms_of(&g["primaryVideoStartTimeMilliseconds"]),
                        ms_of(&g["counterpartVideoStartTimeMilliseconds"]),
                        ms_of(&g["durationMilliseconds"]),
                    ) else {
                        continue;
                    };
                    if d > 0.0 {
                        segs.push(crate::bgvideo::VideoSeg {
                            primary_ms: p,
                            counterpart_ms: q,
                            duration_ms: d,
                        });
                    }
                }
            }
            found = Some((id.to_string(), segs));
            return;
        }
    });
    found
}

fn walk<F: FnMut(&str, &Value)>(v: &Value, f: &mut F) {
    match v {
        Value::Object(m) => {
            for (k, val) in m {
                f(k, val);
                walk(val, f);
            }
        }
        Value::Array(a) => {
            for val in a {
                walk(val, f);
            }
        }
        _ => {}
    }
}

fn yt_text(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        return Some(s.to_string());
    }
    if let Some(s) = v.get("simpleText").and_then(|x| x.as_str()) {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    if let Some(runs) = v.get("runs").and_then(|x| x.as_array()) {
        let s: String = runs
            .iter()
            .filter_map(|r| r.get("text")?.as_str())
            .collect();
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    if let Some(label) = v
        .pointer("/accessibility/accessibilityData/label")
        .and_then(|x| x.as_str())
    {
        let s = label.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

fn thumb_of(v: &Value) -> Option<String> {
    let mut best: Option<(u64, String)> = None;
    walk(v, &mut |key, val| {
        if key == "thumbnails" {
            if let Some(arr) = val.as_array() {
                for t in arr {
                    if let Some(url) = t["url"].as_str() {
                        if url.starts_with("http")
                            && !url.contains("music.youtube.com/image/")
                            && !url.contains("/mixart")
                        {
                            let w = t["width"].as_u64().unwrap_or(0)
                                + t["height"].as_u64().unwrap_or(0);
                            if best.as_ref().map(|(bw, _)| w > *bw).unwrap_or(true) {
                                best = Some((w, url.to_string()));
                            }
                        }
                    }
                }
            }
        }
    });
    // Keep the API URL as-is; frontend sizedCover picks a display size.
    // Rewriting to =w512 here used to 404 many album arts.
    best.map(|(_, u)| u)
}

fn video_id_of(v: &Value) -> Option<String> {
    let mut found = None;
    // Prefer playlistItemData / watchEndpoint
    if let Some(s) = v.pointer("/playlistItemData/videoId").and_then(|x| x.as_str()) {
        return Some(s.to_string());
    }
    // Greyed-out / unavailable rows often only expose removedVideoId in the
    // menu (ytmusicapi does the same). YTM still counts them in Liked Music.
    if let Some(s) = v
        .pointer("/playlistItemData/removedVideoId")
        .and_then(|x| x.as_str())
    {
        if s.len() == 11 {
            return Some(s.to_string());
        }
    }
    walk(v, &mut |key, val| {
        if found.is_none() && (key == "videoId" || key == "removedVideoId") {
            if let Some(s) = val.as_str() {
                if s.len() == 11 {
                    found = Some(s.to_string());
                }
            }
        }
    });
    found
}

fn playlist_id_of(v: &Value) -> Option<String> {
    let mut found = None;
    walk(v, &mut |key, val| {
        if found.is_none() && (key == "playlistId" || key == "playlistSetVideoId") {
            if key == "playlistId" {
                if let Some(s) = val.as_str() {
                    if s.len() > 4 && !s.starts_with("RDAMVM") {
                        // RDAMVM is a radio derived from a video - still useful as playlist_id
                        found = Some(s.trim_start_matches("VL").to_string());
                    }
                }
            }
        }
    });
    found
}

fn browse_of(v: &Value) -> (Option<String>, Option<String>) {
    let mut browse = None;
    let mut params = None;
    walk(v, &mut |key, val| {
        if key == "browseEndpoint" {
            if browse.is_none() {
                browse = val["browseId"].as_str().map(|s| s.to_string());
                params = val["params"].as_str().map(|s| s.to_string());
            }
        }
        if key == "navigationEndpoint" && browse.is_none() {
            if let Some(b) = val.get("browseEndpoint") {
                browse = b["browseId"].as_str().map(|s| s.to_string());
                params = b["params"].as_str().map(|s| s.to_string());
            }
        }
    });
    (browse, params)
}

fn page_type(v: &Value) -> Option<String> {
    let mut p = None;
    walk(v, &mut |key, val| {
        if key == "pageType" {
            if p.is_none() {
                p = val.as_str().map(|s| s.to_string());
            }
        }
    });
    p
}

fn artists_of(v: &Value) -> (Vec<String>, Vec<String>) {
    let mut names = Vec::new();
    let mut ids = Vec::new();
    walk(v, &mut |key, val| {
        if key == "browseEndpoint" {
            let page = val
                .pointer("/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let id = val["browseId"].as_str().unwrap_or("");
            if page == "MUSIC_PAGE_TYPE_ARTIST" || id.starts_with("UC") {
                // name is not on the endpoint; filled from runs below
                if !id.is_empty() && !ids.contains(&id.to_string()) {
                    ids.push(id.to_string());
                }
            }
        }
    });
    // subtitle runs often list artist names
    if let Some(runs) = v
        .pointer("/subtitle/runs")
        .or_else(|| v.pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text/runs"))
        .and_then(|x| x.as_array())
    {
        for r in runs {
            let t = r["text"].as_str().unwrap_or("").trim();
            if t.is_empty() || t == "•" || t == "&" || t == "," {
                continue;
            }
            if t.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.') {
                continue;
            }
            let low = t.to_ascii_lowercase();
            if low.contains("play") || low.contains("song") || low.contains("view") {
                continue;
            }
            let bid = r
                .pointer("/navigationEndpoint/browseEndpoint/browseId")
                .and_then(|x| x.as_str());
            if bid.map(|s| s.starts_with("UC")).unwrap_or(false) || bid.is_none() {
                if !names.iter().any(|n| n == t) && t.len() < 80 {
                    names.push(t.to_string());
                }
            }
        }
    }
    (names, ids)
}

fn duration_of(v: &Value) -> Option<u32> {
    let mut found = None;
    walk(v, &mut |key, val| {
        if found.is_some() {
            return;
        }
        if key == "fixedColumns" {
            // Album/playlist rows keep the length in an array of fixed columns:
            // fixedColumns[i].musicResponsiveListItemFixedColumnRenderer.text.
            // yt_text() cannot read the array itself, so look inside each column.
            if let Some(cols) = val.as_array() {
                for col in cols {
                    let text = &col["musicResponsiveListItemFixedColumnRenderer"]["text"];
                    if let Some(s) = yt_text(text).and_then(|t| parse_mmss(&t)) {
                        found = Some(s);
                        return;
                    }
                }
            }
        } else if key == "lengthText" || key == "duration" {
            if let Some(s) = yt_text(val).and_then(|t| parse_mmss(&t)) {
                found = Some(s);
            }
        }
    });
    found
}

fn parse_mmss(s: &str) -> Option<u32> {
    let s = s.trim();
    // find a token like 3:50 or 1:02:13
    for tok in s.split_whitespace() {
        let parts: Vec<&str> = tok.split(':').collect();
        if parts.len() == 2 || parts.len() == 3 {
            if parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty()) {
                let nums: Vec<u32> = parts.iter().filter_map(|p| p.parse().ok()).collect();
                if nums.len() == 2 {
                    return Some(nums[0] * 60 + nums[1]);
                }
                if nums.len() == 3 {
                    return Some(nums[0] * 3600 + nums[1] * 60 + nums[2]);
                }
            }
        }
    }
    None
}

fn color_of(v: &Value) -> Option<String> {
    let mut n = None;
    walk(v, &mut |key, val| {
        if n.is_none()
            && matches!(
                key,
                "leftStripeColor" | "color" | "backgroundColor" | "solid"
            )
        {
            if let Some(x) = val.as_u64() {
                n = Some(x);
            } else if let Some(x) = val.get("leftStripeColor").and_then(|c| c.as_u64()) {
                n = Some(x);
            } else if let Some(x) = val.get("color").and_then(|c| c.as_u64()) {
                n = Some(x);
            }
        }
    });
    n.map(|c| {
        let rgb = (c as u32) & 0x00FF_FFFF;
        format!("#{rgb:06X}")
    })
}


fn response_has_account(v: &Value) -> bool {
    let mut ok = false;
    walk(v, &mut |key, val| {
        if matches!(
            key,
            "activeAccountHeaderRenderer"
                | "accountButtonRenderer"
                | "multiPageMenuRenderer"
                | "accountName"
        ) {
            ok = true;
        }
        if key == "logged_in" && val.as_bool() == Some(true) {
            ok = true;
        }
    });
    ok
}

fn find_page_continuation(v: &Value) -> Option<String> {
    // Prefer playlist/library shelves and appendContinuationItemsAction -
    // a deep walk of every `contents` array also hits unrelated UI (chips,
    // related, voting) and used to feed reload / non-browse tokens into
    // browse → HTTP 400 INVALID_ARGUMENT.
    //
    // Do NOT use musicShelfRenderer / musicCarouselShelfRenderer here: those
    // hold per-shelf "show more" tokens. On Home that would steal the page
    // continuation and break feed paging (or 400 on the wrong token).
    let mut from_playlist = None;
    walk(v, &mut |key, val| {
        if matches!(
            key,
            "musicPlaylistShelfRenderer"
                | "musicPlaylistShelfContinuation"
                | "musicShelfContinuation"
        ) {
            if let Some(arr) = val["contents"]
                .as_array()
                .or_else(|| val["items"].as_array())
            {
                if let Some(t) = continuation_token_from_list(arr) {
                    from_playlist = Some(t);
                }
            }
            if from_playlist.is_none() {
                from_playlist = next_continuation_data(&val["continuations"]);
            }
        }
        if key == "appendContinuationItemsAction" {
            if let Some(arr) = val["continuationItems"].as_array() {
                if let Some(t) = continuation_token_from_list(arr) {
                    from_playlist = Some(t);
                }
            }
        }
    });
    if from_playlist.is_some() {
        return from_playlist;
    }

    // Home / explore: trailing continuationItemRenderer as a direct child of
    // sectionList / continuationItems (not nested inside a shelf).
    let mut from_list = None;
    walk(v, &mut |key, val| {
        if key == "continuationItems" || key == "sectionListRenderer" {
            let arr = if key == "sectionListRenderer" {
                val["contents"].as_array()
            } else {
                val.as_array()
            };
            if let Some(arr) = arr {
                if let Some(t) = continuation_token_from_list(arr) {
                    from_list = Some(t);
                }
            }
            if from_list.is_none() && key == "sectionListRenderer" {
                from_list = next_continuation_data(&val["continuations"]);
            }
        }
    });
    if from_list.is_some() {
        return from_list;
    }

    // Library song lists (FEmusic_liked_videos, etc.): token lives inside the
    // sole musicShelfRenderer. Only reached when there is no page-level token,
    // so Home's per-shelf "show more" tokens are not stolen as feed paging.
    let mut from_shelf = None;
    walk(v, &mut |key, val| {
        if key == "musicShelfRenderer" {
            if let Some(arr) = val["contents"]
                .as_array()
                .or_else(|| val["items"].as_array())
            {
                if let Some(t) = continuation_token_from_list(arr) {
                    from_shelf = Some(t);
                }
            }
            if from_shelf.is_none() {
                from_shelf = next_continuation_data(&val["continuations"]);
            }
        }
    });
    from_shelf.or_else(|| next_continuation_data(v))
}

/// Token from the last `continuationItemRenderer` in a shelf/list (ytmusicapi 2025 shape).
fn continuation_token_from_list(arr: &[Value]) -> Option<String> {
    for c in arr.iter().rev() {
        if let Some(r) = c.get("continuationItemRenderer") {
            if let Some(t) = token_from_continuation_item(r) {
                return Some(t);
            }
        }
    }
    None
}

fn token_from_continuation_item(r: &Value) -> Option<String> {
    if let Some(t) = r
        .pointer("/continuationEndpoint/continuationCommand/token")
        .and_then(|x| x.as_str())
    {
        if t.len() > 20 {
            return Some(t.to_string());
        }
    }
    // Nested under commandExecutorCommand (alongside voting refresh, etc.).
    // Only CONTINUATION_REQUEST_TYPE_BROWSE is valid for /browse - other
    // command types (watch next, reload) return 400 INVALID_ARGUMENT.
    if let Some(cmds) = r
        .pointer("/continuationEndpoint/commandExecutorCommand/commands")
        .and_then(|x| x.as_array())
    {
        for cmd in cmds {
            let req = cmd
                .pointer("/continuationCommand/request")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if req != "CONTINUATION_REQUEST_TYPE_BROWSE" {
                continue;
            }
            if let Some(t) = cmd
                .pointer("/continuationCommand/token")
                .and_then(|x| x.as_str())
            {
                if t.len() > 20 {
                    return Some(t.to_string());
                }
            }
        }
    }
    None
}

fn parse_continuation_items(v: &Value) -> (Vec<ShelfItem>, Option<String>) {
    let mut items = Vec::new();
    let mut list_tok = None;
    // Prefer the dedicated continuation payload when present.
    if let Some(arr) = v
        .pointer("/onResponseReceivedActions/0/appendContinuationItemsAction/continuationItems")
        .and_then(|x| x.as_array())
    {
        list_tok = continuation_token_from_list(arr);
        for c in arr {
            if let Some(it) = parse_item(c) {
                items.push(it);
            }
        }
    }
    if items.is_empty() {
        walk(v, &mut |key, val| {
            if key == "continuationItems" || key == "contents" || key == "items" {
                if let Some(arr) = val.as_array() {
                    if list_tok.is_none() {
                        if let Some(t) = continuation_token_from_list(arr) {
                            list_tok = Some(t);
                        }
                    }
                    for c in arr {
                        if let Some(it) = parse_item(c) {
                            items.push(it);
                        }
                    }
                }
            }
        });
    }
    if items.is_empty() {
        let mut shelves = Vec::new();
        collect_shelves(v, &mut shelves);
        for s in shelves {
            items.extend(s.items);
        }
    }
    let mut seen = std::collections::HashSet::new();
    items.retain(|it| {
        if it.id.is_empty() {
            return true;
        }
        seen.insert(it.id.clone())
    });
    (
        items,
        list_tok
            .or_else(|| find_page_continuation(v))
            .or_else(|| find_continuation(v)),
    )
}

/// Legacy `continuations[].nextContinuationData` only - never reload tokens.
fn next_continuation_data(v: &Value) -> Option<String> {
    let mut tok = None;
    walk(v, &mut |key, val| {
        if tok.is_none() && key == "nextContinuationData" {
            if let Some(s) = val["continuation"].as_str() {
                if s.len() > 20 {
                    tok = Some(s.to_string());
                }
            }
        }
    });
    tok
}

fn find_continuation(v: &Value) -> Option<String> {
    // Shelf "show more": last continuationItemRenderer, else nextContinuationData.
    // Do NOT grab the first bare `"continuation"` string - that often belongs to
    // reloadContinuationData and /browse rejects it with 400 INVALID_ARGUMENT.
    if let Some(arr) = v["contents"]
        .as_array()
        .or_else(|| v["items"].as_array())
    {
        if let Some(t) = continuation_token_from_list(arr) {
            return Some(t);
        }
    }
    next_continuation_data(v).or_else(|| {
        let mut tok = None;
        walk(v, &mut |key, val| {
            if tok.is_none() && key == "continuationCommand" {
                let req = val["request"].as_str().unwrap_or("CONTINUATION_REQUEST_TYPE_BROWSE");
                if req == "CONTINUATION_REQUEST_TYPE_BROWSE" {
                    if let Some(s) = val["token"].as_str() {
                        if s.len() > 20 {
                            tok = Some(s.to_string());
                        }
                    }
                }
            }
        });
        tok
    })
}

// Silence unused import warnings if PlaylistCard etc. unused
#[allow(dead_code)]
fn _unused(_: (Track, PlaylistCard, ArtistCard, Explore)) {}

fn looks_like_duration(s: &str) -> bool {
    let mut parts = s.split(':');
    let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !a.is_empty()
        && a.chars().all(|c| c.is_ascii_digit())
        && b.len() == 2
        && b.chars().all(|c| c.is_ascii_digit())
}

/// Video id → (album name, album browse id) from a playlist/album browse
/// response. Playlist rows put the album in the third flex column.
pub fn albums_from_browse(v: &Value) -> std::collections::HashMap<String, (String, Option<String>)> {
    let mut out = std::collections::HashMap::new();
    walk(v, &mut |key, val| {
        if key == "musicResponsiveListItemRenderer" {
            if let Some(item) = parse_responsive(val) {
                if !item.id.is_empty() {
                    if let Some(album) = item
                        .album
                        .filter(|a| !a.is_empty() && !looks_like_duration(a))
                    {
                        let aid = item
                            .browse_id
                            .filter(|b| b.starts_with("MPREb") || b.starts_with("OLAK"));
                        out.entry(item.id).or_insert((album, aid));
                    }
                }
            }
        }
    });
    out
}

impl InnerTube {
    /// Browse a library / likes page and collect tracks, playlists, albums, artists.
    /// Follows continuation tokens so Liked Music is not capped at the first page (~100).
    pub async fn library_browse(&self, browse_id: &str) -> Result<(Vec<Track>, Vec<PlaylistCard>, Vec<ArtistCard>)> {
        let v = self.browse(browse_id, None).await?;
        let header_count = library_header_track_count(&v);
        let (mut tracks, mut cards, mut artists) = parse_library_page(&v);
        let mut tok = find_page_continuation(&v);
        let mut prev_tok: Option<String> = None;
        for n in 0..80 {
            let Some(token) = tok.take() else {
                break;
            };
            if prev_tok.as_deref() == Some(token.as_str()) {
                crate::diag::log(format!(
                    "library_browse {browse_id}: same continuation token twice; stopping at {}",
                    tracks.len()
                ));
                break;
            }
            prev_tok = Some(token.clone());
            // A single transient failure (rate limit, dropped connection)
            // used to end the paging here and leave only the first 100 tracks.
            // Do not retry hard 400 INVALID_ARGUMENT - that is a bad token.
            let mut page = self.browse_continuation(&token).await;
            for attempt in 1..=3u64 {
                if page.is_ok() {
                    break;
                }
                let msg = page.as_ref().err().map(|e| e.to_string()).unwrap_or_default();
                if msg.contains("HTTP 400") || msg.contains("INVALID_ARGUMENT") {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(400 * attempt)).await;
                page = self.browse_continuation(&token).await;
            }
            match page {
                Ok(more) => {
                    let before = tracks.len() + cards.len() + artists.len();
                    let (tr, pl, ar) = parse_library_page(&more);
                    tracks.extend(tr);
                    cards.extend(pl);
                    artists.extend(ar);
                    let (items, next) = parse_continuation_items(&more);
                    for i in items {
                        match i.kind.as_str() {
                            "track" | "video" => {
                                if let Some(t) = i.as_track() {
                                    tracks.push(t);
                                }
                            }
                            "playlist" | "album" => {
                                cards.push(crate::models::PlaylistCard {
                                    id: i.id.clone(),
                                    name: i.title.clone(),
                                    cover: i.cover.clone().filter(|u| {
                                        !u.contains("music.youtube.com/image/")
                                            && !u.contains("/mixart")
                                    }),
                                    subtitle: i.subtitle.clone(),
                                    browse_id: i.browse_id.clone(),
                                    year: i.year,
                                    album_type: i.album_type.clone(),
                                    track_count: i.track_count,
                                    author: i.artists.first().cloned(),
                                    from_ytm: Some(true),
                                    color: i.color.clone(),
                                    params: i.params.clone(),
                                    ..Default::default()
                                });
                            }
                            "artist" => {
                                artists.push(crate::models::ArtistCard {
                                    id: i.id.clone(),
                                    name: i.title.clone(),
                                    cover: i.cover.clone(),
                                    subscribers: None,
                                });
                            }
                            _ => {}
                        }
                    }
                    tracks = dedup_tracks(tracks);
                    cards = dedup_playlists(cards);
                    artists = dedup_artists(artists);
                    let after = tracks.len() + cards.len() + artists.len();
                    crate::diag::log(format!(
                        "library_browse {browse_id} continuation {}: +{} items (tracks={}, token {} chars)",
                        n + 1,
                        after.saturating_sub(before),
                        tracks.len(),
                        token.len()
                    ));
                    // ytmusicapi stops on an empty continuation page - chasing
                    // another token from a ~1KB empty body is what produced 400s.
                    if after == before {
                        break;
                    }
                    tok = next.or_else(|| find_page_continuation(&more));
                }
                Err(e) => {
                    crate::diag::log(format!(
                        "library_browse {browse_id} continuation {}: {e} (token {} chars)",
                        n + 1,
                        token.len()
                    ));
                    break;
                }
            }
        }
        if let Some(claimed) = header_count {
            if tracks.len() < claimed {
                crate::diag::log(format!(
                    "library_browse {browse_id}: got {} tracks but header says {claimed}",
                    tracks.len()
                ));
            }
        }
        crate::diag::debug(format!(
            "library_browse {browse_id}: tracks={} cards={} artists={} body≈{}b",
            tracks.len(),
            cards.len(),
            artists.len(),
            v.to_string().len()
        ));
        if tracks.is_empty() && cards.is_empty() && artists.is_empty() {
            crate::diag::soft_fail(
                "library",
                format!("{browse_id}: parsed empty (HTTP ok) - check renderer shapes"),
            );
        }
        Ok((tracks, cards, artists))
    }
}

/// Digits from "1,657 songs" / "1.657 de melodii" in playlist/library headers.
fn library_header_track_count(v: &Value) -> Option<usize> {
    let mut best = None;
    walk(v, &mut |key, val| {
        if key != "secondSubtitle" && key != "subtitle" && key != "description" {
            return;
        }
        let text = yt_text(val).unwrap_or_default();
        if text.is_empty() {
            return;
        }
        let low = text.to_ascii_lowercase();
        if !(low.contains("song")
            || low.contains("track")
            || low.contains("melod")
            || low.contains("șanson")
            || low.contains("chanson")
            || low.contains("lied"))
        {
            return;
        }
        let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<usize>() {
            if n > 0 {
                best = Some(n);
            }
        }
    });
    best
}

pub fn parse_library_page(v: &Value) -> (Vec<Track>, Vec<PlaylistCard>, Vec<ArtistCard>) {
    let mut shelves = Vec::new();
    collect_shelves(v, &mut shelves);
    // Also walk responsive list items that are not in a titled shelf.
    let mut extra_tracks = Vec::new();
    let mut extra_pls = Vec::new();
    walk(v, &mut |key, val| {
        if key == "musicResponsiveListItemRenderer" {
            if let Some(item) = parse_responsive(val) {
                if item.kind == "track" || item.kind == "video" {
                    if let Some(t) = item.as_track() {
                        extra_tracks.push(t);
                    }
                } else if item.kind == "playlist" || item.kind == "album" {
                    extra_pls.push(crate::models::PlaylistCard {
                        id: item.id.clone(),
                        name: item.title.clone(),
                        cover: item.cover.clone().filter(|u| {
                            !u.contains("music.youtube.com/image/") && !u.contains("/mixart")
                        }),
                        subtitle: item.subtitle.clone(),
                        browse_id: item.browse_id.clone(),
                        year: item.year,
                        album_type: item.album_type.clone(),
                        track_count: item.track_count,
                        author: item.artists.first().cloned(),
                        from_ytm: Some(true),
                        color: item.color.clone(),
                        params: item.params.clone(),
                        ..Default::default()
                    });
                }
            }
        }
    });
    let mut tracks = Vec::new();
    let mut playlists = Vec::new();
    let mut artists = Vec::new();
    for s in &shelves {
        for i in &s.items {
            match i.kind.as_str() {
                "track" | "video" => {
                    if let Some(t) = i.as_track() {
                        tracks.push(t);
                    }
                }
                "playlist" | "album" => {
                    playlists.push(crate::models::PlaylistCard {
                        id: i.id.clone(),
                        name: i.title.clone(),
                        cover: i.cover.clone().filter(|u| {
                            !u.contains("music.youtube.com/image/") && !u.contains("/mixart")
                        }),
                        subtitle: i.subtitle.clone(),
                        browse_id: i.browse_id.clone(),
                        year: i.year,
                        album_type: i.album_type.clone(),
                        track_count: i.track_count,
                        author: i.artists.first().cloned(),
                        from_ytm: Some(true),
                        color: i.color.clone(),
                        params: i.params.clone(),
                        ..Default::default()
                    });
                }
                "artist" => {
                    artists.push(crate::models::ArtistCard {
                        id: i.id.clone(),
                        name: i.title.clone(),
                        cover: i.cover.clone(),
                        subscribers: None,
                    });
                }
                _ => {}
            }
        }
    }
    tracks.extend(extra_tracks);
    playlists.extend(extra_pls);
    (dedup_tracks(tracks), dedup_playlists(playlists), dedup_artists(artists))
}

fn dedup_tracks(mut v: Vec<Track>) -> Vec<Track> {
    let mut seen = std::collections::HashSet::new();
    v.retain(|t| !t.id.is_empty() && seen.insert(t.id.clone()));
    v
}
fn dedup_playlists(mut v: Vec<PlaylistCard>) -> Vec<PlaylistCard> {
    let mut seen = std::collections::HashSet::new();
    v.retain(|t| !t.id.is_empty() && seen.insert(t.id.clone()));
    v
}
fn dedup_artists(mut v: Vec<ArtistCard>) -> Vec<ArtistCard> {
    let mut seen = std::collections::HashSet::new();
    v.retain(|t| !t.id.is_empty() && seen.insert(t.id.clone()));
    v
}

#[cfg(test)]
mod continuation_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counterpart_with_segment_map() {
        let v = json!({"contents": {"x": [{
            "playlistPanelVideoWrapperRenderer": {
                "primaryRenderer": {"playlistPanelVideoRenderer": {"videoId": "AAAAAAAAAAA"}},
                "counterpart": [{
                    "counterpartRenderer": {"playlistPanelVideoRenderer": {"videoId": "BBBBBBBBBBB"}},
                    "segmentMap": {"segment": [{
                        "primaryVideoStartTimeMilliseconds": "0",
                        "counterpartVideoStartTimeMilliseconds": "4200",
                        "durationMilliseconds": "180000"
                    }]}
                }]
            }
        }]}});
        let (id, segs) = parse_counterpart(&v, "AAAAAAAAAAA").unwrap();
        assert_eq!(id, "BBBBBBBBBBB");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].counterpart_ms, 4200.0);
        assert!(parse_counterpart(&v, "CCCCCCCCCCC").is_none());
    }

    #[test]
    fn ignores_reload_continuation_data() {
        let v = json!({
            "continuations": [{
                "reloadContinuationData": {
                    "continuation": "RELOAD_TOKEN_THAT_IS_LONG_ENOUGH_TO_PASS_LEN_CHECK_XXXX"
                }
            }]
        });
        assert!(find_continuation(&v).is_none());
        assert!(find_page_continuation(&v).is_none());
    }

    #[test]
    fn prefers_next_continuation_data() {
        let v = json!({
            "continuations": [
                {
                    "reloadContinuationData": {
                        "continuation": "RELOAD_TOKEN_THAT_IS_LONG_ENOUGH_TO_PASS_LEN_CHECK_XXXX"
                    }
                },
                {
                    "nextContinuationData": {
                        "continuation": "NEXT_TOKEN_THAT_IS_LONG_ENOUGH_TO_PASS_LEN_CHECK_YYYYY"
                    }
                }
            ]
        });
        assert_eq!(
            find_continuation(&v).as_deref(),
            Some("NEXT_TOKEN_THAT_IS_LONG_ENOUGH_TO_PASS_LEN_CHECK_YYYYY")
        );
    }

    #[test]
    fn command_executor_requires_browse_type() {
        let r = json!({
            "continuationEndpoint": {
                "commandExecutorCommand": {
                    "commands": [{
                        "continuationCommand": {
                            "request": "CONTINUATION_REQUEST_TYPE_WATCH_NEXT",
                            "token": "WATCH_NEXT_TOKEN_LONG_ENOUGH_TO_PASS_LEN_CHECK_ZZZZ"
                        }
                    }]
                }
            }
        });
        assert!(token_from_continuation_item(&r).is_none());

        let r = json!({
            "continuationEndpoint": {
                "commandExecutorCommand": {
                    "commands": [{
                        "continuationCommand": {
                            "request": "CONTINUATION_REQUEST_TYPE_BROWSE",
                            "token": "BROWSE_TOKEN_LONG_ENOUGH_TO_PASS_LEN_CHECK_AAAAAA"
                        }
                    }]
                }
            }
        });
        assert_eq!(
            token_from_continuation_item(&r).as_deref(),
            Some("BROWSE_TOKEN_LONG_ENOUGH_TO_PASS_LEN_CHECK_AAAAAA")
        );
    }

    #[test]
    fn playlist_shelf_token_beats_nested_noise() {
        let v = json!({
            "contents": {
                "singleColumnBrowseResultsRenderer": {
                    "tabs": [{
                        "tabRenderer": {
                            "content": {
                                "sectionListRenderer": {
                                    "contents": [{
                                        "musicPlaylistShelfRenderer": {
                                            "contents": [
                                                { "musicResponsiveListItemRenderer": { "playlistItemData": { "videoId": "abcdefghijk" } } },
                                                {
                                                    "continuationItemRenderer": {
                                                        "continuationEndpoint": {
                                                            "continuationCommand": {
                                                                "token": "PLAYLIST_PAGE_TOKEN_LONG_ENOUGH_TO_PASS_LEN_CHECK"
                                                            }
                                                        }
                                                    }
                                                }
                                            ]
                                        }
                                    }]
                                }
                            }
                        }
                    }]
                }
            }
        });
        assert_eq!(
            find_page_continuation(&v).as_deref(),
            Some("PLAYLIST_PAGE_TOKEN_LONG_ENOUGH_TO_PASS_LEN_CHECK")
        );
    }
}
