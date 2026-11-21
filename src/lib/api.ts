// Typed wrappers around Tauri commands. UI components call these; they never
// talk to providers directly.
import { invoke } from "@tauri-apps/api/core";
import type * as T from "./types";

export class ApiError extends Error {
  constructor(
    public kind: string,
    message: string,
  ) {
    super(message);
  }
}

export function toApiError(e: unknown): ApiError {
  if (e instanceof ApiError) return e;
  if (e && typeof e === "object" && "kind" in e && "message" in e) {
    const p = e as T.AppErrorPayload;
    return new ApiError(p.kind, p.message);
  }
  return new ApiError("other", e instanceof Error ? e.message : String(e));
}

async function call<R>(cmd: string, args?: Record<string, unknown>): Promise<R> {
  try {
    return await invoke<R>(cmd, args);
  } catch (e) {
    throw toApiError(e);
  }
}

export const api = {
  // library
  listTracks: (query: T.TrackQuery) => call<T.Track[]>("library_list_tracks", { query }),
  getTrack: (id: string) => call<T.Track>("library_get_track", { id }),
  listArtists: () => call<T.ArtistSummary[]>("library_list_artists"),
  listAlbums: (artist?: string) => call<T.AlbumSummary[]>("library_list_albums", { artist: artist ?? null }),
  setMembership: (trackIds: string[], inLibrary: boolean) => call<void>("library_set_membership", { trackIds, inLibrary }),

  // local
  listFolders: () => call<T.LibraryFolder[]>("local_list_folders"),
  addFolder: (path: string) => call<T.LibraryFolder>("local_add_folder", { path }),
  removeFolder: (id: number) => call<void>("local_remove_folder", { id }),
  scan: (folderId?: number) => call<T.ScanReport[]>("local_scan", { folderId: folderId ?? null }),
  readMetadata: (trackId: string) => call<T.FileMetadata>("local_read_metadata", { trackId }),
  writeMetadata: (trackId: string, edit: T.MetadataEdit) => call<T.WriteReport>("local_write_metadata", { trackId, edit }),
  listBackups: (trackId: string) => call<T.MetadataBackup[]>("local_list_backups", { trackId }),
  restoreBackup: (backupId: number) => call<void>("local_restore_backup", { backupId }),

  // overrides
  getOverride: (trackId: string) => call<T.TrackOverride | null>("override_get", { trackId }),
  setOverride: (trackId: string, value: T.TrackOverride) => call<T.Track>("override_set", { trackId, value }),
  setOverrideArtwork: (trackId: string, imagePath: string) => call<T.Track>("override_set_artwork", { trackId, imagePath }),
  clearOverride: (trackId: string) => call<T.Track>("override_clear", { trackId }),

  // playlists
  listPlaylists: () => call<T.Playlist[]>("playlist_list"),
  /** Local-only unless `syncWithSpotify` (then a private Spotify playlist is created and linked). */
  createPlaylist: (name: string, syncWithSpotify = false) => call<T.CreatePlaylistResult>("playlist_create", { name, syncWithSpotify }),
  /** Explicitly create/link the Spotify counterpart of an existing playlist. */
  linkPlaylistToSpotify: (id: string) => call<T.Playlist>("playlist_link_spotify", { id }),
  renamePlaylist: (id: string, name: string) => call<void>("playlist_rename", { id, name }),
  /** For Spotify Synced playlists pass `confirmSpotify` (also remove on Spotify) or `keepOnSpotify`. */
  deletePlaylist: (id: string, opts: { confirmSpotify?: boolean; keepOnSpotify?: boolean } = {}) =>
    call<string | null>("playlist_delete", { id, confirmSpotify: opts.confirmSpotify ?? null, keepOnSpotify: opts.keepOnSpotify ?? null }),
  playlistTracks: (id: string) => call<T.Track[]>("playlist_tracks", { id }),
  markPlaylistPlayed: (id: string) => call<void>("playlist_mark_played", { id }),
  setPlaylistArtwork: (id: string, imagePath: string | null) => call<T.Playlist>("playlist_set_artwork", { id, imagePath }),
  /** Use `addTracksToPlaylist` (lib/playlistAdd) from UI code: it handles Spotify Synced copies. */
  addToPlaylist: (id: string, trackIds: string[], position?: number) =>
    call<T.AddOutcome>("playlist_add_tracks", { id, trackIds, position: position ?? null }),
  removeFromPlaylist: (id: string, position: number) => call<void>("playlist_remove_item", { id, position }),
  movePlaylistItem: (id: string, from: number, to: number) => call<void>("playlist_move_item", { id, from, to }),
  /** Save a playlist as a shareable `.bran` file (includes local audio). */
  exportBran: (id: string, path: string) => call<T.BranExportReport>("playlist_export_bran", { id, path }),
  inspectBran: (path: string) => call<T.BranSummary>("playlist_inspect_bran", { path }),
  importBran: (path: string) => call<T.BranImportReport>("playlist_import_bran", { path }),
  /** `.bran` files the app was opened with (double-click), not yet handled. */
  takePendingBran: () => call<string[]>("bran_take_pending"),

  // find alternative
  alternativesSearch: (trackId: string, provider: "local" | "soundcloud") => call<T.AlternativesResult>("alternatives_search", { trackId, provider }),
  alternativesAdd: (playlistId: string, originalTrackId: string, alternativeTrackId: string, position?: number) =>
    call<T.AddAlternativeOutcome>("alternatives_add", { playlistId, originalTrackId, alternativeTrackId, position: position ?? null }),
  alternativesList: (playlistId: string) => call<T.AlternativeRelation[]>("alternatives_list", { playlistId }),

  // queue
  getQueue: () => call<T.QueueSnapshot>("queue_get"),
  replaceQueue: (trackIds: string[], startIndex: number) => call<T.QueueSnapshot>("queue_replace", { trackIds, startIndex }),
  addToQueue: (trackIds: string[], playNext: boolean) => call<T.QueueSnapshot>("queue_add", { trackIds, playNext }),
  removeFromQueue: (position: number) => call<T.QueueSnapshot>("queue_remove", { position }),
  moveInQueue: (from: number, to: number) => call<T.QueueSnapshot>("queue_move", { from, to }),
  clearQueue: () => call<T.QueueSnapshot>("queue_clear"),
  advanceQueue: (direction: "next" | "previous", userInitiated: boolean) =>
    call<T.QueueSnapshot>("queue_advance", { direction, userInitiated }),
  setQueueCurrent: (index: number) => call<T.QueueSnapshot>("queue_set_current", { index }),
  shuffleUpcoming: () => call<T.QueueSnapshot>("queue_shuffle_upcoming"),
  savePlaybackState: (value: T.PlaybackState) => call<void>("playback_save_state", { value }),

  // providers & settings
  providersStatus: () => call<T.ProviderStatus[]>("providers_status"),
  getSettings: () => call<T.AppSettings>("settings_get"),
  updateSettings: (patch: T.SettingsPatch) => call<T.AppSettings>("settings_update", { patch }),
  /** Integrated loudness (LUFS) of a local/SoundCloud track, analysed once and cached. */
  trackLoudness: (trackId: string) => call<number | null>("track_loudness", { trackId }),
  /** Live loudness capture of the in-app Spotify player (Windows process loopback). */
  loudnessCaptureStart: (trackId: string, gain: number, settleMs: number) => call<void>("loudness_capture_start", { trackId, gain, settleMs }),
  loudnessCaptureGain: (gain: number, settleMs: number) => call<void>("loudness_capture_gain", { gain, settleMs }),
  loudnessCaptureStatus: () =>
    call<{ trackId: string; seconds: number; lufs: number | null; error: string | null } | null>("loudness_capture_status"),
  /** Ends the capture; returns the saved LUFS when enough of the song was heard. */
  loudnessCaptureStop: () => call<number | null>("loudness_capture_stop"),
  /** Lyrics (Genius text, LRCLIB time-synced lines), cached per track. `null` = none found. */
  getLyrics: (trackId: string, refresh = false) => call<T.Lyrics | null>("lyrics_get", { trackId, refresh }),
  searchProviders: (query: string, spotifyOffset?: number) =>
    call<T.ProviderSearchResults>("search_providers", { query, spotifyOffset: spotifyOffset ?? null }),

  // spotify
  spotifyConnect: () => call<void>("spotify_connect"),
  spotifyDisconnect: () => call<void>("spotify_disconnect"),
  spotifyImport: () => call<T.ImportReport>("spotify_import_library"),
  spotifyDevices: () => call<T.SpotifyDevice[]>("spotify_devices"),
  spotifyPlaybackState: () => call<T.SpotifyPlaybackState | null>("spotify_playback_state"),
  spotifyPlayTrack: (trackId: string, deviceId: string | null, positionMs: number) =>
    call<void>("spotify_play_track", { trackId, deviceId, positionMs }),
  spotifyControl: (action: "pause" | "resume" | "seek" | "volume" | "transfer", deviceId: string | null, value?: number) =>
    call<void>("spotify_control", { action, deviceId, value: value ?? null }),
  spotifyAddToQueue: (trackId: string, deviceId: string | null) => call<void>("spotify_add_to_queue", { trackId, deviceId }),
  spotifySdkToken: () => call<string>("spotify_sdk_token"),
  spotifySyncCandidates: () => call<T.SpotifyPlaylistOption[]>("spotify_sync_candidates"),
  spotifySyncEnable: (spotifyPlaylistId: string) => call<T.Playlist>("spotify_sync_enable", { spotifyPlaylistId }),
  spotifySyncDisable: (playlistId: string) => call<void>("spotify_sync_disable", { playlistId }),
  spotifySyncNow: (playlistId: string) => call<T.SyncReport>("spotify_sync_now", { playlistId }),
  spotifySyncAll: () => call<T.SyncAllReport>("spotify_sync_all"),

  // followed spotify artists
  artistsSearchSpotify: (query: string) => call<T.ArtistSearchResult[]>("artists_search_spotify", { query }),
  artistsList: () => call<T.FollowedArtist[]>("artists_list"),
  artistsGet: (id: string) => call<T.FollowedArtist>("artists_get", { id }),
  /** Starts the first sync in the background. */
  artistsFollow: (artist: T.SpotifyArtist) => call<T.FollowedArtist>("artists_follow", { artist }),
  artistsUnfollow: (id: string) => call<void>("artists_unfollow", { id }),
  artistsTracks: (id: string) => call<T.ArtistTrack[]>("artists_tracks", { id }),
  artistsSetAutoSync: (id: string, on: boolean) => call<T.FollowedArtist>("artists_set_auto_sync", { id, on }),
  artistsMarkSeen: (id: string) => call<T.FollowedArtist>("artists_mark_seen", { id }),
  artistsSyncNow: (id: string) => call<T.ArtistSyncReport>("artists_sync_now", { id }),

  // soundcloud
  soundcloudConnect: () => call<void>("soundcloud_connect"),
  soundcloudDisconnect: () => call<void>("soundcloud_disconnect"),
  soundcloudImport: () => call<T.ImportReport>("soundcloud_import_library"),
  soundcloudWebImport: (url: string) => call<T.ImportReport>("soundcloud_web_import", { url }),
  soundcloudResolveStream: (trackId: string) => call<T.StreamInfo>("soundcloud_resolve_stream", { trackId }),

  // lastfm
  lastfmConnect: () => call<string>("lastfm_connect"),
  lastfmDisconnect: () => call<void>("lastfm_disconnect"),
  lastfmNowPlaying: (trackId: string) => call<boolean>("lastfm_now_playing", { trackId }),
  lastfmSubmitPlay: (trackId: string, startedAt: number, playedMs: number) =>
    call<T.SubmitOutcome>("lastfm_submit_play", { trackId, startedAt, playedMs }),
  lastfmFlush: () => call<T.FlushReport>("lastfm_flush"),
  lastfmQueueStatus: () => call<T.ScrobbleQueueStatus>("lastfm_queue_status"),
  lastfmClearFailed: () => call<void>("lastfm_clear_failed"),

  // system
  mediaUpdate: (update: T.MediaUpdate) => call<void>("media_update", { update }),
  appInfo: () => call<{ version: string; dataDir: string; schemaVersion: number; mediaControls: boolean; taskbarControls: boolean }>("app_info"),
};
