import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { AlertCircle, ImagePlus, Link2, Pencil, Play, Plus, RefreshCw, ScanSearch, Share2, Trash2, Unlink } from "lucide-react";
import { toast } from "sonner";
import { TrackTable } from "@/components/TrackTable";
import { PlaylistCover } from "@/components/PlaylistCover";
import { PlaylistKindIcon, playlistKindLabel } from "@/components/PlaylistKind";
import { SourceBadges } from "@/components/SourceBadge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useAsync } from "@/hooks/useAsync";
import { api, toApiError } from "@/lib/api";
import { exportPlaylistAsBran } from "@/lib/bran";
import { formatDuration } from "@/lib/format";
import { sortPlaylistTracks, TRACK_SORT_LABEL, type PlaylistTrackSort } from "@/lib/playlistSort";
import { cn } from "@/lib/utils";
import { usePlayer } from "@/playback/store";
import { useUi } from "@/state/ui";
import { AddSongsDialog } from "./AddSongsDialog";
import { AlternativesDialog } from "@/components/AlternativesDialog";
import { DeleteSyncedPlaylistDialog } from "@/components/DeleteSyncedPlaylistDialog";
import type { Playlist, Source, Track } from "@/lib/types";

/** Unavailable Spotify tracks get "Find alternative" (temporary errors never mark a track unavailable). */
export function canFindAlternative(t: Pick<Track, "source" | "availability">): boolean {
  return t.source === "spotify" && t.availability === "unavailable";
}

/** "just now", "5 min ago", "2 h ago", or a date. */
export function relativeTime(iso: string, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - new Date(iso).getTime()) / 1000));
  if (s < 45) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return new Date(iso).toLocaleDateString();
}

function spotifyIdFromOrigin(origin: string | null): string | null {
  return origin?.startsWith("spotify:playlist:") ? origin.slice("spotify:playlist:".length) : null;
}

/** Re-render periodically so relative times stay fresh. */
function useTick(ms: number) {
  const [, set] = useState(0);
  useEffect(() => {
    const t = window.setInterval(() => set((n) => n + 1), ms);
    return () => window.clearInterval(t);
  }, [ms]);
}

function SyncStatus({ playlist, syncing }: { playlist: Playlist; syncing: boolean }) {
  useTick(30_000);
  const s = playlist.sync;
  if (!s) return null;
  if (syncing || s.status === "syncing") return <span className="text-muted-foreground">· Syncing with Spotify…</span>;
  return (
    <>
      {s.lastSyncedAt && <span title={new Date(s.lastSyncedAt).toLocaleString()}>· Synced {relativeTime(s.lastSyncedAt)}</span>}
      {s.status === "pending" && <span>· Changes waiting to sync</span>}
      {s.status === "error" && s.lastError && (
        <span className="inline-flex items-center gap-1 text-destructive" title={s.lastError}>
          <AlertCircle className="size-3.5" /> Last sync failed: {s.lastError}
        </span>
      )}
    </>
  );
}

export function PlaylistView({ id }: { id: string }) {
  const { playlists, refreshPlaylists, navigate, libraryVersion, syncing: syncingMap } = useUi();
  const providers = usePlayer((s) => s.providers);
  const playTracks = usePlayer((s) => s.playTracks);
  const [rev, setRev] = useState(0);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [sort, setSort] = useState<PlaylistTrackSort>("custom");
  const [manualSync, setManualSync] = useState(false);
  const [linking, setLinking] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [altFor, setAltFor] = useState<{ track: Track; position: number } | null>(null);
  const playlist = playlists.find((p) => p.id === id);
  const { data: tracks } = useAsync(() => api.playlistTracks(id), [id, rev, libraryVersion]);
  const { data: relations } = useAsync(() => api.alternativesList(id), [id, rev, libraryVersion]);

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

  const synced = playlist.sync;
  const syncing = manualSync || !!syncingMap[id] || synced?.status === "syncing";
  const detailsLocked = !!synced && !synced.canEditDetails;
  const lockedWhy = "Collaborative Spotify playlist owned by someone else: only the owner can change this";
  const spotifyConnected = !!providers.spotify?.connected;
  const linkableSpotifyId = !synced ? spotifyIdFromOrigin(playlist.origin) : null;
  // Local-only playlists (incl. Branflakes-only copies) can be linked on explicit request.
  const canCreateOnSpotify = !synced && !linkableSpotifyId && spotifyConnected;
  const activeAlts = (relations ?? []).filter((r) => !r.removed && existing.has(r.alternativeTrackId));
  const altsByOriginal = new Map<string, Set<string>>();
  for (const r of activeAlts) altsByOriginal.set(r.originalTrackId, (altsByOriginal.get(r.originalTrackId) ?? new Set()).add(r.alternativeTrackId));
  const originalOfAlt = new Map(activeAlts.map((r) => [r.alternativeTrackId, r.originalTrackId]));
  const titleOf = (trackId: string) => (tracks ?? []).find((t) => t.id === trackId)?.title ?? "the original";
  const needsReconnect = !!providers.spotify?.missingScopes?.some((s) => s.startsWith("playlist-modify"));

  const syncNow = async () => {
    if (syncing) return; // one request at a time (the backend also refuses duplicates)
    setManualSync(true);
    try {
      const r = await api.spotifySyncNow(id);
      const parts = [
        r.pulledAdded && `${r.pulledAdded} added from Spotify`,
        r.pulledRemoved && `${r.pulledRemoved} removed on Spotify`,
        r.pushedOps && "your changes sent to Spotify",
        r.renamed && "renamed",
        r.artworkChanged && "cover updated",
      ].filter(Boolean);
      toast.success("Synced with Spotify", { description: [parts.join(", ") || "Already up to date", ...r.notices].join(" ") });
      setRev((n) => n + 1);
    } catch (e) {
      toast.error("Spotify sync failed", { description: toApiError(e).message });
    } finally {
      setManualSync(false);
      await refreshPlaylists();
    }
  };

  const enableSync = async () => {
    if (!linkableSpotifyId) return;
    if (!confirm(`Keep “${playlist.name}” in sync with Spotify?\n\nBranflakes will first update this playlist to match Spotify. After that, changes in either app are synced automatically.`)) return;
    setLinking(true);
    try {
      await api.spotifySyncEnable(linkableSpotifyId);
      toast.success("Spotify sync turned on", { description: playlist.name });
      setRev((n) => n + 1);
      await refreshPlaylists();
    } catch (e) {
      toast.error("Can't sync this playlist", { description: toApiError(e).message, duration: 10000 });
    } finally {
      setLinking(false);
    }
  };

  const linkToSpotify = async () => {
    if (
      !confirm(
        `Create “${playlist.name}” on Spotify and keep it in sync?\n\nThis creates a new private playlist in your Spotify account with the same name and tracks. After that, changes in either app are synced automatically.`,
      )
    )
      return;
    setLinking(true);
    try {
      await api.linkPlaylistToSpotify(id);
      toast.success("Created on Spotify and synced", { description: playlist.name });
      setRev((n) => n + 1);
    } catch (e) {
      toast.error("Couldn't sync this playlist with Spotify", {
        description: `${toApiError(e).message}. The playlist stays in Branflakes; you can try again.`,
        duration: 12000,
      });
    } finally {
      setLinking(false);
      await refreshPlaylists();
    }
  };

  const rowAccessory = (t: Track, i: number) => {
    const altOf = originalOfAlt.get(t.id);
    if (altOf) {
      const where = synced ? " It exists only in Branflakes, not in the Spotify playlist." : "";
      return (
        <span className="shrink-0 rounded bg-muted px-1 text-[10px] font-normal text-muted-foreground" title={`Alternative for “${titleOf(altOf)}”.${where}`}>
          ALTERNATIVE{synced ? " · BRANFLAKES ONLY" : ""}
        </span>
      );
    }
    if (!canFindAlternative(t)) return null;
    const chosen = altsByOriginal.get(t.id)?.size ?? 0;
    return (
      <>
        <Button
          variant="ghost"
          size="icon"
          className="size-6 shrink-0 text-muted-foreground hover:text-foreground"
          aria-label="Find alternative"
          title="Find alternative"
          onClick={(e) => {
            e.stopPropagation();
            setAltFor({ track: t, position: rows[i]?.position ?? i });
          }}
        >
          <ScanSearch className="size-4" />
        </Button>
        {chosen > 0 && <span className="shrink-0 text-[10px] font-normal text-muted-foreground">Alternative added</span>}
      </>
    );
  };

  const disableSync = () => {
    if (!confirm(`Stop syncing “${playlist.name}” with Spotify?\n\nBoth playlists stay as they are; changes will no longer be copied between them.`)) return;
    void act(() => api.spotifySyncDisable(id));
  };

  const kindLabel = playlistKindLabel(playlist);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-end gap-3 px-4 pb-4 pt-5 @xl:gap-5 @xl:px-6">
        <DropdownMenu>
          <DropdownMenuTrigger asChild disabled={detailsLocked}>
            <button className="group relative shrink-0 rounded-md focus-visible:outline-2" aria-label="Change playlist cover" title={detailsLocked ? lockedWhy : undefined}>
              <PlaylistCover path={playlist.artworkPath} className="size-20 shadow-md @xl:size-36" alt={`${playlist.name} cover`} />
              {!detailsLocked && (
                <span className="absolute inset-0 grid place-items-center rounded-md bg-black/50 text-xs font-medium text-white opacity-0 transition-opacity group-hover:opacity-100">
                  <span className="flex flex-col items-center gap-1">
                    <ImagePlus className="size-5" /> Change cover
                  </span>
                </span>
              )}
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            <DropdownMenuItem onSelect={() => void chooseCover()}>Choose image…</DropdownMenuItem>
            {playlist.artworkPath && (
              <DropdownMenuItem variant="destructive" onSelect={() => void act(() => api.setPlaylistArtwork(id, null))}>
                Remove cover{synced ? " (Spotify keeps its cover)" : ""}
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>

        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-1.5 text-xs uppercase tracking-wide text-muted-foreground">
            <PlaylistKindIcon playlist={playlist} />
            {kindLabel ?? `Playlist${playlist.origin ? ` · imported from ${playlist.origin.startsWith("spotify") ? "Spotify" : "SoundCloud"}` : ""}`}
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
            <h1 className="truncate text-2xl font-bold tracking-tight @xl:text-3xl" title={playlist.name}>
              {playlist.name}
            </h1>
          )}
          <p className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            {playlist.trackCount} {playlist.trackCount === 1 ? "track" : "tracks"}
            {totalMs > 0 && <span>· {formatDuration(totalMs)}</span>}
            {sources.length > 0 && <SourceBadges sources={sources} />}
            {unavailable > 0 && <span className="text-destructive">{unavailable} unavailable (will be skipped)</span>}
            {playlist.lastPlayedAt && <span>· last played {new Date(playlist.lastPlayedAt).toLocaleDateString()}</span>}
            <SyncStatus playlist={playlist} syncing={syncing} />
          </p>
          {synced?.lastNotice && synced.status !== "error" && <p className="mt-1 max-w-3xl text-xs text-muted-foreground">{synced.lastNotice}</p>}
          {synced && needsReconnect && (
            <p className="mt-1 flex max-w-3xl items-center gap-2 text-xs text-destructive">
              <AlertCircle className="size-3.5 shrink-0" />
              Your Spotify sign-in doesn't allow changing playlists yet, so changes made here can't be sent to Spotify.
              <button className="underline underline-offset-2" onClick={() => navigate({ name: "settings" })}>
                Reconnect Spotify
              </button>
            </p>
          )}
          {playlist.forkNote && (
            <p className={cn("mt-1 max-w-3xl text-xs", /not complete|not included/.test(playlist.forkNote) ? "text-amber-600 dark:text-amber-400" : "text-muted-foreground")}>
              {playlist.forkNote}
            </p>
          )}
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
            {synced && (
              <Button
                size="sm"
                variant="ghost"
                aria-label="Sync Spotify Playlist"
                title={spotifyConnected ? "Sync Spotify Playlist" : "Connect Spotify in Settings to sync"}
                disabled={syncing || !spotifyConnected}
                aria-busy={syncing}
                onClick={() => void syncNow()}
              >
                <RefreshCw className={cn("size-4", syncing && "animate-spin")} />
              </Button>
            )}
            {canCreateOnSpotify && (
              <Button size="sm" variant="ghost" disabled={linking} onClick={() => void linkToSpotify()} title="Create this playlist on Spotify and keep it in sync">
                {linking ? <RefreshCw className="size-4 animate-spin" /> : <Link2 className="size-4" />} Sync with Spotify…
              </Button>
            )}
            {linkableSpotifyId && spotifyConnected && (
              <Button size="sm" variant="ghost" disabled={linking} onClick={() => void enableSync()} title="Keep this playlist in sync with Spotify">
                {linking ? <RefreshCw className="size-4 animate-spin" /> : <Link2 className="size-4" />} Sync with Spotify
              </Button>
            )}
            <Button
              size="sm"
              variant="ghost"
              aria-label="Share playlist"
              title="Save as a .bran file to share (includes local songs and the cover)"
              onClick={() => void exportPlaylistAsBran(playlist)}
            >
              <Share2 className="size-4" />
            </Button>
            <Button
              size="sm"
              variant="ghost"
              aria-label="Rename playlist"
              disabled={detailsLocked}
              title={detailsLocked ? lockedWhy : "Rename playlist"}
              onClick={() => setRenaming(playlist.name)}
            >
              <Pencil className="size-4" />
            </Button>
            {synced && (
              <Button size="sm" variant="ghost" aria-label="Stop syncing with Spotify" title="Stop syncing with Spotify" onClick={disableSync}>
                <Unlink className="size-4" />
              </Button>
            )}
            <Button
              size="sm"
              variant="ghost"
              aria-label="Delete playlist"
              onClick={() => {
                if (synced) {
                  setDeleting(true); // explicit Spotify confirmation dialog
                  return;
                }
                const extra = playlist.forkedFrom ? " The original Spotify playlist is not affected." : " Nothing on Spotify is changed.";
                if (confirm(`Delete playlist “${playlist.name}”? Tracks stay in your library.${extra}`)) {
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
        rowAccessory={rowAccessory}
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
                Remove from playlist{synced ? " (also on Spotify)" : ""}
              </DropdownMenuItem>
            </>
          );
        }}
      />

      {altFor && (
        <AlternativesDialog
          open={!!altFor}
          onOpenChange={(o) => !o && setAltFor(null)}
          playlistId={id}
          original={altFor.track}
          position={sort === "custom" ? altFor.position : undefined}
          synced={!!synced}
          alreadyAdded={altsByOriginal.get(altFor.track.id) ?? new Set()}
          onAdded={() => {
            setRev((r) => r + 1);
            void refreshPlaylists();
          }}
        />
      )}

      {synced && (
        <DeleteSyncedPlaylistDialog
          playlist={playlist}
          open={deleting}
          onOpenChange={setDeleting}
          onDeleted={(msg) => {
            toast.success(msg ?? "Playlist deleted");
            void refreshPlaylists().then(() => navigate({ name: "library" }));
          }}
        />
      )}

      <AddSongsDialog
        open={adding}
        onOpenChange={setAdding}
        playlistId={id}
        playlistName={playlist.name}
        existing={existing}
        note={
          synced
            ? "This playlist is Spotify Synced: Spotify tracks are added on Spotify too. Adding a local or SoundCloud track creates a Branflakes-only copy instead."
            : undefined
        }
        onAdded={() => {
          setRev((r) => r + 1);
          void refreshPlaylists();
        }}
        onForked={(copy) => {
          setAdding(false);
          void refreshPlaylists().then(() => navigate({ name: "playlist", id: copy.id }));
        }}
      />
    </div>
  );
}
