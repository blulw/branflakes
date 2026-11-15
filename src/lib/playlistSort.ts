import type { Playlist, Track } from "./types";

export type PlaylistSort = "recent" | "name" | "created" | "tracks";

export const PLAYLIST_SORT_LABEL: Record<PlaylistSort, string> = {
  recent: "Recently played",
  name: "Name",
  created: "Recently added",
  tracks: "Most tracks",
};

const byName = (a: Playlist, b: Playlist) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });

/** Sort playlists. "recent": most recently played first; never-played ones
 * follow, newest-created first. */
export function sortPlaylists(list: Playlist[], sort: PlaylistSort): Playlist[] {
  const out = [...list];
  switch (sort) {
    case "recent":
      return out.sort((a, b) => {
        if (a.lastPlayedAt && b.lastPlayedAt) return b.lastPlayedAt.localeCompare(a.lastPlayedAt);
        if (a.lastPlayedAt) return -1;
        if (b.lastPlayedAt) return 1;
        return b.createdAt.localeCompare(a.createdAt);
      });
    case "name":
      return out.sort(byName);
    case "created":
      return out.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
    case "tracks":
      return out.sort((a, b) => b.trackCount - a.trackCount || byName(a, b));
  }
}

export type PlaylistTrackSort = "custom" | "title" | "artist" | "album" | "duration" | "source";

export const TRACK_SORT_LABEL: Record<PlaylistTrackSort, string> = {
  custom: "Custom order",
  title: "Title",
  artist: "Artist",
  album: "Album",
  duration: "Duration",
  source: "Source",
};

/** Sort playlist rows while remembering each row's real position (needed for
 * remove/move, which work on the stored order). */
export function sortPlaylistTracks(tracks: Track[], sort: PlaylistTrackSort): { track: Track; position: number }[] {
  const rows = tracks.map((track, position) => ({ track, position }));
  if (sort === "custom") return rows;
  const text = (s: string) => s.toLocaleLowerCase();
  const key: Record<Exclude<PlaylistTrackSort, "custom">, (t: Track) => string | number> = {
    title: (t) => text(t.title),
    artist: (t) => text(t.artist),
    album: (t) => text(t.album),
    duration: (t) => t.durationMs ?? 0,
    source: (t) => t.source,
  };
  const k = key[sort];
  return rows.sort((a, b) => {
    const x = k(a.track);
    const y = k(b.track);
    if (x < y) return -1;
    if (x > y) return 1;
    return a.position - b.position;
  });
}
