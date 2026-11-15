import { convertFileSrc } from "@tauri-apps/api/core";
import type { Source, Track } from "./types";

export function formatDuration(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return "–:––";
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = s.toString().padStart(2, "0");
  return h > 0 ? `${h}:${m.toString().padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

export const SOURCE_LABEL: Record<Source, string> = {
  local: "Local",
  spotify: "Spotify",
  soundcloud: "SoundCloud",
};

/** True for an absolute Windows/Unix filesystem path (not a URL). */
export function isLocalPath(p: string): boolean {
  return /^[a-zA-Z]:[\\/]/.test(p) || p.startsWith("\\\\") || p.startsWith("/");
}

/** URL usable in <img src>. Local paths go through Tauri's asset protocol. */
export function artworkSrc(artwork: string | null | undefined): string | null {
  if (!artwork) return null;
  if (isLocalPath(artwork)) {
    try {
      return convertFileSrc(artwork);
    } catch {
      return null;
    }
  }
  return artwork.startsWith("https://") ? artwork : null;
}

/** file:// URL (for Windows SMTC thumbnails) or https URL. */
export function artworkForSystem(artwork: string | null | undefined): string | undefined {
  if (!artwork) return undefined;
  if (isLocalPath(artwork)) return "file:///" + artwork.replace(/\\/g, "/").replace(/^\/+/, "");
  return artwork.startsWith("https://") ? artwork : undefined;
}

export function trackSubtitle(t: Track): string {
  return [t.artist, t.album].filter(Boolean).join(" — ");
}
