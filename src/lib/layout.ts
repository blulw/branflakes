import { useEffect, useState } from "react";

/**
 * Window-width driven layout. As the window narrows, sections give way in
 * this order:
 *   1. the main view (only while lyrics are open — lyrics take its place),
 *   2. the text on the sidebar buttons (and the "branflakes" title),
 *   3. the playlist titles — leaving a rail of icons and playlist covers.
 * With lyrics closed the main view never disappears; it gets thinner instead.
 */

export type SidebarMode = "full" | "icons" | "rail";

export interface Layout {
  sidebarMode: SidebarMode;
  sidebarWidth: number;
  /** Main view (songs, playlist, search…) is shown. */
  showMain: boolean;
  /** Lyrics sit beside the main view at a fixed width (otherwise they fill the space). */
  lyricsDocked: boolean;
}

export const MIN_WINDOW_WIDTH = 480;

const SIDEBAR_FULL = 240;
/** Below this the sidebar button labels go; playlists keep their titles. */
const SIDEBAR_LABELS_MIN = 200;
/** Below this the playlist titles go too; only icons and covers remain. */
const SIDEBAR_TITLES_MIN = 140;
export const SIDEBAR_RAIL = 64;
const LYRICS_DOCKED = 320;
/** Narrowest the main view may get while it shares the window with lyrics. */
const MAIN_BESIDE_LYRICS_MIN = 400;
/** Narrowest the one content section (main view or lyrics) gets before the sidebar shrinks. */
const CONTENT_MIN = 380;

export function computeLayout(windowWidth: number, lyricsOpen: boolean): Layout {
  const w = Math.max(windowWidth, MIN_WINDOW_WIDTH);
  if (lyricsOpen && w >= SIDEBAR_FULL + LYRICS_DOCKED + MAIN_BESIDE_LYRICS_MIN) {
    return { sidebarMode: "full", sidebarWidth: SIDEBAR_FULL, showMain: true, lyricsDocked: true };
  }
  const base = { showMain: !lyricsOpen, lyricsDocked: false };
  const room = w - CONTENT_MIN;
  if (room >= SIDEBAR_LABELS_MIN) return { ...base, sidebarMode: "full", sidebarWidth: Math.min(SIDEBAR_FULL, room) };
  if (room >= SIDEBAR_TITLES_MIN) return { ...base, sidebarMode: "icons", sidebarWidth: room };
  return { ...base, sidebarMode: "rail", sidebarWidth: SIDEBAR_RAIL };
}

export function useWindowWidth(): number {
  const [width, setWidth] = useState(() => window.innerWidth);
  useEffect(() => {
    const on = () => setWidth(window.innerWidth);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return width;
}
