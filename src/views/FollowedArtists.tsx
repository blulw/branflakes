import { useEffect, useMemo, useState } from "react";
import { CheckCheck, Loader2, Plus, RefreshCw, Search, UserMinus } from "lucide-react";
import { toast } from "sonner";
import { Artwork } from "@/components/Artwork";
import { SpotifyGlyph } from "@/components/PlaylistKind";
import { TrackTable } from "@/components/TrackTable";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { api, toApiError } from "@/lib/api";
import { cn } from "@/lib/utils";
import { usePlayer } from "@/playback/store";
import { useUi } from "@/state/ui";
import type { ArtistSearchResult, ArtistTrack, FollowedArtist } from "@/lib/types";

/** "just now", "5 min ago", "3 h ago", "2 days ago". */
export function timeAgo(iso: string | null, now = Date.now()): string {
  if (!iso) return "never";
  const s = Math.max(0, (now - new Date(iso).getTime()) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  const d = Math.floor(s / 86400);
  return `${d} day${d === 1 ? "" : "s"} ago`;
}

function syncSummary(a: FollowedArtist, syncing: boolean): string {
  if (syncing) return a.lastSyncedAt ? "Syncing…" : "Gathering every release…";
  if (a.lastError) return `Last sync failed: ${a.lastError}`;
  return a.lastSyncedAt ? `Synced ${timeAgo(a.lastSyncedAt)}` : "Not synced yet";
}

function NewBadge({ count }: { count?: number }) {
  return (
    <span className="shrink-0 rounded bg-primary px-1 text-[10px] font-semibold leading-4 text-primary-foreground">
      {count != null ? `${count} NEW` : "NEW"}
    </span>
  );
}

function useSpotifyConnected() {
  return usePlayer((s) => !!s.providers.spotify?.connected);
}

/** The list of followed Spotify artists, with search to follow more. */
export function FollowedArtistsView() {
  const { followed, refreshFollowed, navigate, artistSyncing, settings, refreshSettings } = useUi();
  const connected = useSpotifyConnected();
  const [q, setQ] = useState("");
  const [busy, setBusy] = useState(false);
  const [results, setResults] = useState<ArtistSearchResult[] | null>(null);
  const [hours, setHours] = useState("");

  useEffect(() => {
    void refreshFollowed().catch(() => undefined);
  }, [refreshFollowed]);

  const search = async () => {
    if (!q.trim()) return;
    setBusy(true);
    try {
      setResults(await api.artistsSearchSpotify(q));
    } catch (e) {
      toast.error(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  };

  const follow = async (a: ArtistSearchResult) => {
    try {
      const f = await api.artistsFollow(a);
      await refreshFollowed();
      navigate({ name: "followedArtist", id: f.id });
    } catch (e) {
      toast.error(toApiError(e).message);
    }
  };

  const saveHours = async () => {
    try {
      await api.updateSettings({ artistSyncIntervalHours: Number(hours) });
      setHours("");
      await refreshSettings();
      toast.success("Saved");
    } catch (e) {
      toast.error(toApiError(e).message);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <header className="px-4 pb-3 pt-5 @xl:px-6">
        <h1 className="text-2xl font-semibold tracking-tight">Followed artists</h1>
        <p className="text-xs text-muted-foreground">Follow a Spotify artist to collect every track they release and see what's new.</p>
        {connected ? (
          <form
            className="mt-3 flex max-w-xl gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              void search();
            }}
          >
            <div className="relative flex-1">
              <Search className="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Find an artist on Spotify" className="pl-8" aria-label="Find an artist on Spotify" />
            </div>
            <Button type="submit" disabled={busy || !q.trim()}>
              {busy ? <Loader2 className="size-4 animate-spin" /> : "Search"}
            </Button>
          </form>
        ) : (
          <p className="mt-3 text-sm text-muted-foreground">Connect Spotify in Settings to follow artists.</p>
        )}
      </header>

      {results && (
        <section className="mx-4 mb-4 rounded-lg border @xl:mx-6">
          <div className="flex items-center justify-between border-b px-4 py-2 text-sm font-medium">
            <span className="flex items-center gap-2">
              <SpotifyGlyph className="size-4 text-source-spotify" /> Artists on Spotify
            </span>
            <Button variant="ghost" size="sm" className="h-7" onClick={() => setResults(null)}>
              Close
            </Button>
          </div>
          {results.length === 0 && <p className="p-4 text-sm text-muted-foreground">No artists found.</p>}
          <ul>
            {results.map((a) => (
              <li key={a.id} className="flex items-center gap-3 px-3 py-2 hover:bg-accent/60">
                <Artwork src={a.imageUrl} className="size-10 shrink-0 rounded-full" alt="" />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{a.name}</div>
                  {a.genres.length > 0 && <div className="truncate text-xs text-muted-foreground">{a.genres.slice(0, 3).join(", ")}</div>}
                </div>
                {a.followed ? (
                  <Button size="sm" variant="ghost" onClick={() => navigate({ name: "followedArtist", id: a.id })}>
                    Following
                  </Button>
                ) : (
                  <Button size="sm" variant="secondary" onClick={() => void follow(a)}>
                    <Plus className="size-4" /> Follow
                  </Button>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      <div className="grid auto-rows-min grid-cols-[repeat(auto-fill,minmax(130px,1fr))] gap-2 px-4 @xl:grid-cols-[repeat(auto-fill,minmax(160px,1fr))] @xl:gap-4 @xl:px-6">
        {followed.map((a) => (
          <button key={a.id} onClick={() => navigate({ name: "followedArtist", id: a.id })} className="rounded-lg p-2 text-left hover:bg-accent">
            <span className="relative block">
              <Artwork src={a.imageUrl} className="aspect-square w-full rounded-full" alt={`${a.name}`} />
              {a.newCount > 0 && (
                <span className="absolute right-1 top-1">
                  <NewBadge count={a.newCount} />
                </span>
              )}
            </span>
            <div className="mt-2 truncate text-center text-sm font-medium">{a.name}</div>
            <div className="truncate text-center text-xs text-muted-foreground">
              {artistSyncing[a.id] ? "Syncing…" : `${a.trackCount} tracks${a.autoSync ? "" : " · auto sync off"}`}
            </div>
          </button>
        ))}
        {followed.length === 0 && <p className="col-span-full py-6 text-sm text-muted-foreground">You don't follow any artists yet. Search for one above.</p>}
      </div>

      <div className="mt-auto flex flex-wrap items-end gap-2 px-4 py-5 @xl:px-6">
        <div className="grid gap-1">
          <Label htmlFor="artist-sync-hours" className="text-xs">
            Auto sync every (hours)
          </Label>
          <Input
            id="artist-sync-hours"
            className="h-8 w-24"
            inputMode="numeric"
            placeholder={String(settings?.artistSyncIntervalHours ?? 24)}
            value={hours}
            onChange={(e) => setHours(e.target.value.replace(/\D/g, ""))}
          />
        </div>
        <Button size="sm" variant="secondary" disabled={!hours} onClick={() => void saveHours()}>
          Save
        </Button>
      </div>
    </div>
  );
}

/** One followed artist: every track they released, new ones first. */
export function FollowedArtistView({ id }: { id: string }) {
  const { followed, refreshFollowed, navigate, artistSyncing, settings } = useUi();
  const artist = followed.find((a) => a.id === id) ?? null;
  const syncing = !!artistSyncing[id];
  const [tracks, setTracks] = useState<ArtistTrack[] | null>(null);
  const [newOnly, setNewOnly] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void refreshFollowed().catch(() => undefined);
  }, [refreshFollowed]);

  // Reload the tracks whenever a sync or "mark as seen" changes the artist.
  const stamp = artist ? `${artist.lastSyncedAt}|${artist.trackCount}|${artist.newCount}` : "";
  useEffect(() => {
    let live = true;
    api
      .artistsTracks(id)
      .then((t) => live && setTracks(t))
      .catch((e) => live && toast.error(toApiError(e).message));
    return () => {
      live = false;
    };
  }, [id, stamp]);

  const shown = useMemo(() => (newOnly ? (tracks ?? []).filter((t) => t.isNew) : tracks ?? []), [tracks, newOnly]);
  const isNew = useMemo(() => new Set((tracks ?? []).filter((t) => t.isNew).map((t) => t.id)), [tracks]);

  if (!artist) {
    return <div className="grid flex-1 place-items-center p-10 text-sm text-muted-foreground">{followed.length ? "You no longer follow this artist." : <Loader2 className="size-5 animate-spin" />}</div>;
  }

  const act = async (f: () => Promise<unknown>) => {
    try {
      await f();
      await refreshFollowed();
    } catch (e) {
      toast.error(toApiError(e).message);
    }
  };

  const syncNow = async () => {
    setBusy(true);
    try {
      const r = await api.artistsSyncNow(id);
      if (r.firstSync) toast.success(`Found ${r.totalTracks} tracks by ${artist.name}`);
      else if (r.added > 0) toast.success(`${r.added} new track${r.added === 1 ? "" : "s"} by ${artist.name}`);
      else toast.success("No new tracks");
    } catch (e) {
      toast.error(`Couldn't sync ${artist.name}`, { description: toApiError(e).message });
    } finally {
      setBusy(false);
      await refreshFollowed().catch(() => undefined);
    }
  };

  const unfollow = () =>
    act(async () => {
      await api.artistsUnfollow(id);
      navigate({ name: "followed" });
    });

  const hours = settings?.artistSyncIntervalHours ?? 24;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-end gap-3 px-4 pb-3 pt-5 @xl:gap-5 @xl:px-6">
        <Artwork src={artist.imageUrl} className="size-20 shrink-0 rounded-full shadow-md @xl:size-36" alt={`${artist.name}`} />
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-1.5 text-xs uppercase tracking-wide text-muted-foreground">
            <SpotifyGlyph className="size-3.5 text-source-spotify" /> Followed artist
          </p>
          <h1 className="truncate text-2xl font-bold tracking-tight @xl:text-3xl" title={artist.name}>
            {artist.name}
          </h1>
          {artist.genres.length > 0 && <p className="truncate text-xs text-muted-foreground">{artist.genres.slice(0, 4).join(", ")}</p>}
          <p className={cn("mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground", artist.lastError && !syncing && "text-destructive")}>
            <span>
              {artist.trackCount} {artist.trackCount === 1 ? "track" : "tracks"}
            </span>
            {artist.newCount > 0 && <NewBadge count={artist.newCount} />}
            <span>· {syncSummary(artist, syncing)}</span>
          </p>
        </div>
      </header>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 pb-3 @xl:px-6">
        <div className="flex items-center gap-2">
          <Switch id="artist-auto-sync" checked={artist.autoSync} onCheckedChange={(on) => void act(() => api.artistsSetAutoSync(id, on))} />
          <Label htmlFor="artist-auto-sync" className="text-sm font-normal">
            Auto sync <span className="text-muted-foreground">(every {hours === 1 ? "hour" : `${hours} hours`})</span>
          </Label>
        </div>
        <Button size="sm" variant="secondary" disabled={busy || syncing} onClick={() => void syncNow()}>
          <RefreshCw className={cn("size-4", (busy || syncing) && "animate-spin")} /> Sync now
        </Button>
        {artist.newCount > 0 && (
          <>
            <div className="flex items-center gap-2">
              <Switch id="artist-new-only" checked={newOnly} onCheckedChange={setNewOnly} />
              <Label htmlFor="artist-new-only" className="text-sm font-normal">
                New only
              </Label>
            </div>
            <Button size="sm" variant="ghost" onClick={() => void act(() => api.artistsMarkSeen(id)).then(() => setNewOnly(false))}>
              <CheckCheck className="size-4" /> Mark all as seen
            </Button>
          </>
        )}
        <Button size="sm" variant="ghost" className="ml-auto text-muted-foreground" onClick={() => void unfollow()}>
          <UserMinus className="size-4" /> Unfollow
        </Button>
      </div>

      <TrackTable
        tracks={shown}
        rowAccessory={(t) => (isNew.has(t.id) ? <NewBadge /> : null)}
        empty={
          tracks == null || (syncing && !artist.lastSyncedAt) ? (
            <span className="flex items-center gap-2">
              <Loader2 className="size-4 animate-spin" /> Gathering every release…
            </span>
          ) : artist.lastError && !artist.lastSyncedAt ? (
            "Couldn't read this artist's releases yet. Try Sync now."
          ) : (
            "No tracks"
          )
        }
      />
    </div>
  );
}
