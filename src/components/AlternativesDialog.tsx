import { useCallback, useEffect, useRef, useState } from "react";
import { Check, Loader2, Plus, RotateCw } from "lucide-react";
import { toast } from "sonner";
import { Artwork } from "@/components/Artwork";
import { SourceBadge } from "@/components/SourceBadge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { api, toApiError } from "@/lib/api";
import { formatDuration } from "@/lib/format";
import type { AlternativesResult, Track } from "@/lib/types";

type Phase = { state: "idle" } | { state: "loading" } | { state: "done"; result: AlternativesResult } | { state: "failed"; message: string };

interface Props {
  open: boolean;
  onOpenChange(open: boolean): void;
  playlistId: string;
  /** The unavailable Spotify track. */
  original: Track;
  /** Its position in the playlist (custom order), so the alternative lands right after it. */
  position?: number;
  /** Spotify Synced playlist: alternatives exist only in Branflakes. */
  synced: boolean;
  /** Track ids already chosen as alternatives for this original (not removed). */
  alreadyAdded: Set<string>;
  onAdded(track: Track): void;
}

/**
 * "Find alternative": searches the local library first, then SoundCloud,
 * and lets the user add a match right after the unavailable track.
 */
export function AlternativesDialog({ open, onOpenChange, playlistId, original, position, synced, alreadyAdded, onAdded }: Props) {
  const [local, setLocal] = useState<Phase>({ state: "idle" });
  const [soundcloud, setSoundcloud] = useState<Phase>({ state: "idle" });
  const [adding, setAdding] = useState<string | null>(null);
  const [added, setAdded] = useState<Set<string>>(new Set());
  const run = useRef(0);

  const search = useCallback(async () => {
    const id = ++run.current;
    setLocal({ state: "loading" });
    setSoundcloud({ state: "loading" });
    // Local library first…
    try {
      const r = await api.alternativesSearch(original.id, "local");
      if (run.current === id) setLocal({ state: "done", result: r });
    } catch (e) {
      if (run.current === id) setLocal({ state: "failed", message: toApiError(e).message });
    }
    // …then SoundCloud, even when local matches exist (to compare).
    try {
      const r = await api.alternativesSearch(original.id, "soundcloud");
      if (run.current === id) setSoundcloud({ state: "done", result: r });
    } catch (e) {
      if (run.current === id) setSoundcloud({ state: "failed", message: toApiError(e).message });
    }
  }, [original.id]);

  useEffect(() => {
    if (open) {
      setAdded(new Set());
      void search();
    } else {
      run.current++; // ignore late results
    }
  }, [open, search]);

  const choose = async (t: Track) => {
    if (adding) return; // one at a time: repeated clicks can't add twice
    setAdding(t.id);
    try {
      const r = await api.alternativesAdd(playlistId, original.id, t.id, position);
      setAdded((s) => new Set(s).add(t.id));
      if (r.kind === "AlreadyAdded") {
        toast.info(`“${t.title}” is already in this playlist as an alternative`);
      } else {
        toast.success(`Added “${t.title}” after “${original.title}”`, {
          description: synced ? "It's in Branflakes only: Spotify can't hold local or SoundCloud tracks. The original stays on Spotify." : "The original track stays in the playlist.",
        });
      }
      onAdded(t);
      onOpenChange(false);
    } catch (e) {
      toast.error("Couldn't add the alternative", { description: toApiError(e).message });
    } finally {
      setAdding(null);
    }
  };

  const loading = local.state === "loading" || soundcloud.state === "loading";
  const count = (p: Phase) => (p.state === "done" ? p.result.candidates.length : 0);
  const nothing = !loading && count(local) === 0 && count(soundcloud) === 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[85vh] max-w-2xl flex-col">
        <DialogHeader>
          <DialogTitle>Find alternative</DialogTitle>
          <DialogDescription>
            “{original.title}” by {original.artist || "unknown artist"} isn't playable on Spotify. Pick a version to add right after it. The original stays where it is.
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-48 flex-1 space-y-4 overflow-y-auto" aria-live="polite" aria-busy={loading}>
          <Group title="On this computer" phase={local} playlistAlready={alreadyAdded} added={added} adding={adding} onChoose={choose} />
          <Group title="SoundCloud" phase={soundcloud} playlistAlready={alreadyAdded} added={added} adding={adding} onChoose={choose} />
          {nothing && (
            <div className="rounded-md border border-dashed p-4 text-center text-sm text-muted-foreground">
              <p>No matching versions of this song were found.</p>
              <p className="mt-1 text-xs">Results must match both the title and the artist.</p>
            </div>
          )}
        </div>
        <div className="flex items-center justify-between pt-2">
          <p className="text-xs text-muted-foreground">{synced ? "Spotify Synced playlist: alternatives are kept in Branflakes only." : ""}</p>
          <Button size="sm" variant="ghost" disabled={loading} onClick={() => void search()}>
            <RotateCw className={loading ? "size-4 animate-spin" : "size-4"} /> Search again
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function Group({
  title,
  phase,
  playlistAlready,
  added,
  adding,
  onChoose,
}: {
  title: string;
  phase: Phase;
  playlistAlready: Set<string>;
  added: Set<string>;
  adding: string | null;
  onChoose(t: Track): void;
}) {
  return (
    <section aria-label={`${title} results`}>
      <h3 className="mb-1 flex items-center gap-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {title}
        {phase.state === "loading" && <Loader2 className="size-3.5 animate-spin" aria-label="Searching" />}
      </h3>
      {phase.state === "failed" && <p className="text-sm text-destructive">Search failed: {phase.message}</p>}
      {phase.state === "done" && phase.result.error && <p className="text-sm text-destructive">Search failed: {phase.result.error.message}</p>}
      {phase.state === "done" && phase.result.unavailable && <p className="text-sm text-muted-foreground">{phase.result.unavailable}</p>}
      {phase.state === "done" && !phase.result.error && !phase.result.unavailable && phase.result.candidates.length === 0 && (
        <p className="text-sm text-muted-foreground">No matches.</p>
      )}
      {phase.state === "done" && phase.result.candidates.length > 0 && (
        <ul className="space-y-0.5">
          {phase.result.candidates.map(({ track: t }) => {
            const done = added.has(t.id) || playlistAlready.has(t.id);
            return (
              <li key={t.id} className="flex items-center gap-3 rounded-md px-2 py-1.5 hover:bg-accent/60">
                <Artwork src={t.artwork} className="size-10 shrink-0" />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{t.title}</div>
                  <div className="truncate text-xs text-muted-foreground">{t.artist || t.uploader || "Unknown artist"}</div>
                </div>
                <SourceBadge source={t.source} />
                <span className="w-10 text-right text-xs tabular-nums text-muted-foreground" aria-label="Duration">
                  {formatDuration(t.durationMs)}
                </span>
                <Button
                  size="sm"
                  variant={done ? "ghost" : "secondary"}
                  className="w-24"
                  disabled={done || adding != null}
                  aria-label={done ? `${t.title} already added` : `Use ${t.title}`}
                  onClick={() => onChoose(t)}
                >
                  {adding === t.id ? <Loader2 className="size-4 animate-spin" /> : done ? <Check className="size-4" /> : <Plus className="size-4" />}
                  {done ? "Added" : "Use"}
                </Button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
