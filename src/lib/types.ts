// Types mirroring the Rust DTOs (serde camelCase). Keep in sync with src-tauri/src.

export type Source = "local" | "spotify" | "soundcloud";
export type Availability = "available" | "preview_only" | "unavailable";

export interface Track {
  id: string;
  source: Source;
  providerId: string;
  providerUri: string | null;
  title: string;
  artist: string;
  album: string;
  albumArtist: string;
  trackNumber: number | null;
  discNumber: number | null;
  year: number | null;
  genre: string;
  durationMs: number | null;
  artwork: string | null;
  filePath: string | null;
  fileFormat: string | null;
  availability: Availability;
  unavailableReason: string | null;
  permalinkUrl: string | null;
  uploader: string | null;
  uploaderUrl: string | null;
  inLibrary: boolean;
  hasOverrides: boolean;
  addedAt: string;
}

export interface LyricLine {
  timeMs: number;
  /** When the singing of this line ends (a gap follows), when known. */
  endMs?: number;
  /** Empty = an instrumental gap (only in a non-Genius source's own lines). */
  text: string;
  /** A Genius section header such as "[Chorus]". */
  header?: boolean;
}

export type LyricsSource = "genius" | "lrclib" | "musixmatch" | "netease";

/** Lyrics for a track: Genius's words, timed by LRCLIB / Musixmatch / NetEase when possible. */
export interface Lyrics {
  /** Timed lines; Genius's words whenever Genius has the song. */
  synced: LyricLine[] | null;
  /** Where the timing came from. */
  syncSource: LyricsSource | null;
  plain: string | null;
  /** Whose words are shown ("genius" unless Genius has none). */
  plainSource: LyricsSource | null;
  geniusUrl: string | null;
  instrumental: boolean;
}

export interface PlaybackCapabilities {
  play: boolean;
  pause: boolean;
  next: boolean;
  previous: boolean;
  seek: boolean;
  volume: boolean;
  queue: boolean;
  deviceSelection: boolean;
  playbackPosition: boolean;
  duration: boolean;
}

export interface ProviderStatus {
  provider: Source | "lastfm";
  displayName: string;
  configured: boolean;
  connected: boolean;
  /** Connected, but the sign-in lacks permissions the app now needs. */
  needsReconnect: boolean;
  /** Permissions the stored sign-in lacks. */
  missingScopes: string[];
  accountName: string | null;
  capabilities: PlaybackCapabilities;
  limitations: string[];
}

export type SortKey = "title" | "artist" | "album" | "duration" | "added" | "year" | "source";

export interface TrackQuery {
  search?: string;
  source?: Source;
  artist?: string;
  album?: string;
  sort?: SortKey;
  descending?: boolean;
  includeUnavailable?: boolean;
  limit?: number;
  offset?: number;
}

export interface ArtistSummary {
  name: string;
  trackCount: number;
  sources: Source[];
}

export interface AlbumSummary {
  name: string;
  artist: string;
  trackCount: number;
  year: number | null;
  artwork: string | null;
  sources: Source[];
}

export interface Playlist {
  id: string;
  name: string;
  description: string;
  origin: string | null;
  trackCount: number;
  artworkPath: string | null;
  lastPlayedAt: string | null;
  createdAt: string;
  updatedAt: string;
  /** Present only for Spotify Synced playlists. */
  sync: PlaylistSync | null;
  /** For a Branflakes-only copy of a synced playlist: the source playlist. */
  forkedFrom: string | null;
  forkedFromName: string | null;
  /** For a copy: what it is based on and which Spotify entries were not copied. */
  forkNote: string | null;
}

export interface UncopyableEntry {
  kind: "spotify_local_file" | "episode" | "unavailable";
  name: string | null;
}

export interface PlaylistSync {
  spotifyPlaylistId: string;
  status: "pending" | "syncing" | "ok" | "error";
  lastSyncedAt: string | null;
  lastError: string | null;
  lastNotice: string | null;
  canEditDetails: boolean;
}

export type AddOutcome =
  | { kind: "Added"; added: number; skippedDuplicates: number }
  | {
      kind: "Forked";
      playlist: Playlist;
      added: number;
      notOnSpotify: number;
      /** Spotify entries that are not in the copy. */
      uncopyable: UncopyableEntry[];
      /** False: Spotify couldn't be checked, so the copy is based on the last sync. */
      fromLatest: boolean;
      note: string;
    };

export interface SyncReport {
  playlistId: string;
  pulledAdded: number;
  pulledRemoved: number;
  pushedOps: number;
  renamed: boolean;
  artworkChanged: boolean;
  notices: string[];
}

export interface SyncAllReport {
  synced: number;
  failed: string[];
}

export interface SyncEvent {
  playlistId: string;
  /** "removed": deleted/removed on Spotify, Branflakes copy removed. "deleted": deleted from Branflakes (and Spotify). */
  status: "syncing" | "ok" | "error" | "removed" | "deleted";
  changed: boolean;
  message: string | null;
}

export interface SpotifyPlaylistOption {
  id: string;
  name: string;
  total: number | null;
  owned: boolean;
  collaborative: boolean;
  canSync: boolean;
  reason: string | null;
  syncedPlaylistId: string | null;
}

export type RepeatMode = "off" | "all" | "one";

export interface PlaybackState {
  currentIndex: number | null;
  positionMs: number;
  volume: number;
  shuffle: boolean;
  repeatMode: RepeatMode;
  spotifyDeviceId: string | null;
}

export interface QueueSnapshot {
  items: Track[];
  state: PlaybackState;
}

export interface LibraryFolder {
  id: number;
  path: string;
  addedAt: string;
  lastScannedAt: string | null;
}

export interface ScanReport {
  folder: string;
  filesSeen: number;
  addedOrUpdated: number;
  unchanged: number;
  missing: number;
  unsupported: number;
  errors: string[];
  unsupportedExamples: string[];
}

export interface FileMetadata {
  path: string;
  format: string;
  tagType: string | null;
  writable: boolean;
  writeUnsupportedReason: string | null;
  title: string;
  artist: string;
  album: string;
  albumArtist: string;
  genre: string;
  year: number | null;
  trackNumber: number | null;
  discNumber: number | null;
  durationMs: number | null;
  hasArtwork: boolean;
}

export type ArtworkEdit = { action: "replace"; path: string } | { action: "remove" };

export interface MetadataEdit {
  title?: string;
  artist?: string;
  album?: string;
  albumArtist?: string;
  genre?: string;
  year?: string;
  trackNumber?: string;
  discNumber?: string;
  artwork?: ArtworkEdit;
}

export interface WriteReport {
  trackId: string;
  backupId: number;
  backupPath: string;
  verified: boolean;
  metadata: FileMetadata;
}

export interface MetadataBackup {
  id: number;
  trackId: string;
  filePath: string;
  backupPath: string;
  createdAt: string;
}

export interface TrackOverride {
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  albumArtist?: string | null;
  genre?: string | null;
  year?: number | null;
  trackNumber?: number | null;
  artworkPath?: string | null;
}

export interface AppSettings {
  spotifyClientId: string;
  soundcloudClientId: string;
  soundcloudSecretSet: boolean;
  soundcloudWebMode: boolean;
  lastfmApiKey: string;
  lastfmSecretSet: boolean;
  lastfmEnabled: boolean;
  lastfmScrobbleSpotify: boolean;
  oauthPort: number;
  spotifySyncIntervalMinutes: number;
  /** Scale in-app playback towards a common loudness. */
  normalizeVolume: boolean;
  /** Also use Musixmatch and NetEase for lyrics timing. */
  lyricsExtraSources: boolean;
  spotifyRedirectUri: string;
  soundcloudRedirectUri: string;
  dataDir: string;
}

export interface SettingsPatch {
  spotifyClientId?: string;
  soundcloudClientId?: string;
  soundcloudClientSecret?: string;
  soundcloudWebMode?: boolean;
  lastfmApiKey?: string;
  lastfmSharedSecret?: string;
  lastfmEnabled?: boolean;
  lastfmScrobbleSpotify?: boolean;
  oauthPort?: number;
  spotifySyncIntervalMinutes?: number;
  normalizeVolume?: boolean;
  lyricsExtraSources?: boolean;
}

export interface ImportReport {
  tracks: number;
  playlists: number;
  skippedPlaylists: string[];
  errors: string[];
  /** Imported playlists that are now Spotify Synced. */
  syncedPlaylists: number;
  /** Imported as ordinary playlists, with the reason sync isn't available. */
  notSynced: { name: string; reason: string }[];
}

export interface CreatePlaylistResult {
  playlist: Playlist;
  /** Spotify sync was requested but couldn't be enabled (the playlist is local-only for now). */
  spotifyError: string | null;
}

export interface RankedAlternative {
  track: Track;
  score: number;
}

export interface AlternativesResult {
  candidates: RankedAlternative[];
  /** The search failed (not the same as "no results"). */
  error: AppErrorPayload | null;
  /** The provider isn't set up. */
  unavailable: string | null;
}

export interface AlternativeRelation {
  originalTrackId: string;
  alternativeTrackId: string;
  removed: boolean;
  createdAt: string;
}

export type AddAlternativeOutcome = { kind: "Added"; position: number } | { kind: "AlreadyAdded"; position: number };

export interface SpotifyDevice {
  id: string | null;
  name: string;
  type: string;
  isActive: boolean;
  isRestricted: boolean;
  supportsVolume: boolean;
  volumePercent: number | null;
}

export interface SpotifyPlaybackState {
  device: SpotifyDevice | null;
  isPlaying: boolean;
  progressMs: number | null;
  itemUri: string | null;
  /** Requested URI when Spotify relinked the track to a market-specific copy. */
  linkedFromUri: string | null;
  itemDurationMs: number | null;
  disallows: string[];
}

export interface StreamInfo {
  url: string;
  protocol: "hls" | "progressive";
  preview: boolean;
  format: string;
}

export interface AppErrorPayload {
  kind: string;
  message: string;
}

export interface ProviderSearchResults {
  spotify: Track[];
  soundcloud: Track[];
  errors: { provider: string; error: AppErrorPayload }[];
}

export interface SubmitOutcome {
  queued: boolean;
  reason: string | null;
}

export interface FlushReport {
  accepted: number;
  ignored: number;
  failed: number;
  remaining: number;
  sessionInvalid: boolean;
  error: string | null;
}

export interface ScrobbleQueueStatus {
  pending: number;
  failed: number;
  lastError: string | null;
}

export interface MediaUpdate {
  title?: string;
  artist?: string;
  album?: string;
  coverUrl?: string;
  durationMs?: number;
  status: "playing" | "paused" | "stopped";
  positionMs?: number;
  /** Resolved controls, mirrored by the Windows taskbar thumbnail toolbar. */
  canPrevious?: boolean;
  canPlayPause?: boolean;
  canNext?: boolean;
}
