export interface Track {
  id: string;
  title: string;
  artists: string[];
  artist_ids: string[];
  album: string | null;
  album_id: string | null;
  duration: number | null;
  cover: string | null;
  is_video: boolean;
}

export interface ArtistCard {
  id: string;
  name: string;
  cover: string | null;
  subscribers: number | null;
}

export interface PlaylistCard {
  id: string;
  name: string;
  cover: string | null;
  track_count: number | null;
  subtitle: string | null;
  year: number | null;
  album_type: string | null;
  author: string | null;
  from_ytm: boolean | null;
  color: string | null;
  params: string | null;
  browse_id: string | null;
}

export interface MoodChip {
  title: string;
  params: string | null;
  browse_id: string | null;
  color: string | null;
  selected: boolean;
}

export interface ShelfItem {
  kind: string;
  id: string;
  title: string;
  subtitle: string | null;
  cover: string | null;
  artists: string[];
  artist_ids: string[];
  album: string | null;
  album_id: string | null;
  duration: number | null;
  playlist_id: string | null;
  color: string | null;
  browse_id: string | null;
  params: string | null;
  year: number | null;
  album_type: string | null;
  track_count: number | null;
  is_video: boolean;
}

export interface Shelf {
  id: string | null;
  title: string;
  subtitle: string | null;
  kind: string;
  items: ShelfItem[];
}

export interface Home {
  chips: MoodChip[];
  shelves: Shelf[];
  continuation: string | null;
  signed_in: boolean;
}

export interface Explore {
  links: MoodChip[];
  shelves: Shelf[];
  moods: ShelfItem[];
}

export interface SearchResults {
  query: string;
  top: ShelfItem | null;
  songs: Track[];
  videos: Track[];
  albums: PlaylistCard[];
  artists: ArtistCard[];
  playlists: PlaylistCard[];
  community: PlaylistCard[];
}

export interface ArtistPage {
  id: string;
  name: string;
  header: string | null;
  cover: string | null;
  description: string | null;
  subscribers: number | null;
  tracks: Track[];
  albums: PlaylistCard[];
  singles: PlaylistCard[];
  videos: Track[];
  playlists: PlaylistCard[];
  similar: ArtistCard[];
  radio_id: string | null;
  tracks_playlist_id: string | null;
  videos_playlist_id: string | null;
  subscribed: boolean;
}

export interface Collection {
  kind: string;
  id: string;
  playlist_id: string | null;
  name: string;
  cover: string | null;
  artists: string[];
  artist_ids: string[];
  author: string | null;
  year: number | null;
  description: string | null;
  track_count: number | null;
  album_type: string | null;
  tracks: Track[];
  related: PlaylistCard[];
  owned: boolean;
  saved: boolean;
}

export interface LyricLine {
  text: string;
  start_ms: number | null;
}

export interface Lyrics {
  source: string | null;
  lines: LyricLine[];
  synced: boolean;
}

export interface Related {
  tracks: Track[];
  versions: Track[];
  albums: PlaylistCard[];
  artists: ArtistCard[];
  playlists: PlaylistCard[];
}

export interface AccountInfo {
  name: string;
  photo: string | null;
  handle: string | null;
}

export interface Library {
  signed_in: boolean;
  mock: boolean;
  account: AccountInfo | null;
  playlists: PlaylistCard[];
  liked: Track[];
  albums: PlaylistCard[];
  artists: ArtistCard[];
  recents: Track[];
}

export interface AuthStatus {
  signed_in: boolean;
  mock: boolean;
  account: AccountInfo | null;
  store: string | null;
}

export interface Settings {
  quality: string;
  language: string;
  region: string;
  notifications: boolean;
  autoplay: boolean;
}

export interface PlayerState {
  track: Track | null;
  playing: boolean;
  loading: boolean;
  position: number;
  duration: number;
  volume: number;
  queue: Track[];
  index: number;
  error: string | null;
  accent: [number, number, number] | null;
  shuffle: boolean;
  repeat: string;
  liked: boolean;
  disliked: boolean;
  autoplay: boolean;
}

export interface GenrePage {
  id: string;
  name: string;
  sections: Shelf[];
}

export interface LikeState {
  liked: boolean;
  disliked: boolean;
}

export type View =
  | { id: 'home' }
  | { id: 'explore' }
  | { id: 'search' }
  | { id: 'library' }
  | { id: 'settings' }
  | { id: 'artist'; artistId: string }
  | { id: 'album'; albumId: string }
  | { id: 'playlist'; playlistId: string }
  | { id: 'genre'; genreId: string; title?: string }
  | { id: 'charts' }
  | { id: 'new-releases' }
  | { id: 'history' };

export interface CtxTarget {
  x: number;
  y: number;
  track?: Track;
  item?: ShelfItem;
  albumId?: string;
  artistId?: string;
  playlistId?: string;
}
