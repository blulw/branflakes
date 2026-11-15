import { useDeferredValue, useEffect, useState } from "react";
import { Check, Loader2, Plus, Search } from "lucide-react";
import { toast } from "sonner";
import { Artwork } from "@/components/Artwork";
import { SourceBadge } from "@/components/SourceBadge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { api, toApiError } from "@/lib/api";
import { formatDuration } from "@/lib/format";
import { usePlayer } from "@/playback/store";
import type { Track } from "@/lib/types";

interface Props {
  open: boolean;
  onOpenChange(open: boolean): void;
  playlistId: string;
  playlistName: string;
  /** Track IDs already in the playlist (shown as "In playlist"). */
  existing: Set<string>;
  onAdded(): void;
}

export function AddSongsDialog({ open, onOpenChange, playlistId, playlistName, existing, onAdded }: Props) {
  const [added, setAdded] = useState<Set<string>>(new Set());
  useEffect(() => {
    if (open) setAdded(new Set());
  }, [open]);

  const add = async (t: Track) => {
    try {
      await api.addToPlaylist(playlistId, [t.id]);
      setAdded((s) => new Set(s).add(t.id));
      onAdded();
    } catch (e) {
      toast.error(toApiError(e).message);
    }
  };

  const row = (t: Track) => {
    const isAdded = added.has(t.id);
    const inPlaylist = existing.has(t.id);
    return (
      <li key={t.id} className="flex items-center gap-3 rounded-md px-2 py-1.5 hover:bg-accent/60" title={t.availability === "unavailable" ? t.unavailableReason ?? "" : undefined}>
        <Artwork src={t.artwork} className="size-9 shrink-0" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium">{t.title}</div>
          <div className="truncate text-xs text-muted-foreground">
            {t.artist}
            {t.album ? ` — ${t.album}` : ""}
          </div>
        </div>
        <SourceBadge source={t.source} compact />
        <span className="w-10 text-right text-xs tabular-nums text-muted-foreground">{formatDuration(t.durationMs)}</span>
        {inPlaylist && !isAdded && <span className="text-[10px] uppercase text-muted-foreground">In playlist</span>}
        <Button size="sm" variant={isAdded ? "ghost" : "secondary"} disabled={isAdded} onClick={() => void add(t)} aria-label={`Add ${t.title}`} className="w-20">
          {isAdded ? (
            <>
              <Check className="size-4" /> Added
            </>
          ) : (
            <>
              <Plus className="size-4" /> Add
            </>
          )}
        </Button>
      </li>
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[85vh] max-w-2xl flex-col">
        <DialogHeader>
          <DialogTitle>Add songs to “{playlistName}”</DialogTitle>
          <DialogDescription>Add from your library, or search Spotify and SoundCloud. Playlists can mix sources.</DialogDescription>
        </DialogHeader>
        <Tabs defaultValue="library" className="flex min-h-0 flex-1 flex-col">
          <TabsList>
            <TabsTrigger value="library">Your library</TabsTrigger>
            <TabsTrigger value="streaming">Spotify & SoundCloud</TabsTrigger>
          </TabsList>
          <TabsContent value="library" className="flex min-h-0 flex-1 flex-col">
            <LibraryPicker row={row} />
          </TabsContent>
          <TabsContent value="streaming" className="flex min-h-0 flex-1 flex-col">
            <StreamingPicker row={row} />
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}

function LibraryPicker({ row }: { row: (t: Track) => React.ReactNode }) {
  const [q, setQ] = useState("");
  const dq = useDeferredValue(q);
  const [tracks, setTracks] = useState<Track[] | null>(null);
  useEffect(() => {
    let live = true;
    api
      .listTracks({ search: dq || undefined, limit: 300, sort: dq ? "title" : "added", descending: !dq })
      .then((t) => live && setTracks(t))
      .catch((e) => toast.error(toApiError(e).message));
    return () => {
      live = false;
    };
  }, [dq]);
  return (
    <>
      <div className="relative my-2">
        <Search className="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input autoFocus value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search your library" className="pl-8" aria-label="Search your library" />
      </div>
      <ul className="min-h-64 flex-1 overflow-y-auto">
        {tracks == null && <Loader2 className="m-4 size-5 animate-spin" />}
        {tracks?.map(row)}
        {tracks?.length === 0 && <p className="p-4 text-sm text-muted-foreground">{dq ? "No matching tracks in your library." : "Your library is empty."}</p>}
      </ul>
      {!dq && tracks && tracks.length > 0 && <p className="pt-2 text-xs text-muted-foreground">Showing recently added tracks. Type to search everything.</p>}
    </>
  );
}

function StreamingPicker({ row }: { row: (t: Track) => React.ReactNode }) {
  const providers = usePlayer((s) => s.providers);
  const [q, setQ] = useState("");
  const [busy, setBusy] = useState(false);
  const [results, setResults] = useState<Track[] | null>(null);
  const available = !!providers.spotify?.connected || !!providers.soundcloud?.capabilities.play;

  const run = async () => {
    if (!q.trim()) return;
    setBusy(true);
    try {
      const r = await api.searchProviders(q);
      r.errors.forEach((e) => toast.error(`${e.provider}: ${e.error.message}`));
      setResults([...r.spotify, ...r.soundcloud]);
    } catch (e) {
      toast.error(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  };

  if (!available) {
    return <p className="p-4 text-sm text-muted-foreground">Connect Spotify, or connect SoundCloud / turn on SoundCloud web mode in Settings, to search streaming services.</p>;
  }
  return (
    <>
      <form
        className="my-2 flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void run();
        }}
      >
        <Input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search Spotify and SoundCloud" aria-label="Search streaming services" />
        <Button type="submit" disabled={busy || !q.trim()}>
          {busy ? <Loader2 className="size-4 animate-spin" /> : "Search"}
        </Button>
      </form>
      <ul className="min-h-64 flex-1 overflow-y-auto">
        {results?.map(row)}
        {results?.length === 0 && <p className="p-4 text-sm text-muted-foreground">No results.</p>}
      </ul>
    </>
  );
}
