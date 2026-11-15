import { useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ImagePlus, Pencil, Play, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { TrackTable } from "@/components/TrackTable";
import { PlaylistCover } from "@/components/PlaylistCover";
import { SourceBadges } from "@/components/SourceBadge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useAsync } from "@/hooks/useAsync";
import { api, toApiError } from "@/lib/api";
import { formatDuration } from "@/lib/format";
import { sortPlaylistTracks, TRACK_SORT_LABEL, type PlaylistTrackSort } from "@/lib/playlistSort";
import { usePlayer } from "@/playback/store";
import { useUi } from "@/state/ui";
import { AddSongsDialog } from "./AddSongsDialog";
import type { Source } from "@/lib/types";

export function PlaylistView({ id }: { id: string }) {
  const { playlists, refreshPlaylists, navigate, libraryVersion } = useUi();
  const playTracks = usePlayer((s) => s.playTracks);
  const [rev, setRev] = useState(0);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [sort, setSort] = useState<PlaylistTrackSort>("custom");
  const playlist = playlists.find((p) => p.id === id);
  const { data: tracks } = useAsync(() => api.playlistTracks(id), [id, rev, libraryVersion]);

  const rows = useMemo(() => sortPlaylistTracks(tracks ?? [], sort), [tracks, sort]);
  const displayed = useMemo(() => rows.map((r) => r.track), [rows]);
  const existing = useMemo(() => new Set((tracks ?? []).map((t) => t.id)), [tracks]);
  const sources = [...new Set((tracks ?? []).map((t) => t.source))] as Source[];
  const unavailable = (tracks ?? []).filter((t) => t.availability === "unavailable").length;
  const totalMs = (tracks ?? []).reduce((n, t) => n + (t.durationMs ?? 0), 0);

  const act = async (fn: () => Promise<unknown>) => {
    try {
      await fn();
      setRev((r) => r + 1);
      await refreshPlaylists();
    } catch (e) {
      toast.error(toApiError(e).message);
    }
  };

  // Starting playback from this page counts as "playing the playlist".
  const markPlayed = () => {
    void api
      .markPlaylistPlayed(id)
      .then(refreshPlaylists)
      .catch(() => undefined);
  };

  const chooseCover = async () => {
    const p = await open({ multiple: false, filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp"] }] });
    if (typeof p === "string") await act(() => api.setPlaylistArtwork(id, p));
  };

  if (!playlist) {
    return <p className="p-6 text-sm text-muted-foreground">This playlist no longer exists.</p>;
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-end gap-5 px-6 pb-4 pt-5">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button className="group relative shrink-0 rounded-md focus-visible:outline-2" aria-label="Change playlist cover">
              <PlaylistCover path={playlist.artworkPath} className="size-36 shadow-md" alt={`${playlist.name} cover`} />
              <span className="absolute inset-0 grid place-items-center rounded-md bg-black/50 text-xs font-medium text-white opacity-0 transition-opacity group-hover:opacity-100">
                <span className="flex flex-col items-center gap-1">
                  <ImagePlus className="size-5" /> Change cover
                </span>
              </span>
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            <DropdownMenuItem onSelect={() => void chooseCover()}>Choose image…</DropdownMenuItem>
            {playlist.artworkPath && (
              <DropdownMenuItem variant="destructive" onSelect={() => void act(() => api.setPlaylistArtwork(id, null))}>
                Remove cover
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>

        <div className="min-w-0 flex-1">
          <p className="text-xs uppercase tracking-wide text-muted-foreground">
            Playlist{playlist.origin ? ` · imported from ${playlist.origin.startsWith("spotify") ? "Spotify" : "SoundCloud"}` : ""}
          </p>
          {renaming != null ? (
            <form
              onSubmit={(e) => {
                e.preventDefault();
                void act(() => api.renamePlaylist(id, renaming)).then(() => setRenaming(null));
              }}
            >
              <Input autoFocus value={renaming} onChange={(e) => setRenaming(e.target.value)} onBlur={() => setRenaming(null)} className="h-11 text-2xl font-bold" aria-label="Playlist name" />
            </form>
          ) : (
            <h1 className="truncate text-3xl font-bold tracking-tight" title={playlist.name}>
              {playlist.name}
            </h1>
          )}
          <p className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            {playlist.trackCount} {playlist.trackCount === 1 ? "track" : "tracks"}
            {totalMs > 0 && <span>· {formatDuration(totalMs)}</span>}
            {sources.length > 0 && <SourceBadges sources={sources} />}
            {unavailable > 0 && <span className="text-destructive">{unavailable} unavailable (will be skipped)</span>}
            {playlist.lastPlayedAt && <span>· last played {new Date(playlist.lastPlayedAt).toLocaleDateString()}</span>}
          </p>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              disabled={!displayed.length}
              onClick={() => {
                void playTracks(displayed, 0);
                markPlayed();
              }}
            >
              <Play className="size-4 fill-current" /> Play
            </Button>
            <Button size="sm" variant="secondary" onClick={() => setAdding(true)}>
              <Plus className="size-4" /> Add songs
            </Button>
            <Select value={sort} onValueChange={(v) => setSort(v as PlaylistTrackSort)}>
              <SelectTrigger size="sm" className="w-40" aria-label="Sort tracks">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(Object.keys(TRACK_SORT_LABEL) as PlaylistTrackSort[]).map((k) => (
                  <SelectItem key={k} value={k}>
                    {TRACK_SORT_LABEL[k]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Button size="sm" variant="ghost" aria-label="Rename playlist" onClick={() => setRenaming(playlist.name)}>
              <Pencil className="size-4" />
            </Button>
            <Button
              size="sm"
              variant="ghost"
              aria-label="Delete playlist"
              onClick={() => {
                if (confirm(`Delete playlist “${playlist.name}”? Tracks stay in your library.`)) {
                  void act(() => api.deletePlaylist(id)).then(() => navigate({ name: "library" }));
                }
              }}
            >
              <Trash2 className="size-4" />
            </Button>
          </div>
        </div>
      </header>

      <TrackTable
        tracks={displayed}
        onPlayStart={markPlayed}
        empty={
          <div className="space-y-3 text-center">
            <p>This playlist is empty.</p>
            <Button size="sm" onClick={() => setAdding(true)}>
              <Plus className="size-4" /> Add songs
            </Button>
          </div>
        }
        extraMenu={(_t, i) => {
          const pos = rows[i]?.position ?? i;
          const custom = sort === "custom";
          return (
            <>
              <DropdownMenuItem disabled={!custom || pos === 0} onSelect={() => void act(() => api.movePlaylistItem(id, pos, pos - 1))}>
                Move up{!custom && " (custom order only)"}
              </DropdownMenuItem>
              <DropdownMenuItem disabled={!custom || pos === rows.length - 1} onSelect={() => void act(() => api.movePlaylistItem(id, pos, pos + 1))}>
                Move down{!custom && " (custom order only)"}
              </DropdownMenuItem>
              <DropdownMenuItem variant="destructive" onSelect={() => void act(() => api.removeFromPlaylist(id, pos))}>
                Remove from playlist
              </DropdownMenuItem>
            </>
          );
        }}
      />

      <AddSongsDialog
        open={adding}
        onOpenChange={setAdding}
        playlistId={id}
        playlistName={playlist.name}
        existing={existing}
        onAdded={() => {
          setRev((r) => r + 1);
          void refreshPlaylists();
        }}
      />
    </div>
  );
}
