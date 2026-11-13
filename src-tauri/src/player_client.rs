//! Innertube player clients that return unobfuscated googlevideo URLs.
//!
//! VISIONOS (client 101) currently does not require a PO token and returns
//! direct AAC (itag 140) URLs. A *single* closed-range request can download
//! the whole file; a second Range on the same URL is what 403s (~1 MiB)
//! without a GVS PO token. ANDROID is raced as a fallback.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::ytm::{upgrade_cover, user_agent, Stream};

const CONSENT: &str = "SOCS=CAISAiAD";
const MUSIC_HOME: &str = "https://music.youtube.com/";
const PLAYER_URL: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";

const VISIONOS_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
const ANDROID_UA: &str =
    "com.google.android.youtube/21.26.364 (Linux; U; Android 11) gzip";

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    VisionOs,
    Android,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::VisionOs => "visionos",
            Kind::Android => "android",
        }
    }


    fn client_version(self) -> &'static str {
        match self {
            Kind::VisionOs => "1.02",
            Kind::Android => "21.26.364",
        }
    }

    fn client_id(self) -> &'static str {
        match self {
            Kind::VisionOs => "101",
            Kind::Android => "3",
        }
    }

    fn ua(self) -> &'static str {
        match self {
            Kind::VisionOs => VISIONOS_UA,
            Kind::Android => ANDROID_UA,
        }
    }

    fn context(&self, visitor: &str) -> Value {
        match self {
            Kind::VisionOs => json!({
                "clientName": "VISIONOS",
                "clientVersion": "1.02",
                "deviceMake": "Apple",
                "deviceModel": "RealityDevice17,1",
                "userAgent": VISIONOS_UA,
                "osName": "visionOS",
                "osVersion": "26.5.23O471",
                "hl": "en",
                "gl": "US",
                "visitorData": visitor,
            }),
            Kind::Android => json!({
                "clientName": "ANDROID",
                "clientVersion": "21.26.364",
                "androidSdkVersion": 30,
                "userAgent": ANDROID_UA,
                "osName": "Android",
                "osVersion": "11",
                "hl": "en",
                "gl": "US",
                "visitorData": visitor,
            }),
        }
    }
}

pub struct PlayerHub {
    http: reqwest::Client,
    visitor: Mutex<Option<(String, Instant)>>,
}

impl PlayerHub {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(8))
            .user_agent(user_agent())
            .build()?;
        Ok(Self {
            http,
            visitor: Mutex::new(None),
        })
    }

    pub async fn prewarm(&self) {
        match self.visitor_data().await {
            Ok(_) => crate::diag::log("player: visitor data prewarmed"),
            Err(e) => crate::diag::log(format!("player: visitor prewarm: {e}")),
        }
    }

    /// Race VISIONOS and ANDROID; first playable AAC URL wins.
    pub async fn race(&self, video_id: &str) -> Result<Stream> {
        let vis = self.player(video_id, Kind::VisionOs);
        let and = self.player(video_id, Kind::Android);
        tokio::pin!(vis, and);
        let mut vis_done = false;
        let mut and_done = false;
        let mut last = None;
        loop {
            tokio::select! {
                r = &mut vis, if !vis_done => {
                    vis_done = true;
                    match r {
                        Ok(s) => return Ok(s),
                        Err(e) => {
                            crate::diag::log(format!("player visionos: {e}"));
                            last = Some(e);
                        }
                    }
                }
                r = &mut and, if !and_done => {
                    and_done = true;
                    match r {
                        Ok(s) => return Ok(s),
                        Err(e) => {
                            crate::diag::log(format!("player android: {e}"));
                            last = Some(e);
                        }
                    }
                }
                else => break,
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("no innertube player client succeeded")))
    }

    pub async fn player(&self, video_id: &str, kind: Kind) -> Result<Stream> {
        let mut visitor = self.visitor_data().await?;
        match self.player_once(video_id, kind, &visitor).await {
            Err(e) if e.to_string().contains("LOGIN_REQUIRED") => {
                // Visitor id went stale; fetch a fresh one and retry once.
                *self.visitor.lock().unwrap() = None;
                visitor = self.visitor_data().await?;
                self.player_once(video_id, kind, &visitor).await
            }
            other => other,
        }
    }

    async fn player_once(&self, video_id: &str, kind: Kind, visitor: &str) -> Result<Stream> {
        let body = json!({
            "context": { "client": kind.context(visitor) },
            "videoId": video_id,
            "contentCheckOk": true,
            "racyCheckOk": true,
        });
        let resp = self
            .http
            .post(PLAYER_URL)
            .header("User-Agent", kind.ua())
            .header("Content-Type", "application/json")
            .header("X-YouTube-Client-Name", kind.client_id())
            .header("X-YouTube-Client-Version", kind.client_version())
            .header("Origin", "https://www.youtube.com")
            .header("X-Origin", "https://www.youtube.com")
            .header("X-Goog-Visitor-Id", visitor)
            .header("Cookie", CONSENT)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(anyhow!(
                "player {}: HTTP {status}",
                kind.name()
            ));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| anyhow!("player json: {e}"))?;
        let play = v
            .pointer("/playabilityStatus/status")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if play != "OK" {
            let reason = v
                .pointer("/playabilityStatus/reason")
                .and_then(|x| x.as_str())
                .unwrap_or(play);
            return Err(anyhow!("player {}: {play} ({reason})", kind.name()));
        }
        let stream = pick_aac(&v, kind).ok_or_else(|| {
            anyhow!(
                "player {}: no AAC url (sabr-only or empty)",
                kind.name()
            )
        })?;
        crate::diag::log(format!(
            "player {}: {} itag={:?} clen={:?}",
            kind.name(),
            video_id,
            stream.itag(),
            url_query(&stream.url, "clen")
        ));
        Ok(stream)
    }

    async fn visitor_data(&self) -> Result<String> {
        {
            let g = self.visitor.lock().unwrap();
            if let Some((v, at)) = g.as_ref() {
                if at.elapsed() < Duration::from_hours(6) && !v.is_empty() {
                    return Ok(v.clone());
                }
            }
        }
        let resp = self
            .http
            .get(MUSIC_HOME)
            .header("User-Agent", user_agent())
            .header("Cookie", CONSENT)
            .header("Origin", "https://music.youtube.com")
            .header("Referer", "https://music.youtube.com/")
            .send()
            .await?;
        let mut found = None;
        for c in resp.headers().get_all(reqwest::header::SET_COOKIE) {
            let Ok(s) = c.to_str() else { continue };
            if let Some(rest) = s.strip_prefix("__Secure-YEC=") {
                let v = rest.split(';').next().unwrap_or("").trim();
                if !v.is_empty() {
                    found = Some(v.to_string());
                    break;
                }
            }
        }
        if found.is_none() {
            let html = resp.text().await.unwrap_or_default();
            found = extract_visitor(&html);
        }
        let v = found.ok_or_else(|| anyhow!("could not get visitor data"))?;
        *self.visitor.lock().unwrap() = Some((v.clone(), Instant::now()));
        Ok(v)
    }
}

fn extract_visitor(html: &str) -> Option<String> {
    for key in ["\"visitorData\":\"", "\"VISITOR_DATA\":\""] {
        if let Some(i) = html.find(key) {
            let rest = &html[i + key.len()..];
            if let Some(end) = rest.find('"') {
                let s = rest[..end].replace("\\u003d", "=").replace("\\/", "/");
                if s.len() >= 16 {
                    return Some(s);
                }
            }
        }
    }
    None
}

fn pick_aac(v: &Value, kind: Kind) -> Option<Stream> {
    struct Cand {
        rank_aac: bool,
        br: u64,
        url: String,
        mime: Option<String>,
        duration: Option<f64>,
    }
    let mut best: Option<Cand> = None;
    // (is_aac, bitrate, url, mime, duration)
    for key in ["adaptiveFormats", "formats"] {
        let Some(arr) = v
            .pointer(&format!("/streamingData/{key}"))
            .and_then(|x| x.as_array())
        else {
            continue;
        };
        for f in arr {
            let url = match f.get("url").and_then(|x| x.as_str()) {
                Some(u) if !u.is_empty() => u.to_string(),
                _ => continue,
            };
            let mime = f
                .get("mimeType")
                .or_else(|| f.get("mime"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let itag = f.get("itag").and_then(|x| x.as_u64()).unwrap_or(0);
            // Adaptive audio only — never muxed itag 18 (360p video+aac),
            // which is large and slow and is not a music stream.
            if key != "adaptiveFormats" {
                continue;
            }
            if !mime.contains("audio/") && !matches!(itag, 139..=141) {
                continue;
            }
            // Prefer AAC/m4a — we do not ship an opus decoder.
            let aac = mime.contains("mp4") || mime.contains("aac") || matches!(itag, 139..=141);
            if !aac {
                continue;
            }
            let br = f
                .get("bitrate")
                .or_else(|| f.get("averageBitrate"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            // Prefer itag 140 (128k AAC) over 139 (48k) or muxed 18.
            let rank_aac = itag == 140 || itag == 141;
            let dur = f
                .get("approxDurationMs")
                .and_then(|x| x.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| x.as_f64()))
                .map(|ms| ms / 1000.0);
            let better = match &best {
                None => true,
                Some(c) => (rank_aac, br) > (c.rank_aac, c.br),
            };
            if better {
                best = Some(Cand { rank_aac, br, url, mime: Some(mime), duration: dur });
            }
        }
    }
    let Cand { url, mime, duration, .. } = best?;
    let expire = url_query(&url, "expire").and_then(|s| s.parse().ok());
    Some(Stream {
        url,
        headers: vec![
            ("User-Agent".into(), kind.ua().to_string()),
            ("Accept".into(), "*/*".into()),
            ("Accept-Language".into(), "en-US,en;q=0.9".into()),
        ],
        duration,
        source: "innertube",
        mime,
        client: kind.name(),
        expire,
    })
}

fn url_query(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    for part in q.split('&') {
        if let Some((k, v)) = part.split_once('=') {
            if k == key {
                return Some(v.to_string());
            }
        }
    }
    None
}

// Silence unused import if upgrade_cover is unused — keep for future cover on player.
#[allow(dead_code)]
fn _cover(u: &str) -> String {
    upgrade_cover(u)
}

/// Persistent resolve cache: video_id + client → stream URL, honouring expire=.
pub struct ResolveCache {
    path: std::path::PathBuf,
    map: Mutex<std::collections::HashMap<String, DiskStream>>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct DiskStream {
    video_id: String,
    client: String,
    url: String,
    headers: Vec<(String, String)>,
    duration: Option<f64>,
    source: String,
    mime: Option<String>,
    expire: Option<u64>,
}

impl ResolveCache {
    pub fn open() -> Self {
        let path = dirs::data_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("tarn")
            .join("resolve-cache.json");
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let map = Self::load(&path);
        Self {
            path,
            map: Mutex::new(map),
        }
    }

    fn load(path: &std::path::Path) -> std::collections::HashMap<String, DiskStream> {
        let Ok(bytes) = std::fs::read(path) else {
            return Default::default();
        };
        serde_json::from_slice::<Vec<DiskStream>>(&bytes)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| still_fresh(s.expire))
            .map(|s| (key(&s.video_id, &s.client), s))
            .collect()
    }

    fn save(&self, map: &std::collections::HashMap<String, DiskStream>) {
        let rows: Vec<&DiskStream> = map.values().filter(|s| still_fresh(s.expire)).collect();
        if let Ok(bytes) = serde_json::to_vec(&rows) {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, &bytes).is_ok() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
                }
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }

    pub fn get(&self, video_id: &str, client: Option<&str>) -> Option<Stream> {
        let g = self.map.lock().unwrap();
        let find = |c: &str| {
            g.get(&key(video_id, c))
                .filter(|s| still_fresh(s.expire))
                .map(to_stream)
        };
        if let Some(c) = client {
            return find(c);
        }
        // Prefer a VISIONOS hit, then anything still fresh.
        find("visionos")
            .or_else(|| find("android"))
            .or_else(|| find("ios"))
            .or_else(|| g.values().find(|s| s.video_id == video_id && still_fresh(s.expire)).map(to_stream))
    }

    pub fn put(&self, video_id: &str, stream: &Stream) {
        if video_id.is_empty() || stream.url.is_empty() {
            return;
        }
        let entry = DiskStream {
            video_id: video_id.to_string(),
            client: stream.client.to_string(),
            url: stream.url.clone(),
            headers: stream.headers.clone(),
            duration: stream.duration,
            source: stream.source.to_string(),
            mime: stream.mime.clone(),
            expire: stream.expire.or_else(|| {
                url_query(&stream.url, "expire").and_then(|s| s.parse().ok())
            }),
        };
        if !still_fresh(entry.expire) {
            return;
        }
        let mut g = self.map.lock().unwrap();
        g.insert(key(video_id, stream.client), entry);
        self.save(&g);
    }

    pub fn clear(&self) {
        let mut g = self.map.lock().unwrap();
        g.clear();
        self.save(&g);
        let _ = std::fs::remove_file(&self.path);
    }

    pub fn evict(&self, video_id: &str) {
        let mut g = self.map.lock().unwrap();
        let before = g.len();
        g.retain(|_, s| s.video_id != video_id);
        if g.len() != before {
            self.save(&g);
        }
    }
}

fn key(id: &str, client: &str) -> String {
    format!("{id}\0{client}")
}

fn still_fresh(expire: Option<u64>) -> bool {
    let Some(exp) = expire else {
        return false;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now + 90 < exp
}

fn intern_source(s: &str) -> &'static str {
    match s {
        "innertube" => "innertube",
        "rustypipe" => "rustypipe",
        "yt-dlp" => "yt-dlp",
        _ => "innertube",
    }
}

fn intern_client(s: &str) -> &'static str {
    match s {
        "visionos" => "visionos",
        "android" => "android",
        "ios" => "ios",
        "tv" => "tv",
        "web" => "web",
        "web-music" => "web-music",
        "mobile" => "mobile",
        "yt-dlp" => "yt-dlp",
        _ => "other",
    }
}

fn to_stream(s: &DiskStream) -> Stream {
    Stream {
        url: s.url.clone(),
        headers: s.headers.clone(),
        duration: s.duration,
        source: intern_source(&s.source),
        mime: s.mime.clone(),
        client: intern_client(&s.client),
        expire: s.expire,
    }
}

// Duration::from_hours is stable in 1.80+; rustc 1.100 has it.
// If not, we'd use from_secs. 1.100 is fine.
