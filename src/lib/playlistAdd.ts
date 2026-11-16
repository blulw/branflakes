// The one place UI code adds tracks to a playlist. The backend enforces the
// Spotify Synced rule (non-Spotify tracks create a Branflakes-only copy);
// this turns the outcome into friendly in-app feedback.
import { toast } from "sonner";
import { api, toApiError } from "./api";
import type { AddOutcome, Playlist, Track } from "./types";

export interface AddResult {
  outcome: AddOutcome | null;
  /** The playlist the tracks actually ended up in. */
  target: Playlist | null;
}

export async function addTracksToPlaylist(
  playlist: Pick<Playlist, "id" | "name">,
  tracks: Pick<Track, "id" | "title">[],
  opts: { position?: number; onOpen?: (p: Playlist) => void; quiet?: boolean } = {},
): Promise<AddResult> {
  try {
    const outcome = await api.addToPlaylist(
      playlist.id,
      tracks.map((t) => t.id),
      opts.position,
    );
    if (outcome.kind === "Forked") {
      const what = tracks.length === 1 ? `“${tracks[0].title}” isn't on Spotify` : "Some of these tracks aren't on Spotify";
      // Never imply the copy is complete or current when it isn't.
      const incomplete = outcome.uncopyable.length > 0 || !outcome.fromLatest;
      const lines = [`“${playlist.name}” stays in sync with Spotify. Your tracks were added to “${outcome.playlist.name}”.`];
      if (!outcome.fromLatest) lines.push("Spotify couldn't be checked, so the copy is based on the last sync.");
      if (outcome.uncopyable.length > 0) {
        const n = outcome.uncopyable.length;
        lines.push(`${n} Spotify ${n === 1 ? "entry" : "entries"} (podcast episodes, Spotify local files or unavailable items) couldn't be copied.`);
      }
      (incomplete ? toast.warning : toast.info)(`${what}, so Branflakes made a local-only copy`, {
        description: lines.join(" "),
        duration: incomplete ? 12000 : 8000,
        action: opts.onOpen ? { label: "Open copy", onClick: () => opts.onOpen?.(outcome.playlist) } : undefined,
      });
      return { outcome, target: outcome.playlist };
    }
    if (!opts.quiet) {
      if (outcome.added === 0 && outcome.skippedDuplicates > 0) {
        toast.info(`Already in “${playlist.name}”`);
      } else if (outcome.skippedDuplicates > 0) {
        toast.success(`Added ${outcome.added} to ${playlist.name}`, { description: `${outcome.skippedDuplicates} already in the playlist` });
      } else {
        toast.success(`Added to ${playlist.name}`);
      }
    }
    return { outcome, target: null };
  } catch (e) {
    toast.error(toApiError(e).message);
    return { outcome: null, target: null };
  }
}
