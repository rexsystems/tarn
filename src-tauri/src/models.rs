use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Track {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub artist_ids: Vec<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub duration: Option<u32>,
    pub cover: Option<String>,
    #[serde(default)]
    pub is_video: bool,
}

impl Track {
    /// Fill a missing / webview-unusable cover with the public YouTube thumb
    /// for this video id so the player bar still has art after restart.
    pub fn ensure_cover(&mut self) {
        if self.cover.as_deref().is_some_and(cover_usable_in_webview) {
            return;
        }
        if is_youtube_video_id(&self.id) {
            self.cover = Some(format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", self.id));
        }
    }
}

fn cover_usable_in_webview(url: &str) -> bool {
    !url.is_empty()
        && url.starts_with("http")
        && !url.contains("music.youtube.com/image/")
        && !url.contains("/mixart")
}

fn is_youtube_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtistCard {
    pub id: String,
    pub name: String,
    pub cover: Option<String>,
    pub subscribers: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaylistCard {
    pub id: String,
    pub name: String,
    pub cover: Option<String>,
    /// Up to 4 track covers for a 2×2 mosaic when there's no custom art.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
    pub track_count: Option<u64>,
    pub subtitle: Option<String>,
    pub year: Option<u16>,
    pub album_type: Option<String>,
    pub author: Option<String>,
    pub from_ytm: Option<bool>,
    pub color: Option<String>,
    pub params: Option<String>,
    pub browse_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MoodChip {
    pub title: String,
    pub params: Option<String>,
    pub browse_id: Option<String>,
    pub color: Option<String>,
    pub selected: bool,
}

/// A row of content on Home, Explore, or a genre page.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Shelf {
    pub id: Option<String>,
    pub title: String,
    pub subtitle: Option<String>,
    /// "tracks" | "albums" | "playlists" | "artists" | "moods" | "mixed" | "videos"
    pub kind: String,
    pub items: Vec<ShelfItem>,
    /// Per-shelf InnerTube continuation ("show more" / next page of this row).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShelfItem {
    /// "track" | "album" | "playlist" | "artist" | "mood" | "video"
    pub kind: String,
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub cover: Option<String>,
    /// Up to 4 track covers for mix/playlist mosaics when header art is mixart.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub artist_ids: Vec<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub duration: Option<u32>,
    pub playlist_id: Option<String>,
    pub color: Option<String>,
    pub browse_id: Option<String>,
    pub params: Option<String>,
    pub year: Option<u16>,
    pub album_type: Option<String>,
    pub track_count: Option<u64>,
    pub is_video: bool,
}

impl ShelfItem {
    pub fn as_track(&self) -> Option<Track> {
        if self.kind == "track" || self.kind == "video" {
            if self.id.is_empty() {
                return None;
            }
            Some(Track {
                id: self.id.clone(),
                title: self.title.clone(),
                artists: self.artists.clone(),
                artist_ids: self.artist_ids.clone(),
                album: self.album.clone(),
                album_id: self.album_id.clone(),
                duration: self.duration,
                cover: self.cover.clone(),
                is_video: self.is_video || self.kind == "video",
            })
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Home {
    pub chips: Vec<MoodChip>,
    pub shelves: Vec<Shelf>,
    pub continuation: Option<String>,
    pub signed_in: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Explore {
    pub links: Vec<MoodChip>,
    pub shelves: Vec<Shelf>,
    pub moods: Vec<ShelfItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchResults {
    pub query: String,
    pub top: Option<ShelfItem>,
    pub songs: Vec<Track>,
    pub videos: Vec<Track>,
    pub albums: Vec<PlaylistCard>,
    pub artists: Vec<ArtistCard>,
    pub playlists: Vec<PlaylistCard>,
    pub community: Vec<PlaylistCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtistPage {
    pub id: String,
    pub name: String,
    pub header: Option<String>,
    pub cover: Option<String>,
    pub description: Option<String>,
    pub subscribers: Option<u64>,
    pub tracks: Vec<Track>,
    pub albums: Vec<PlaylistCard>,
    pub singles: Vec<PlaylistCard>,
    pub videos: Vec<Track>,
    pub playlists: Vec<PlaylistCard>,
    pub similar: Vec<ArtistCard>,
    pub radio_id: Option<String>,
    pub tracks_playlist_id: Option<String>,
    pub videos_playlist_id: Option<String>,
    pub subscribed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Collection {
    /// "album" | "playlist"
    pub kind: String,
    pub id: String,
    pub playlist_id: Option<String>,
    pub name: String,
    pub cover: Option<String>,
    pub artists: Vec<String>,
    pub artist_ids: Vec<String>,
    pub author: Option<String>,
    pub year: Option<u16>,
    pub description: Option<String>,
    pub track_count: Option<u64>,
    pub album_type: Option<String>,
    pub tracks: Vec<Track>,
    pub related: Vec<PlaylistCard>,
    pub owned: bool,
    pub saved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LyricLine {
    pub text: String,
    pub start_ms: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Lyrics {
    pub source: Option<String>,
    pub lines: Vec<LyricLine>,
    pub synced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Related {
    pub tracks: Vec<Track>,
    pub versions: Vec<Track>,
    pub albums: Vec<PlaylistCard>,
    pub artists: Vec<ArtistCard>,
    pub playlists: Vec<PlaylistCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountInfo {
    pub name: String,
    pub photo: Option<String>,
    pub handle: Option<String>,
    /// Brand-account Gaia / page id for X-Goog-PageId and onBehalfOfUser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Library {
    pub signed_in: bool,
    pub mock: bool,
    pub account: Option<AccountInfo>,
    pub playlists: Vec<PlaylistCard>,
    pub liked: Vec<Track>,
    pub albums: Vec<PlaylistCard>,
    pub artists: Vec<ArtistCard>,
    pub recents: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthStatus {
    pub signed_in: bool,
    pub mock: bool,
    pub account: Option<AccountInfo>,
    pub store: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// "high" | "medium" | "low"
    pub quality: String,
    pub language: String,
    pub region: String,
    /// Desktop track-change notifications. Off by default.
    #[serde(default)]
    pub notifications: bool,
    /// Show what you're listening to on Discord. On by default.
    #[serde(default = "default_true")]
    pub discord_presence: bool,
    /// Set after the one-shot migration that turns the old default (on)
    /// into off. Missing from older settings.json files.
    #[serde(default = "default_true")]
    pub notifications_migrated: bool,
    pub autoplay: bool,
    /// When on, resume the last session and start playing on app launch
    /// (instead of arming paused).
    #[serde(default)]
    pub resume_on_start: bool,
    /// Disk cache cap for prefetched audio, in mebibytes. 0 disables it.
    #[serde(default = "default_cache_mb")]
    pub cache_max_mb: u64,
    #[serde(default = "default_true")]
    pub crossfade: bool,
    #[serde(default = "default_xf_secs")]
    pub crossfade_seconds: f64,
    #[serde(default = "default_true")]
    pub album_continuity: bool,
    /// When on, Next / Prev / picking another track also uses the crossfade
    /// length. When off, those stay a short cut even if crossfade is enabled
    /// for natural end-of-track handoffs.
    #[serde(default = "default_true")]
    pub crossfade_manual: bool,
    /// Five-band graphic EQ. Gains are dB in [-12, 12].
    #[serde(default)]
    pub eq_enabled: bool,
    #[serde(default = "default_eq_bands")]
    pub eq_bands: [f32; 5],
    /// Fade-out shape of the outgoing track: "equal_power" | "linear" |
    /// "logarithmic" | "exponential" | "s_curve".
    #[serde(default = "default_curve")]
    pub crossfade_curve_out: String,
    /// Fade-in shape of the incoming track (same values).
    #[serde(default = "default_curve")]
    pub crossfade_curve_in: String,
    /// Percent (10-100) of the crossfade the outgoing track spends fading.
    #[serde(default = "default_pct")]
    pub crossfade_out_len: f64,
    /// Percent (10-100) of the crossfade the incoming track spends rising.
    #[serde(default = "default_pct")]
    pub crossfade_in_len: f64,
    /// Skip trailing silence of the outgoing and leading silence of the
    /// incoming track.
    #[serde(default)]
    pub crossfade_smart: bool,
    /// "off" | "lowpass" | "highpass" | "sweep"
    #[serde(default = "default_filter")]
    pub crossfade_filter: String,
    /// Detect tempo and match the incoming track to the outgoing one.
    #[serde(default)]
    pub bpm_match: bool,
    /// Largest tempo change BPM matching may apply, in percent.
    #[serde(default = "default_bpm_max")]
    pub bpm_max_change: f64,
    /// Start the overlap on a beat and the incoming track on its first beat.
    #[serde(default = "default_true")]
    pub bpm_beat_align: bool,
    /// Seconds the new track takes to glide back to its own tempo.
    #[serde(default = "default_bpm_return")]
    pub bpm_return_seconds: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: "high".into(),
            language: "en".into(),
            region: "US".into(),
            notifications: false,
            discord_presence: true,
            notifications_migrated: true,
            autoplay: true,
            resume_on_start: false,
            cache_max_mb: default_cache_mb(),
            crossfade: true,
            crossfade_seconds: default_xf_secs(),
            album_continuity: true,
            crossfade_manual: true,
            eq_enabled: false,
            eq_bands: default_eq_bands(),
            crossfade_curve_out: default_curve(),
            crossfade_curve_in: default_curve(),
            crossfade_out_len: default_pct(),
            crossfade_in_len: default_pct(),
            crossfade_smart: false,
            crossfade_filter: default_filter(),
            bpm_match: false,
            bpm_max_change: default_bpm_max(),
            bpm_beat_align: true,
            bpm_return_seconds: default_bpm_return(),
        }
    }
}

impl Settings {
    /// The crossfade configuration the player uses, clamped to sane ranges.
    pub fn transition(&self) -> crate::transition::XfConfig {
        use crate::transition::{Curve, FilterSweep, XfConfig, XfShape};
        XfConfig {
            shape: XfShape {
                secs: self.crossfade_seconds.clamp(0.0, 20.0) as f32,
                curve_out: Curve::parse(&self.crossfade_curve_out),
                curve_in: Curve::parse(&self.crossfade_curve_in),
                out_len: (self.crossfade_out_len.clamp(10.0, 100.0) / 100.0) as f32,
                in_len: (self.crossfade_in_len.clamp(10.0, 100.0) / 100.0) as f32,
                filter: FilterSweep::parse(&self.crossfade_filter),
            },
            smart: self.crossfade_smart,
            bpm_match: self.bpm_match,
            bpm_max_pct: self.bpm_max_change.clamp(0.5, 16.0) as f32,
            beat_align: self.bpm_beat_align,
            bpm_return_secs: self.bpm_return_seconds.clamp(1.0, 60.0) as f32,
        }
    }
}

fn default_cache_mb() -> u64 {
    1024
}
fn default_true() -> bool {
    true
}
fn default_xf_secs() -> f64 {
    4.0
}
fn default_curve() -> String {
    "equal_power".into()
}
fn default_pct() -> f64 {
    100.0
}
fn default_filter() -> String {
    "off".into()
}
fn default_bpm_max() -> f64 {
    6.0
}
fn default_bpm_return() -> f64 {
    8.0
}
fn default_eq_bands() -> [f32; 5] {
    [0.0; 5]
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GenrePage {
    pub id: String,
    pub name: String,
    pub sections: Vec<Shelf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub track: Option<Track>,
    pub playing: bool,
    pub loading: bool,
    #[serde(default)]
    pub buffering: bool,
    /// Seconds of the track already downloaded (from the start).
    #[serde(default)]
    pub buffered: f64,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub queue: Vec<Track>,
    pub index: usize,
    pub error: Option<String>,
    pub accent: Option<[u8; 3]>,
    pub shuffle: bool,
    /// "off" | "all" | "one"
    pub repeat: String,
    pub liked: bool,
    pub disliked: bool,
    pub autoplay: bool,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            track: None,
            playing: false,
            loading: false,
            buffering: false,
            buffered: 0.0,
            position: 0.0,
            duration: 0.0,
            volume: 0.85,
            queue: Vec::new(),
            index: 0,
            error: None,
            accent: None,
            shuffle: false,
            repeat: "off".into(),
            liked: false,
            disliked: false,
            autoplay: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LikeState {
    pub liked: bool,
    pub disliked: bool,
}
