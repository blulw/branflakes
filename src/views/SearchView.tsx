import { useState } from "react";
import { Loader2, Search } from "lucide-react";
import { TrackTable } from "@/components/TrackTable";
import { SourceBadge } from "@/components/SourceBadge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, toApiError } from "@/lib/api";
import { usePlayer } from "@/playback/store";
import type { Source, Track } from "@/lib/types";

export function SearchView() {
  const providers = usePlayer((s) => s.providers);
  const [q, setQ] = useState("");
  const [busy, setBusy] = useState(false);
  const [results, setResults] = useState<{ local: Track[]; spotify: Track[]; soundcloud: Track[]; errors: { provider: string; message: string }[] } | null>(null);
  const [spotifyOffset, setSpotifyOffset] = useState(0);

  const run = async (offset = 0) => {
    if (!q.trim()) return;
    setBusy(true);
    try {
      const [local, remote] = await Promise.all([api.listTracks({ search: q, limit: 200 }), api.searchProviders(q, offset)]);
      setSpotifyOffset(offset);
      setResults((prev) => ({
        local,
        spotify: offset > 0 && prev ? [...prev.spotify, ...remote.spotify] : remote.spotify,
        soundcloud: offset > 0 && prev ? prev.soundcloud : remote.soundcloud,
        errors: remote.errors.map((e) => ({ provider: e.provider, message: e.error.message })),
      }));
    } catch (e) {
      setResults({ local: [], spotify: [], soundcloud: [], errors: [{ provider: "search", message: toApiError(e).message }] });
    } finally {
      setBusy(false);
    }
  };

  const section = (source: Source | "library", title: string, tracks: Track[], note?: string, more?: React.ReactNode) => (
    <section className="flex min-h-64 flex-col rounded-lg border">
      <div className="flex items-center gap-2 border-b px-4 py-2 text-sm font-medium">
        {source !== "library" && <SourceBadge source={source} compact />}
        {title}
        <span className="text-xs text-muted-foreground">{tracks.length}</span>
        {note && <span className="ml-auto text-xs text-muted-foreground">{note}</span>}
      </div>
      <div className="flex h-80 min-h-0 flex-col">
        <TrackTable tracks={tracks} showAlbum={false} empty="No results" />
      </div>
      {more}
    </section>
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <header className="px-6 pb-3 pt-5">
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
        <p className="mt-2 text-xs text-muted-foreground">
          Results from streaming services are not added to your library until you choose “Add to library”. They can be played, queued and added to playlists directly.
        </p>
      </header>
      {results && (
        <div className="grid gap-4 px-6 pb-6">
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
