// Picks the few albums worth showing above search results. Providers return
// albums for almost any query, so only ones whose name/artist actually cover
// what was typed are kept.
import type { AlbumHit, AlbumSummary, Source } from "./types";

export function normalize(s: string): string {
  return s
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/['’]/g, "")
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();
}

export function localAlbumHit(a: AlbumSummary): AlbumHit {
  return {
    source: "local",
    id: `${a.name}\u0000${a.artist}`,
    name: a.name,
    artist: a.artist,
    year: a.year,
    artwork: a.artwork,
    trackCount: a.trackCount,
    albumType: "album",
  };
}

const SOURCE_RANK: Record<Source, number> = { local: 0, spotify: 1, soundcloud: 2 };

/** 0 = not suitable; higher is a better match. */
export function albumScore(query: string, a: Pick<AlbumHit, "name" | "artist" | "albumType" | "trackCount" | "source">): number {
  const q = normalize(query);
  const name = normalize(a.name);
  if (!q || !name) return 0;
  // A one- or two-track "single" is really a song; the track list already shows it.
  if (a.source !== "local" && a.albumType === "single" && (a.trackCount ?? 0) < 3) return 0;
  const words = `${name} ${normalize(a.artist)}`.split(" ");
  const covered = q.split(" ").every((t) => words.some((w) => w.startsWith(t)));
  if (!covered) return 0;
  if (name === q) return 4;
  if (name.startsWith(q) || q.startsWith(name)) return 3;
  if (name.includes(q)) return 2;
  return 1;
}

/** Up to `max` suitable albums: best matches first, library before providers, no duplicates. */
export function pickAlbums(query: string, hits: AlbumHit[], max = 3): AlbumHit[] {
  const scored = hits
    .map((a, i) => ({ a, i, s: albumScore(query, a) }))
    .filter((x) => x.s > 0)
    .sort((x, y) => y.s - x.s || SOURCE_RANK[x.a.source] - SOURCE_RANK[y.a.source] || x.i - y.i);
  const seen = new Set<string>();
  const out: AlbumHit[] = [];
  for (const { a } of scored) {
    const key = `${normalize(a.name)}|${normalize(a.artist.split(",")[0] ?? "")}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(a);
    if (out.length === max) break;
  }
  return out;
}
