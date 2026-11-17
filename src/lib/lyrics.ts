import type { LyricLine } from "./types";

/** Lines are highlighted slightly early so they appear as the singing starts. */
export const LYRIC_LEAD_MS = 250;

/** Index of the line being sung at `positionMs` (-1 before the first line). Lines are sorted by time. */
export function activeLineIndex(lines: LyricLine[], positionMs: number): number {
  const t = positionMs + LYRIC_LEAD_MS;
  let lo = 0;
  let hi = lines.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (lines[mid].timeMs <= t) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
}

/** Genius section headers such as "[Chorus]" or "[Verse 2: Artist]". */
export function isSectionHeader(line: string): boolean {
  return /^\[[^\]]+\]$/.test(line.trim());
}
