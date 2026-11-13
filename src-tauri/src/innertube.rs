//! Raw InnerTube helpers: public browse (Home, Explore), likes, lyrics, mutations.
//! Cookie values are never logged.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::auth::{sapisid_from_header, sapisidhash_now};
use crate::models::{
    AccountInfo, ArtistCard, Explore, Home, LikeState, LyricLine, Lyrics, MoodChip, PlaylistCard,
    Shelf, ShelfItem, Track,
};
use crate::ytm::{upgrade_cover, user_agent};

const ORIGIN: &str = "https://music.youtube.com";
const CLIENT_NAME: &str = "WEB_REMIX";
const CLIENT_VERSION: &str = "1.20241216.01.00";

#[derive(Clone)]
pub struct InnerTube {
    http: reqwest::Client,
    cookie: Option<String>,
    lang: String,
    region: String,
}

impl InnerTube {
    pub fn new(http: reqwest::Client, cookie: Option<String>, lang: String, region: String) -> Self {
        Self {
            http,
            cookie,
            lang,
            region,
        }
    }

    fn context(&self) -> Value {
        json!({
            "client": {
                "clientName": CLIENT_NAME,
                "clientVersion": CLIENT_VERSION,
                "hl": self.lang,
                "gl": self.region.to_uppercase(),
            }
        })
    }

    pub async fn post(&self, endpoint: &str, mut body: Value) -> Result<Value> {
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
        }
        let resp = req.json(&body).send().await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("innertube {endpoint}: HTTP {status}"));
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

    pub async fn home(&self, params: Option<&str>) -> Result<Home> {
        let v = self.browse("FEmusic_home", params).await?;
        let mut home = parse_home(&v);
        // One continuation page fills "Listen again" / mixed shelves when present.
        if let Some(tok) = home.continuation.clone() {
            if let Ok(more) = self.browse_continuation(&tok).await {
                let extra = parse_home(&more);
                merge_home(&mut home, extra);
            }
        }
        home.signed_in = self.cookie.is_some();
        Ok(home)
    }

    pub async fn explore(&self) -> Result<Explore> {
        let v = self.browse("FEmusic_explore", None).await?;
        Ok(parse_explore(&v))
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
        let v = self.browse(lyrics_browse_id, None).await?;
        Ok(parse_timed_lyrics(&v))
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
                    // Avoid pushing the same shelf twice if walk hits nested copies — by title+len
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
    Some(Shelf {
        id: val["shelfId"].as_str().map(|s| s.to_string()),
        title,
        subtitle,
        kind,
        items,
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

fn parse_two_row(r: &Value) -> Option<ShelfItem> {
    let title = yt_text(&r["title"])?;
    let subtitle = yt_text(&r["subtitle"]);
    let cover = thumb_of(r);
    let video_id = video_id_of(r);
    let (browse_id, params, page) = primary_browse(r);
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
    } else if page.as_deref() == Some("MUSIC_PAGE_TYPE_PLAYLIST") || playlist_id.is_some() {
        (
            "playlist",
            playlist_id
                .clone()
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
        (
            "playlist",
            playlist_id_of(r)
                .or_else(|| {
                    browse_id
                        .as_ref()
                        .map(|b| b.trim_start_matches("VL").to_string())
                })
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

fn parse_account(v: &Value) -> Option<AccountInfo> {
    let mut name = None;
    let mut photo = None;
    let mut handle = None;
    walk(v, &mut |key, val| {
        if key == "accountName" || key == "accountNameButtonRenderer" {
            if name.is_none() {
                name = yt_text(val);
            }
        }
        if key == "accountPhoto" || key == "thumbnails" {
            if photo.is_none() {
                photo = thumb_of(val).or_else(|| {
                    val.as_array()
                        .and_then(|a| a.last())
                        .and_then(|t| t["url"].as_str().map(|s| upgrade_cover(s)))
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
                    if t.len() < 80 && !t.contains("http") {
                        name = Some(t);
                    }
                }
            }
        });
    }
    name.map(|name| AccountInfo {
        name,
        photo,
        handle,
    })
}

fn parse_timed_lyrics(v: &Value) -> Option<Lyrics> {
    let mut lines = Vec::new();
    walk(v, &mut |key, val| {
        if key == "timedLyricsData" {
            if let Some(arr) = val.as_array() {
                for ln in arr {
                    let text = ln["lyricLine"]
                        .as_str()
                        .map(|s| s.to_string())
                        .or_else(|| yt_text(&ln["lyricLine"]))
                        .unwrap_or_default();
                    let start = ln["cueRange"]["startTimeMilliseconds"]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| ln["cueRange"]["startTimeMilliseconds"].as_u64().map(|n| n as u32));
                    if !text.is_empty() {
                        lines.push(LyricLine {
                            text,
                            start_ms: start,
                        });
                    }
                }
            }
        }
        if key == "lyricsData" {
            // unsynced alternative
        }
    });
    if lines.is_empty() {
        return None;
    }
    let synced = lines.iter().any(|l| l.start_ms.is_some());
    Some(Lyrics {
        source: None,
        lines,
        synced,
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
                        if url.starts_with("http") {
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
    best.map(|(_, u)| upgrade_cover(&u))
}

fn video_id_of(v: &Value) -> Option<String> {
    let mut found = None;
    // Prefer playlistItemData / watchEndpoint
    if let Some(s) = v.pointer("/playlistItemData/videoId").and_then(|x| x.as_str()) {
        return Some(s.to_string());
    }
    walk(v, &mut |key, val| {
        if found.is_none() && key == "videoId" {
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
                        // RDAMVM is a radio derived from a video — still useful as playlist_id
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
        if found.is_none() && (key == "lengthText" || key == "duration" || key == "fixedColumns") {
            if let Some(t) = yt_text(val) {
                if let Some(s) = parse_mmss(&t) {
                    found = Some(s);
                }
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

fn find_continuation(v: &Value) -> Option<String> {
    let mut tok = None;
    walk(v, &mut |key, val| {
        if tok.is_none() {
            if key == "continuation" && val.is_string() {
                if let Some(s) = val.as_str() {
                    if s.len() > 20 {
                        tok = Some(s.to_string());
                    }
                }
            }
            if key == "nextContinuationData" {
                if let Some(s) = val["continuation"].as_str() {
                    tok = Some(s.to_string());
                }
            }
            if key == "continuationCommand" {
                if let Some(s) = val["token"].as_str() {
                    tok = Some(s.to_string());
                }
            }
        }
    });
    tok
}

// Silence unused import warnings if PlaylistCard etc. unused
#[allow(dead_code)]
fn _unused(_: (Track, PlaylistCard, ArtistCard, Explore)) {}
