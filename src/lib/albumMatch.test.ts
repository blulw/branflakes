import { describe, expect, it } from "vitest";
import { albumScore, normalize, pickAlbums } from "./albumMatch";
import type { AlbumHit } from "./types";

const hit = (p: Partial<AlbumHit>): AlbumHit => ({
  source: "spotify",
  id: p.name ?? "x",
  name: "",
  artist: "",
  year: null,
  artwork: null,
  trackCount: 10,
  albumType: "album",
  ...p,
});

describe("albumMatch", () => {
  it("normalizes accents, case and punctuation", () => {
    expect(normalize("Beyoncé – LEMONADE!")).toBe("beyonce lemonade");
    expect(normalize("Don't")).toBe("dont");
  });

  it("requires every query word in the album name or artist", () => {
    expect(albumScore("abbey road", hit({ name: "Abbey Road", artist: "The Beatles" }))).toBe(4);
    expect(albumScore("beatles abbey", hit({ name: "Abbey Road", artist: "The Beatles" }))).toBeGreaterThan(0);
    expect(albumScore("abbey road", hit({ name: "Let It Be", artist: "The Beatles" }))).toBe(0);
  });

  it("skips short singles from providers", () => {
    expect(albumScore("gods plan", hit({ name: "God's Plan", albumType: "single", trackCount: 1 }))).toBe(0);
    expect(albumScore("gods plan", hit({ name: "God's Plan", albumType: "single", trackCount: 4 }))).toBe(4);
  });

  it("picks at most three, best first, library before providers, without duplicates", () => {
    const picked = pickAlbums("abbey road", [
      hit({ id: "1", name: "Abbey Road (Remastered)", artist: "The Beatles" }),
      hit({ id: "2", name: "Abbey Road", artist: "The Beatles" }),
      hit({ id: "3", source: "local", name: "Abbey Road", artist: "The Beatles" }),
      hit({ id: "4", name: "Abbey Road Live", artist: "Someone" }),
      hit({ id: "5", name: "Abbey Road Sessions", artist: "Other" }),
      hit({ id: "6", name: "Unrelated", artist: "Nobody" }),
    ]);
    expect(picked.map((a) => a.id)).toEqual(["3", "1", "4"]);
  });
});
