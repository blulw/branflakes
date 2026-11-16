import { useState } from "react";
import { Loader2, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { SpotifyGlyph } from "@/components/PlaylistKind";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api, toApiError } from "@/lib/api";
import { useUi } from "@/state/ui";
import type { SpotifyPlaylistOption } from "@/lib/types";

/** Playlist sync controls inside Settings → Accounts → Spotify. */
export function SpotifySyncSettings({ intervalMinutes, needsReconnect }: { intervalMinutes: number; needsReconnect: boolean }) {
  const { refreshSettings, refreshPlaylists, navigate } = useUi();
  const [minutes, setMinutes] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [picker, setPicker] = useState(false);
  const [options, setOptions] = useState<SpotifyPlaylistOption[] | null>(null);

  const saveInterval = async () => {
    try {
      await api.updateSettings({ spotifySyncIntervalMinutes: Number(minutes) });
      setMinutes("");
      await refreshSettings();
      toast.success("Saved");
    } catch (e) {
      toast.error(toApiError(e).message);
    }
  };

  const syncAll = async () => {
    setBusy("all");
    try {
      const r = await api.spotifySyncAll();
      if (r.failed.length) toast.error(`Synced ${r.synced}; ${r.failed.length} failed`, { description: r.failed.join("\n"), duration: 10000 });
      else toast.success(r.synced ? `Synced ${r.synced} playlist${r.synced === 1 ? "" : "s"}` : "No Spotify Synced playlists yet");
    } catch (e) {
      toast.error(toApiError(e).message);
    } finally {
      setBusy(null);
      await refreshPlaylists();
    }
  };

  const openPicker = async () => {
    setPicker(true);
    setOptions(null);
    try {
      setOptions(await api.spotifySyncCandidates());
    } catch (e) {
      toast.error(toApiError(e).message);
      setPicker(false);
    }
  };

  const enable = async (o: SpotifyPlaylistOption) => {
    setBusy(o.id);
    try {
      const p = await api.spotifySyncEnable(o.id);
      setOptions((list) => list?.map((x) => (x.id === o.id ? { ...x, syncedPlaylistId: p.id } : x)) ?? null);
      await refreshPlaylists();
      toast.success("Spotify sync turned on", { description: p.name });
    } catch (e) {
      toast.error(`Can't sync “${o.name}”`, { description: toApiError(e).message, duration: 10000 });
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="space-y-2 rounded-md border p-3">
      <p className="flex items-center gap-1.5 text-sm font-medium">
        <SpotifyGlyph className="size-4 text-source-spotify" /> Playlist sync
      </p>
      <p className="text-xs text-muted-foreground">
        Spotify Synced playlists stay the same in Branflakes and Spotify: tracks, order, name and cover. Only playlists you own or collaborate on can be synced.
        Adding a local or SoundCloud track to one creates a Branflakes-only copy instead.
      </p>
      {needsReconnect && <p className="text-xs text-destructive">Reconnect Spotify above to allow playlist changes and cover uploads.</p>}
      <div className="flex flex-wrap items-end gap-2">
        <div className="grid gap-1">
          <Label htmlFor="sp-sync-int" className="text-xs">
            Sync automatically every (minutes)
          </Label>
          <Input id="sp-sync-int" className="h-8 w-24" inputMode="numeric" placeholder={String(intervalMinutes)} value={minutes} onChange={(e) => setMinutes(e.target.value.replace(/\D/g, ""))} />
        </div>
        <Button size="sm" variant="secondary" disabled={!minutes} onClick={() => void saveInterval()}>
          Save
        </Button>
        <Button size="sm" variant="secondary" onClick={() => void openPicker()}>
          Choose playlists to sync…
        </Button>
        <Button size="sm" variant="ghost" disabled={busy === "all"} onClick={() => void syncAll()} title="Sync all Spotify Synced playlists now">
          <RefreshCw className={busy === "all" ? "size-4 animate-spin" : "size-4"} /> Sync all now
        </Button>
      </div>

      <Dialog open={picker} onOpenChange={setPicker}>
        <DialogContent className="flex max-h-[80vh] max-w-xl flex-col">
          <DialogHeader>
            <DialogTitle>Sync Spotify playlists</DialogTitle>
            <DialogDescription>Turning sync on imports the playlist (or updates your imported copy) to match Spotify, then keeps both in step.</DialogDescription>
          </DialogHeader>
          <ul className="min-h-40 flex-1 overflow-y-auto">
            {options == null && <Loader2 className="m-4 size-5 animate-spin" />}
            {options?.length === 0 && <p className="p-4 text-sm text-muted-foreground">No playlists found on your Spotify account.</p>}
            {options?.map((o) => (
              <li key={o.id} className="flex items-center gap-3 rounded-md px-2 py-1.5 hover:bg-accent/60">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{o.name}</div>
                  <div className="truncate text-xs text-muted-foreground">
                    {o.total != null ? `${o.total} tracks · ` : ""}
                    {o.owned ? "Yours" : o.collaborative ? "Collaborative (tracks only: the owner controls name and cover)" : o.reason}
                  </div>
                </div>
                {o.syncedPlaylistId ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => {
                      setPicker(false);
                      navigate({ name: "playlist", id: o.syncedPlaylistId! });
                    }}
                  >
                    <SpotifyGlyph className="size-3.5 text-source-spotify" /> Synced
                  </Button>
                ) : (
                  <Button size="sm" variant="secondary" disabled={!o.canSync || busy != null} title={o.reason ?? undefined} onClick={() => void enable(o)}>
                    {busy === o.id && <Loader2 className="size-4 animate-spin" />} Sync
                  </Button>
                )}
              </li>
            ))}
          </ul>
        </DialogContent>
      </Dialog>
    </div>
  );
}
