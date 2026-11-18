import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Loader2, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { Artwork } from "@/components/Artwork";
import { SourceBadge } from "@/components/SourceBadge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { api, toApiError } from "@/lib/api";
import { usePlayer } from "@/playback/store";
import { useUi } from "@/state/ui";
import type { FileMetadata, MetadataBackup, MetadataEdit, Track, TrackOverride } from "@/lib/types";

const TEXT_FIELDS = [
  ["title", "Title"],
  ["artist", "Artist"],
  ["album", "Album"],
  ["albumArtist", "Album artist"],
  ["genre", "Genre"],
] as const;

export function TrackEditorDialog() {
  const { editing, edit, bumpLibrary } = useUi();
  const close = () => edit(null);
  return (
    <Dialog open={!!editing} onOpenChange={(o) => !o && close()}>
      <DialogContent className="max-w-2xl">
        {editing && (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                {editing.title} <SourceBadge source={editing.source} compact />
              </DialogTitle>
              <DialogDescription>
                {editing.source === "local"
                  ? "Edit tags inside the audio file, or set app-only overrides that never touch the file."
                  : "Streaming tracks can't be changed at the provider. Overrides change how this track is shown in this app only."}
              </DialogDescription>
            </DialogHeader>
            <Tabs defaultValue={editing.source === "local" ? "file" : "override"}>
              <TabsList>
                {editing.source === "local" && <TabsTrigger value="file">File tags</TabsTrigger>}
                <TabsTrigger value="override">App overrides</TabsTrigger>
              </TabsList>
              {editing.source === "local" && (
                <TabsContent value="file">
                  <FileTagEditor track={editing} onSaved={() => (bumpLibrary(), close())} />
                </TabsContent>
              )}
              <TabsContent value="override">
                <OverrideEditor track={editing} onSaved={() => (bumpLibrary(), close())} />
              </TabsContent>
            </Tabs>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

function FileTagEditor({ track, onSaved }: { track: Track; onSaved: () => void }) {
  const [md, setMd] = useState<FileMetadata | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [form, setForm] = useState<Record<string, string>>({});
  const [art, setArt] = useState<{ action: "replace"; path: string } | { action: "remove" } | null>(null);
  const [busy, setBusy] = useState(false);
  const [backups, setBackups] = useState<MetadataBackup[]>([]);
  const isLoaded = usePlayer((s) => s.loadedTrackId === track.id && s.status === "playing");

  useEffect(() => {
    api.readMetadata(track.id).then(setMd).catch((e) => setError(toApiError(e).message));
    api.listBackups(track.id).then(setBackups).catch(() => undefined);
  }, [track.id]);

  if (error) return <p className="py-4 text-sm text-destructive">{error}</p>;
  if (!md) return <Loader2 className="m-6 size-5 animate-spin" />;

  const current: Record<string, string> = {
    title: md.title,
    artist: md.artist,
    album: md.album,
    albumArtist: md.albumArtist,
    genre: md.genre,
    year: md.year?.toString() ?? "",
    trackNumber: md.trackNumber?.toString() ?? "",
    discNumber: md.discNumber?.toString() ?? "",
  };
  const changed = Object.entries(form).filter(([k, v]) => v !== current[k]);

  const save = async () => {
    const edit: MetadataEdit = Object.fromEntries(changed);
    if (art) edit.artwork = art;
    setBusy(true);
    try {
      const r = await api.writeMetadata(track.id, edit);
      toast.success("Tags written and verified", { description: `Backup saved: ${r.backupPath}` });
      onSaved();
    } catch (e) {
      toast.error("Write failed", { description: toApiError(e).message, duration: 10000 });
    } finally {
      setBusy(false);
    }
  };

  const pickArt = async () => {
    const p = await open({ multiple: false, filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png"] }] });
    if (typeof p === "string") setArt({ action: "replace", path: p });
  };

  if (!md.writable) {
    return <p className="py-4 text-sm text-muted-foreground">{md.writeUnsupportedReason ?? "This format cannot be edited."} You can still use app overrides.</p>;
  }

  return (
    <div className="space-y-4 pt-2">
      <div className="grid grid-cols-2 gap-3">
        {TEXT_FIELDS.map(([k, label]) => (
          <div key={k} className="grid gap-1">
            <Label htmlFor={`f-${k}`}>{label}</Label>
            <Input id={`f-${k}`} value={form[k] ?? current[k]} onChange={(e) => setForm({ ...form, [k]: e.target.value })} />
          </div>
        ))}
        <div className="grid grid-cols-3 gap-2">
          {(
            [
              ["year", "Year"],
              ["trackNumber", "Track"],
              ["discNumber", "Disc"],
            ] as const
          ).map(([k, label]) => (
            <div key={k} className="grid gap-1">
              <Label htmlFor={`f-${k}`}>{label}</Label>
              <Input id={`f-${k}`} inputMode="numeric" value={form[k] ?? current[k]} onChange={(e) => setForm({ ...form, [k]: e.target.value.replace(/\D/g, "").slice(0, 4) })} />
            </div>
          ))}
        </div>
      </div>
      <div className="flex items-center gap-3">
        <Artwork src={track.artwork} className="size-16" />
        <div className="space-y-1 text-xs text-muted-foreground">
          <div>
            Embedded cover:{" "}
            {art?.action === "replace" ? `will be replaced with ${art.path}` : art?.action === "remove" ? "will be removed" : md.hasArtwork ? "present" : "none"}
          </div>
          <div className="flex gap-2">
            <Button size="sm" variant="secondary" onClick={() => void pickArt()}>
              Replace cover…
            </Button>
            {md.hasArtwork && (
              <Button size="sm" variant="ghost" onClick={() => setArt({ action: "remove" })}>
                Remove cover
              </Button>
            )}
          </div>
        </div>
      </div>
      <p className="flex gap-2 rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
        <ShieldCheck className="size-4 shrink-0 text-primary" />
        {md.format.toUpperCase()} · {md.tagType ?? "no tag yet"}. The file is backed up first, only changed fields are written, other tags are preserved, and the result is read back and verified. On any mismatch the original is restored.
        {isLoaded && " This track is playing. If Windows reports the file is in use, pause it and try again."}
      </p>
      {backups.length > 0 && (
        <details className="text-xs text-muted-foreground">
          <summary className="cursor-pointer">Backups ({backups.length})</summary>
          <ul className="mt-1 space-y-1">
            {backups.map((b) => (
              <li key={b.id} className="flex items-center gap-2">
                <span className="flex-1 truncate" data-selectable>
                  {new Date(b.createdAt).toLocaleString()} · {b.backupPath}
                </span>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    if (confirm("Restore this backup over the current file?"))
                      void api.restoreBackup(b.id).then(
                        () => (toast.success("Backup restored"), onSaved()),
                        (e) => toast.error(toApiError(e).message),
                      );
                  }}
                >
                  Restore
                </Button>
              </li>
            ))}
          </ul>
        </details>
      )}
      <div className="flex justify-end">
        <Button disabled={busy || (!changed.length && !art)} onClick={() => void save()}>
          {busy && <Loader2 className="size-4 animate-spin" />} Write to file
        </Button>
      </div>
    </div>
  );
}

function OverrideEditor({ track, onSaved }: { track: Track; onSaved: () => void }) {
  const [o, setO] = useState<TrackOverride>({});
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.getOverride(track.id).then((v) => {
      setO(v ?? {});
      setLoaded(true);
    });
  }, [track.id]);

  if (!loaded) return <Loader2 className="m-6 size-5 animate-spin" />;

  const run = async (fn: () => Promise<unknown>, msg: string) => {
    setBusy(true);
    try {
      await fn();
      toast.success(msg);
      onSaved();
    } catch (e) {
      toast.error(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  };

  const pickArt = async () => {
    const p = await open({ multiple: false, filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp"] }] });
    if (typeof p === "string") await run(() => api.setOverrideArtwork(track.id, p), "Custom artwork set");
  };

  return (
    <div className="space-y-4 pt-2">
      <p className="text-xs text-muted-foreground">Leave a field blank to use the provider's value. Overrides are stored in this app's database only.</p>
      <div className="grid grid-cols-2 gap-3">
        {TEXT_FIELDS.map(([k, label]) => (
          <div key={k} className="grid gap-1">
            <Label htmlFor={`o-${k}`}>{label}</Label>
            <Input id={`o-${k}`} placeholder={String(track[k] ?? "")} value={(o[k] as string | null | undefined) ?? ""} onChange={(e) => setO({ ...o, [k]: e.target.value })} />
          </div>
        ))}
        <div className="grid grid-cols-2 gap-2">
          <div className="grid gap-1">
            <Label htmlFor="o-year">Year</Label>
            <Input id="o-year" inputMode="numeric" value={o.year ?? ""} onChange={(e) => setO({ ...o, year: e.target.value ? Number(e.target.value.replace(/\D/g, "").slice(0, 4)) : null })} />
          </div>
          <div className="grid gap-1">
            <Label htmlFor="o-track">Track</Label>
            <Input id="o-track" inputMode="numeric" value={o.trackNumber ?? ""} onChange={(e) => setO({ ...o, trackNumber: e.target.value ? Number(e.target.value.replace(/\D/g, "").slice(0, 4)) : null })} />
          </div>
        </div>
      </div>
      <div className="flex items-center gap-3">
        <Artwork src={track.artwork} className="size-16" />
        <Button size="sm" variant="secondary" disabled={busy} onClick={() => void pickArt()}>
          Custom artwork…
        </Button>
      </div>
      <div className="flex justify-end gap-2">
        {track.hasOverrides && (
          <Button variant="ghost" disabled={busy} onClick={() => void run(() => api.clearOverride(track.id), "Overrides cleared")}>
            Reset to provider metadata
          </Button>
        )}
        <Button disabled={busy} onClick={() => void run(() => api.setOverride(track.id, o), "Overrides saved")}>
          Save overrides
        </Button>
      </div>
    </div>
  );
}
