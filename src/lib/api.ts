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
  createPlaylist: (name: string) => call<T.Playlist>("playlist_create", { name }),
  renamePlaylist: (id: string, name: string) => call<void>("playlist_rename", { id, name }),
  deletePlaylist: (id: string) => call<void>("playlist_delete", { id }),
  playlistTracks: (id: string) => call<T.Track[]>("playlist_tracks", { id }),
  markPlaylistPlayed: (id: string) => call<void>("playlist_mark_played", { id }),
  setPlaylistArtwork: (id: string, imagePath: string | null) => call<T.Playlist>("playlist_set_artwork", { id, imagePath }),
  addToPlaylist: (id: string, trackIds: string[]) => call<void>("playlist_add_tracks", { id, trackIds }),
  removeFromPlaylist: (id: string, position: number) => call<void>("playlist_remove_item", { id, position }),
  movePlaylistItem: (id: string, from: number, to: number) => call<void>("playlist_move_item", { id, from, to }),

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
  appInfo: () => call<{ version: string; dataDir: string; schemaVersion: number; mediaControls: boolean }>("app_info"),
};
