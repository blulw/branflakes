import { useDeferredValue, useState } from "react";
import { Input } from "@/components/ui/input";
import { Artwork } from "@/components/Artwork";
import { SourceBadges } from "@/components/SourceBadge";
import { useAsync } from "@/hooks/useAsync";
import { api } from "@/lib/api";
import { useUi } from "@/state/ui";

export function ArtistsView() {
  const version = useUi((s) => s.libraryVersion);
  const navigate = useUi((s) => s.navigate);
  const [filter, setFilter] = useState("");
  const f = useDeferredValue(filter.toLowerCase());
  const { data } = useAsync(() => api.listArtists(), [version]);
  const list = (data ?? []).filter((a) => a.name.toLowerCase().includes(f));
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex flex-wrap items-end justify-between gap-x-4 gap-y-2 px-4 pb-3 pt-5 @xl:px-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Artists</h1>
          <p className="text-xs text-muted-foreground">{list.length} artists across all sources</p>
        </div>
        <Input className="h-8 w-full @xl:w-64" placeholder="Filter artists" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter artists" />
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-4 @xl:px-4">
        {list.map((a) => (
          <button
            key={a.name}
            onClick={() => navigate({ name: "artist", artist: a.name })}
            className="flex w-full items-center gap-3 rounded-md px-3 py-2 text-left text-sm hover:bg-accent"
          >
            <span className="flex-1 truncate font-medium">{a.name}</span>
            <span className="hidden @sm:contents">
              <SourceBadges sources={a.sources} />
            </span>
            <span className="w-20 shrink-0 text-right text-xs tabular-nums text-muted-foreground">{a.trackCount} tracks</span>
          </button>
        ))}
        {list.length === 0 && <p className="p-6 text-sm text-muted-foreground">No artists yet.</p>}
      </div>
    </div>
  );
}

export function AlbumsView() {
  const version = useUi((s) => s.libraryVersion);
  const navigate = useUi((s) => s.navigate);
  const [filter, setFilter] = useState("");
  const f = useDeferredValue(filter.toLowerCase());
  const { data } = useAsync(() => api.listAlbums(), [version]);
  const list = (data ?? []).filter((a) => a.name.toLowerCase().includes(f) || a.artist.toLowerCase().includes(f));
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex flex-wrap items-end justify-between gap-x-4 gap-y-2 px-4 pb-3 pt-5 @xl:px-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Albums</h1>
          <p className="text-xs text-muted-foreground">{list.length} albums</p>
        </div>
        <Input className="h-8 w-full @xl:w-64" placeholder="Filter albums" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter albums" />
      </header>
      <div className="grid min-h-0 flex-1 auto-rows-min grid-cols-[repeat(auto-fill,minmax(130px,1fr))] gap-2 overflow-y-auto px-4 pb-6 @xl:grid-cols-[repeat(auto-fill,minmax(160px,1fr))] @xl:gap-4 @xl:px-6">
        {list.map((a) => (
          <button key={`${a.name}|${a.artist}`} onClick={() => navigate({ name: "album", album: a.name, artist: a.artist, from: "albums" })} className="group rounded-lg p-2 text-left hover:bg-accent">
            <Artwork src={a.artwork} className="aspect-square w-full" alt={`${a.name} cover`} />
            <div className="mt-2 truncate text-sm font-medium">{a.name}</div>
            <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <span className="truncate">
                {a.artist}
                {a.year ? ` · ${a.year}` : ""}
              </span>
              <span className="ml-auto">
                <SourceBadges sources={a.sources} />
              </span>
            </div>
          </button>
        ))}
        {list.length === 0 && <p className="col-span-full p-6 text-sm text-muted-foreground">No albums yet.</p>}
      </div>
    </div>
  );
}
