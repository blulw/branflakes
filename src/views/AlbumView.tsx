import { ListPlus, Loader2, Play } from "lucide-react";
import { toast } from "sonner";
import { openUrl } from "@tauri-apps/plugin-opener";
import { TrackTable } from "@/components/TrackTable";
import { Artwork } from "@/components/Artwork";
import { AddToPlaylistMenu } from "@/components/AddToPlaylistMenu";
import { SourceBadges } from "@/components/SourceBadge";
import { Button } from "@/components/ui/button";
import { useAsync } from "@/hooks/useAsync";
import { api, toApiError } from "@/lib/api";
import { formatDuration } from "@/lib/format";
import { usePlayer } from "@/playback/store";
import { useUi, type View } from "@/state/ui";
import type { Source, Track } from "@/lib/types";

type AlbumRoute = Extract<View, { name: "album" }>;

function albumUrl(remote: NonNullable<AlbumRoute["remote"]>): string | null {
  return remote.source === "spotify" ? `https://open.spotify.com/album/${encodeURIComponent(remote.id)}` : null;
}

/** One album's full track list, laid out like a playlist. Library albums read from the library; Spotify/SoundCloud albums are fetched. */
export function AlbumView({ route }: { route: AlbumRoute }) {
  const version = useUi((s) => s.libraryVersion);
  const navigate = useUi((s) => s.navigate);
  const playTracks = usePlayer((s) => s.playTracks);
  const enqueue = usePlayer((s) => s.enqueue);
  const { album, artist, remote, from } = route;

  const { data, error, loading } = useAsync<Track[]>(
    () => (remote ? api.albumTracks(remote.source, remote.id) : api.listTracks({ album, sort: "album", includeUnavailable: true })),
    [album, artist, remote?.source, remote?.id, remote ? 0 : version],
  );
  const tracks = data ?? [];
  const totalMs = tracks.reduce((n, t) => n + (t.durationMs ?? 0), 0);
  const sources = [...new Set(tracks.map((t) => t.source))] as Source[];
  const cover = remote?.artwork ?? tracks.find((t) => t.artwork)?.artwork ?? null;
  const year = remote?.year ?? tracks.find((t) => t.year)?.year ?? null;
  const unavailable = tracks.filter((t) => t.availability === "unavailable").length;
  const link = remote && albumUrl(remote);

  const back = from === "search" ? { label: "Search", view: { name: "search" } as View } : { label: "Albums", view: { name: "albums" } as View };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-end gap-3 px-4 pb-4 pt-5 @xl:gap-5 @xl:px-6">
        <Artwork src={cover} className="size-20 shrink-0 shadow-md @xl:size-36" alt={`${album} cover`} />
        <div className="min-w-0 flex-1">
          <button className="text-xs text-muted-foreground hover:underline" onClick={() => navigate(back.view)}>
            ← {back.label}
          </button>
          <p className="text-xs uppercase tracking-wide text-muted-foreground">Album</p>
          <h1 className="truncate text-2xl font-bold tracking-tight @xl:text-3xl" title={album}>
            {album}
          </h1>
          <p className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            {artist && <span className="font-medium text-foreground">{artist}</span>}
            {year && <span>· {year}</span>}
            {!loading && (
              <span>
                · {tracks.length} {tracks.length === 1 ? "track" : "tracks"}
              </span>
            )}
            {totalMs > 0 && <span>· {formatDuration(totalMs)}</span>}
            {sources.length > 0 && <SourceBadges sources={sources} />}
            {unavailable > 0 && <span className="text-destructive">{unavailable} unavailable (will be skipped)</span>}
          </p>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Button size="sm" disabled={!tracks.length} onClick={() => void playTracks(tracks, 0)}>
              <Play className="size-4 fill-current" /> Play
            </Button>
            <AddToPlaylistMenu tracks={tracks} newPlaylistName={album} align="start">
              <Button size="sm" variant="secondary" disabled={!tracks.length}>
                <ListPlus className="size-4" /> Add album to playlist
              </Button>
            </AddToPlaylistMenu>
            <Button
              size="sm"
              variant="ghost"
              disabled={!tracks.length}
              onClick={() =>
                void enqueue(tracks, false)
                  .then(() => toast.success("Album added to queue"))
                  .catch((e) => toast.error(toApiError(e).message))
              }
            >
              Add to queue
            </Button>
            {link && (
              <Button size="sm" variant="ghost" onClick={() => void openUrl(link)}>
                Open on Spotify
              </Button>
            )}
          </div>
        </div>
      </header>
      {error ? (
        <p className="p-6 text-sm text-destructive">{error.message}</p>
      ) : loading && !data ? (
        <div className="grid flex-1 place-items-center">
          <Loader2 className="size-6 animate-spin text-muted-foreground" />
        </div>
      ) : (
        <TrackTable tracks={tracks} showAlbum={false} empty="This album has no tracks." />
      )}
    </div>
  );
}
