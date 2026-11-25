import { useState } from "react";
import { Loader2, Search } from "lucide-react";
import { create } from "zustand";
import { TrackTable } from "@/components/TrackTable";
import { SourceBadge } from "@/components/SourceBadge";
import { Artwork } from "@/components/Artwork";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, toApiError } from "@/lib/api";
import { localAlbumHit, pickAlbums } from "@/lib/albumMatch";
import { usePlayer } from "@/playback/store";
import { useUi } from "@/state/ui";
import type { AlbumHit, Source, Track } from "@/lib/types";

interface Results {
  local: Track[];
  spotify: Track[];
  soundcloud: Track[];
  errors: { provider: string; message: string }[];
}

/** Kept outside the view so results survive opening an album and coming back. */
const useSearch = create<{
  q: string;
  results: Results | null;
  albums: AlbumHit[];
  spotifyOffset: number;
  set(p: Partial<{ q: string; results: Results | null; albums: AlbumHit[]; spotifyOffset: number }>): void;
}>((set) => ({ q: "", results: null, albums: [], spotifyOffset: 0, set: (p) => set(p) }));

let searchSeq = 0;

export function SearchView() {
  const providers = usePlayer((s) => s.providers);
  const navigate = useUi((s) => s.navigate);
  const { q, results, albums, spotifyOffset, set } = useSearch();
  const setQ = (v: string) => set({ q: v });
  const [busy, setBusy] = useState(false);

  const searchAlbums = async (query: string, seq: number) => {
    const [local, remote] = await Promise.all([
      api.listAlbums().catch(() => []),
      api.searchAlbums(query).catch(() => ({ albums: [], errors: [] })),
    ]);
    if (seq !== searchSeq) return;
    set({ albums: pickAlbums(query, [...local.map(localAlbumHit), ...remote.albums]) });
  };

  const run = async (offset = 0) => {
    const query = q.trim();
    if (!query) return;
    setBusy(true);
    const seq = offset === 0 ? ++searchSeq : searchSeq;
    if (offset === 0) {
      set({ albums: [] });
      void searchAlbums(query, seq);
    }
    try {
      const [local, remote] = await Promise.all([api.listTracks({ search: query, limit: 200 }), api.searchProviders(query, offset)]);
      if (seq !== searchSeq) return;
      const prev = useSearch.getState().results;
      set({
        spotifyOffset: offset,
        results: {
          local,
          spotify: offset > 0 && prev ? [...prev.spotify, ...remote.spotify] : remote.spotify,
          soundcloud: offset > 0 && prev ? prev.soundcloud : remote.soundcloud,
          errors: remote.errors.map((e) => ({ provider: e.provider, message: e.error.message })),
        },
      });
    } catch (e) {
      set({ results: { local: [], spotify: [], soundcloud: [], errors: [{ provider: "search", message: toApiError(e).message }] } });
    } finally {
      setBusy(false);
    }
  };

  const openAlbum = (a: AlbumHit) =>
    navigate({
      name: "album",
      album: a.name,
      artist: a.artist,
      from: "search",
      remote: a.source === "local" ? undefined : { source: a.source, id: a.id, artwork: a.artwork, year: a.year },
    });

  const section = (source: Source | "library", title: string, tracks: Track[], note?: string, more?: React.ReactNode) => (
    <section className="flex min-h-64 flex-col rounded-lg border">
      <div className="flex items-center gap-2 border-b px-4 py-2 text-sm font-medium">
        {source !== "library" && <SourceBadge source={source} compact />}
        {title}
        <span className="text-xs text-muted-foreground">{tracks.length}</span>
        {note && <span className="ml-auto hidden text-xs text-muted-foreground @2xl:inline">{note}</span>}
      </div>
      <div className="flex h-80 min-h-0 flex-col">
        <TrackTable tracks={tracks} showAlbum={false} empty="No results" />
      </div>
      {more}
    </section>
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <header className="px-4 pb-3 pt-5 @xl:px-6">
        <h1 className="text-2xl font-semibold tracking-tight">Search</h1>
        <form
          className="mt-3 flex max-w-xl gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            void run(0);
          }}
        >
          <div className="relative flex-1">
            <Search className="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input autoFocus value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search your library, Spotify and SoundCloud" className="pl-8" aria-label="Search" />
          </div>
          <Button type="submit" disabled={busy || !q.trim()}>
            {busy ? <Loader2 className="size-4 animate-spin" /> : "Search"}
          </Button>
        </form>
      </header>
      {results && (
        <div className="grid gap-4 px-4 pb-6 @xl:px-6">
          {albums.length > 0 && (
            <section aria-label="Albums">
              <h2 className="mb-2 text-sm font-medium">Albums</h2>
              <div className="grid max-w-3xl grid-cols-3 gap-2 @xl:gap-4">
                {albums.map((a) => (
                  <button
                    key={`${a.source}|${a.id}`}
                    onClick={() => openAlbum(a)}
                    className="min-w-0 rounded-lg p-2 text-left hover:bg-accent"
                    title={`${a.name} · ${a.artist}`}
                  >
                    <Artwork src={a.artwork} className="aspect-square w-full" alt={`${a.name} cover`} />
                    <div className="mt-2 truncate text-sm font-medium">{a.name}</div>
                    <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                      <span className="truncate">
                        {a.artist}
                        {a.year ? ` · ${a.year}` : ""}
                      </span>
                      <span className="ml-auto shrink-0">
                        {a.source === "local" ? <span className="rounded bg-muted px-1 text-[10px]">LIBRARY</span> : <SourceBadge source={a.source} compact />}
                      </span>
                    </div>
                  </button>
                ))}
              </div>
            </section>
          )}
          {results.errors.map((e) => (
            <p key={e.provider} className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {e.provider}: {e.message}
            </p>
          ))}
          {section("library", "In your library", results.local)}
          {providers.spotify?.connected
            ? section(
                "spotify",
                "Spotify",
                results.spotify,
                "Spotify limits search to 10 results per request in Development Mode",
                <div className="border-t p-2 text-center">
                  <Button variant="ghost" size="sm" disabled={busy || results.spotify.length < spotifyOffset + 10} onClick={() => void run(spotifyOffset + 10)}>
                    Load 10 more
                  </Button>
                </div>,
              )
            : <p className="text-sm text-muted-foreground">Connect Spotify in Settings to include Spotify results.</p>}
          {providers.soundcloud?.capabilities.play
            ? section("soundcloud", providers.soundcloud.connected ? "SoundCloud" : "SoundCloud (web mode, unofficial)", results.soundcloud)
            : <p className="text-sm text-muted-foreground">Connect SoundCloud or turn on SoundCloud web mode in Settings to include SoundCloud results.</p>}
        </div>
      )}
    </div>
  );
}
