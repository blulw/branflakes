import { useDeferredValue, useState } from "react";
import { Play, Search } from "lucide-react";
import { TrackTable } from "@/components/TrackTable";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useAsync } from "@/hooks/useAsync";
import { api } from "@/lib/api";
import { usePlayer } from "@/playback/store";
import { useUi } from "@/state/ui";
import type { SortKey, Source, TrackQuery } from "@/lib/types";

export function LibraryView({ artist, album, title }: { artist?: string; album?: string; title?: string }) {
  const version = useUi((s) => s.libraryVersion);
  const navigate = useUi((s) => s.navigate);
  const playTracks = usePlayer((s) => s.playTracks);
  const [search, setSearch] = useState("");
  const [source, setSource] = useState<Source | "all">("all");
  const [sort, setSort] = useState<SortKey>(album ? "album" : artist ? "album" : "title");
  const [descending, setDescending] = useState(false);
  const [availability, setAvailability] = useState<"all" | "playable">("all");
  const q = useDeferredValue(search);

  const query: TrackQuery = {
    search: q || undefined,
    source: source === "all" ? undefined : source,
    artist,
    album,
    sort,
    descending,
    includeUnavailable: availability === "all",
  };
  const { data: tracks, error, loading } = useAsync(() => api.listTracks(query), [JSON.stringify(query), version]);

  const onSort = (k: SortKey) => {
    if (k === sort) setDescending((d) => !d);
    else {
      setSort(k);
      setDescending(false);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex flex-wrap items-center gap-3 px-4 pb-3 pt-5 @xl:px-6">
        <div className="mr-auto min-w-0">
          {(artist || album) && (
            <button className="text-xs text-muted-foreground hover:underline" onClick={() => navigate({ name: album ? "albums" : "artists" })}>
              ← {album ? "Albums" : "Artists"}
            </button>
          )}
          <h1 className="truncate text-2xl font-semibold tracking-tight">{title ?? "Songs"}</h1>
          <p className="text-xs text-muted-foreground">{loading ? "Loading…" : `${tracks?.length ?? 0} tracks`}</p>
        </div>
        <Button size="sm" disabled={!tracks?.length} onClick={() => tracks && void playTracks(tracks, 0)}>
          <Play className="size-4 fill-current" /> Play
        </Button>
      </header>
      <div className="flex flex-wrap items-center gap-2 px-4 pb-3 @xl:px-6">
        <div className="relative w-full @xl:w-72">
          <Search className="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Filter by title, artist, album, genre" className="h-8 pl-8" aria-label="Filter library" />
        </div>
        <Tabs value={source} onValueChange={(v) => setSource(v as Source | "all")}>
          <TabsList className="h-8">
            <TabsTrigger value="all">All</TabsTrigger>
            <TabsTrigger value="local">Local</TabsTrigger>
            <TabsTrigger value="spotify">Spotify</TabsTrigger>
            <TabsTrigger value="soundcloud">SoundCloud</TabsTrigger>
          </TabsList>
        </Tabs>
        <Select value={availability} onValueChange={(v) => setAvailability(v as "all" | "playable")}>
          <SelectTrigger size="sm" className="w-40" aria-label="Availability filter">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All tracks</SelectItem>
            <SelectItem value="playable">Hide unavailable</SelectItem>
          </SelectContent>
        </Select>
        <Select value={sort} onValueChange={(v) => setSort(v as SortKey)}>
          <SelectTrigger size="sm" className="w-36" aria-label="Sort by">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="title">Title</SelectItem>
            <SelectItem value="artist">Artist</SelectItem>
            <SelectItem value="album">Album</SelectItem>
            <SelectItem value="duration">Duration</SelectItem>
            <SelectItem value="year">Year</SelectItem>
            <SelectItem value="added">Date added</SelectItem>
            <SelectItem value="source">Source</SelectItem>
          </SelectContent>
        </Select>
      </div>
      {error ? (
        <p className="p-6 text-sm text-destructive">{error.message}</p>
      ) : (
        <TrackTable
          tracks={tracks ?? []}
          sort={sort}
          descending={descending}
          onSort={onSort}
          showAlbum={!album}
          empty={
            q || source !== "all" ? (
              "No tracks match these filters."
            ) : (
              <div className="max-w-sm space-y-2 text-center">
                <p className="font-medium text-foreground">Your library is empty</p>
                <p>Add a music folder or connect Spotify / SoundCloud in Settings, then import your library.</p>
                <Button size="sm" variant="secondary" onClick={() => navigate({ name: "settings" })}>
                  Open Settings
                </Button>
              </div>
            )
          }
        />
      )}
    </div>
  );
}
