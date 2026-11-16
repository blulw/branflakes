import { GitFork } from "lucide-react";
import { cn } from "@/lib/utils";
import type { Playlist } from "@/lib/types";

/** Small Spotify glyph used to mark "Spotify Synced" playlists. */
export function SpotifyGlyph({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} aria-hidden fill="currentColor">
      <path d="M12 1.5a10.5 10.5 0 1 0 0 21 10.5 10.5 0 0 0 0-21Zm4.82 15.15a.65.65 0 0 1-.9.22c-2.46-1.5-5.56-1.84-9.21-1.01a.65.65 0 1 1-.29-1.27c4-.91 7.43-.52 10.18 1.16.31.19.41.59.22.9Zm1.29-2.86a.82.82 0 0 1-1.12.27c-2.82-1.73-7.11-2.23-10.44-1.22a.82.82 0 0 1-.47-1.57c3.8-1.15 8.53-.59 11.77 1.4.38.24.5.74.26 1.12Zm.11-2.98C14.84 8.8 9.27 8.61 6.05 9.59a.98.98 0 1 1-.57-1.88c3.7-1.12 9.84-.9 13.72 1.4a.98.98 0 0 1-1 1.7Z" />
    </svg>
  );
}

export type PlaylistKind = "synced" | "fork" | "plain";

export function playlistKind(p: Pick<Playlist, "sync" | "forkedFrom">): PlaylistKind {
  if (p.sync) return "synced";
  if (p.forkedFrom) return "fork";
  return "plain";
}

export function playlistKindLabel(p: Pick<Playlist, "sync" | "forkedFrom" | "forkedFromName">): string | null {
  switch (playlistKind(p)) {
    case "synced":
      return "Spotify Synced";
    case "fork":
      return p.forkedFromName ? `Branflakes-only copy of “${p.forkedFromName}”` : "Branflakes-only copy";
    default:
      return null;
  }
}

/**
 * Subtle marker shown wherever a playlist is listed: a Spotify glyph for
 * synced playlists, a fork glyph for Branflakes-only copies, nothing for
 * ordinary playlists.
 */
export function PlaylistKindIcon({ playlist, className }: { playlist: Pick<Playlist, "sync" | "forkedFrom" | "forkedFromName">; className?: string }) {
  const kind = playlistKind(playlist);
  const label = playlistKindLabel(playlist);
  if (kind === "plain" || !label) return null;
  return (
    <span title={label} aria-label={label} role="img" className={cn("inline-flex shrink-0 items-center", className)}>
      {kind === "synced" ? (
        <SpotifyGlyph className={cn("size-3.5 text-source-spotify", playlist.sync?.status === "error" && "opacity-60")} />
      ) : (
        <GitFork className="size-3.5 text-muted-foreground" />
      )}
    </span>
  );
}
