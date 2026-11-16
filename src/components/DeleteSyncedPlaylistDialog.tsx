import { useState } from "react";
import { AlertTriangle, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { api, toApiError } from "@/lib/api";
import type { Playlist } from "@/lib/types";

/**
 * Deleting a Spotify Synced playlist affects the real Spotify playlist, so
 * it needs an explicit choice. Nothing happens until a button is pressed;
 * Cancel (or Esc) leaves both playlists untouched.
 */
export function DeleteSyncedPlaylistDialog({
  playlist,
  open,
  onOpenChange,
  onDeleted,
}: {
  playlist: Playlist;
  open: boolean;
  onOpenChange(open: boolean): void;
  onDeleted(message: string | null): void;
}) {
  const [busy, setBusy] = useState<"spotify" | "local" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const owned = !!playlist.sync?.canEditDetails;

  const run = async (mode: "spotify" | "local") => {
    if (busy) return;
    setBusy(mode);
    setError(null);
    try {
      const msg = await api.deletePlaylist(playlist.id, mode === "spotify" ? { confirmSpotify: true } : { keepOnSpotify: true });
      onDeleted(msg);
      onOpenChange(false);
    } catch (e) {
      // Nothing was removed locally: the user can retry or cancel.
      setError(toApiError(e).message);
    } finally {
      setBusy(null);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !busy && onOpenChange(o)}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <AlertTriangle className="size-5 text-destructive" /> Delete “{playlist.name}” from Spotify?
          </DialogTitle>
          <DialogDescription asChild>
            <div className="space-y-2 text-sm">
              {owned ? (
                <p>
                  This playlist is <b>synced with your Spotify account</b>. Deleting it here also <b>deletes the actual playlist from your Spotify account</b>, not just the
                  Branflakes copy.
                </p>
              ) : (
                <p>
                  This playlist belongs to <b>another Spotify user</b>. Branflakes will <b>remove it from your Spotify library</b> (unfollow it). The owner's playlist is not
                  changed.
                </p>
              )}
              <p className="text-muted-foreground">
                Spotify doesn't let apps erase playlists permanently: like Spotify's own Delete, it's removed from your library. Your tracks stay in your Branflakes library.
              </p>
            </div>
          </DialogDescription>
        </DialogHeader>
        {error && (
          <p role="alert" className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            Nothing was deleted: {error}
          </p>
        )}
        <div className="flex flex-wrap justify-end gap-2 pt-2">
          <Button variant="ghost" autoFocus disabled={!!busy} onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant="secondary" disabled={!!busy} onClick={() => void run("local")}>
            {busy === "local" && <Loader2 className="size-4 animate-spin" />} Keep on Spotify, delete here only
          </Button>
          <Button variant="destructive" disabled={!!busy} onClick={() => void run("spotify")}>
            {busy === "spotify" && <Loader2 className="size-4 animate-spin" />}
            {owned ? "Delete from Spotify too" : "Remove from my Spotify library"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
