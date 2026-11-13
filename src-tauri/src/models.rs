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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShelfItem {
    /// "track" | "album" | "playlist" | "artist" | "mood" | "video"
    pub kind: String,
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub cover: Option<String>,
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
    pub autoplay: bool,
    /// Disk cache cap for prefetched audio, in mebibytes. 0 disables it.
    #[serde(default = "default_cache_mb")]
    pub cache_max_mb: u64,
    #[serde(default = "default_true")]
    pub crossfade: bool,
    #[serde(default = "default_xf_secs")]
    pub crossfade_seconds: f64,
    #[serde(default = "default_true")]
    pub album_continuity: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: "high".into(),
            language: "en".into(),
            region: "US".into(),
            notifications: false,
            autoplay: true,
            cache_max_mb: default_cache_mb(),
            crossfade: true,
            crossfade_seconds: default_xf_secs(),
            album_continuity: true,
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
