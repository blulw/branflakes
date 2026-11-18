import type { LyricLine } from "./types";

/** Lines are highlighted slightly early so they appear as the singing starts. */
export const LYRIC_LEAD_MS = 250;

/**
 * The line at `positionMs`: `index` is the last lyric line (not a section
 * header) that has started, -1 before the first; `singing` is false in a gap
 * after a line has ended and before the next starts. Lines are sorted by time.
 */
export function activeLine(lines: LyricLine[], positionMs: number): { index: number; singing: boolean } {
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
  while (found >= 0 && lines[found].header) found--;
  if (found < 0) return { index: -1, singing: false };
  const end = lines[found].endMs;
  return { index: found, singing: end == null || t < end };
}

/** Genius section headers such as "[Chorus]" or "[Verse 2: Artist]". */
export function isSectionHeader(line: string): boolean {
  return /^\[[^\]]+\]$/.test(line.trim());
}
