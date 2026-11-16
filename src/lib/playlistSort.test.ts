import { describe, expect, it } from "vitest";
import { sortPlaylists, sortPlaylistTracks } from "./playlistSort";
import type { Playlist, Track } from "./types";

const pl = (name: string, created: string, lastPlayed: string | null, count = 0): Playlist => ({
  id: name,
  name,
  description: "",
  origin: null,
  trackCount: count,
  artworkPath: null,
  lastPlayedAt: lastPlayed,
  sync: null,
  forkedFrom: null,
  forkedFromName: null,
  forkNote: null,
  createdAt: created,
  updatedAt: created,
});

describe("sortPlaylists", () => {
  const list = [
    pl("b-old-played", "2026-01-01", "2026-03-01", 5),
    pl("a-never", "2026-02-01", null, 9),
    pl("c-recent", "2026-01-05", "2026-11-01", 1),
    pl("d-never-newer", "2026-04-01", null, 9),
  ];

  it("defaults to most recently played, then never-played by newest", () => {
    expect(sortPlaylists(list, "recent").map((p) => p.name)).toEqual(["c-recent", "b-old-played", "d-never-newer", "a-never"]);
  });
  it("sorts by name, created and track count", () => {
    expect(sortPlaylists(list, "name").map((p) => p.name)).toEqual(["a-never", "b-old-played", "c-recent", "d-never-newer"]);
    expect(sortPlaylists(list, "created")[0].name).toBe("d-never-newer");
    expect(sortPlaylists(list, "tracks").map((p) => p.name)).toEqual(["a-never", "d-never-newer", "b-old-played", "c-recent"]);
  });
  it("does not mutate the input", () => {
    const copy = [...list];
    sortPlaylists(list, "name");
    expect(list).toEqual(copy);
  });
});

describe("sortPlaylistTracks", () => {
  const t = (title: string, durationMs: number) => ({ id: title, title, artist: "", album: "", durationMs, source: "local" }) as unknown as Track;
  const tracks = [t("b", 3), t("a", 2), t("c", 1)];
  it("keeps stored positions for remove/move", () => {
    const rows = sortPlaylistTracks(tracks, "title");
    expect(rows.map((r) => r.track.title)).toEqual(["a", "b", "c"]);
    expect(rows.map((r) => r.position)).toEqual([1, 0, 2]);
    expect(sortPlaylistTracks(tracks, "custom").map((r) => r.position)).toEqual([0, 1, 2]);
    expect(sortPlaylistTracks(tracks, "duration").map((r) => r.track.title)).toEqual(["c", "a", "b"]);
  });
});
