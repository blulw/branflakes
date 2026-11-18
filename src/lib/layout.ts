import { useEffect, useState } from "react";

/**
 * Window-width driven layout. As the window narrows, sections give way in
 * this order:
 *   1. the main view (only while lyrics are open — lyrics take its place),
 *   2. the text on the sidebar buttons (and the "branflakes" title),
 *   3. the playlist titles — leaving a rail of icons and playlist covers.
 * With lyrics closed the main view never disappears; it gets thinner instead.
 */

export interface Layout {
  /** Sidebar width in px; shrinks smoothly with the window. */
  sidebarWidth: number;
  /** Sidebar button labels (Songs, Artists…) are shown. */
  sidebarLabels: boolean;
  /** Playlist titles are shown next to their covers. */
  playlistTitles: boolean;
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
const CONTENT_MIN = MIN_WINDOW_WIDTH - SIDEBAR_RAIL; // 416: at the minimum width the sidebar is a rail

export function computeLayout(windowWidth: number, lyricsOpen: boolean): Layout {
  const w = Math.max(windowWidth, MIN_WINDOW_WIDTH);
  const docked = lyricsOpen && w >= SIDEBAR_FULL + LYRICS_DOCKED + MAIN_BESIDE_LYRICS_MIN;
  const sidebarWidth = docked ? SIDEBAR_FULL : Math.round(Math.min(SIDEBAR_FULL, Math.max(SIDEBAR_RAIL, w - CONTENT_MIN)));
  return {
    sidebarWidth,
    sidebarLabels: sidebarWidth >= SIDEBAR_LABELS_MIN,
    playlistTitles: sidebarWidth >= SIDEBAR_TITLES_MIN,
    showMain: docked || !lyricsOpen,
    lyricsDocked: docked,
  };
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
