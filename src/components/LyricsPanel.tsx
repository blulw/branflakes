import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AlertCircle, Loader2, Music2, RefreshCw, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api, toApiError } from "@/lib/api";
import { activeLine, isSectionHeader } from "@/lib/lyrics";
import type { Lyrics, LyricsSource } from "@/lib/types";
import { cn } from "@/lib/utils";
import { formatDuration } from "@/lib/format";
import { usePlayer } from "@/playback/store";
import { useUi } from "@/state/ui";

type Load = { status: "idle" } | { status: "loading" } | { status: "error"; message: string } | { status: "done"; lyrics: Lyrics | null };

/** After the user scrolls, stop following the song for this long. */
const MANUAL_SCROLL_PAUSE_MS = 4000;

const SOURCE_NAME: Record<LyricsSource, string> = {
  genius: "Genius",
  lrclib: "LRCLIB",
  musixmatch: "Musixmatch",
  netease: "NetEase Cloud Music",
};

/** `docked`: fixed width beside the main view; otherwise it fills the window. */
/** Loads lyrics for a track; `lookUpAgain` bypasses the cache. */
export function useLyrics(trackId: string | null) {
  const [load, setLoad] = useState<Load>({ status: "idle" });
  const [reload, setReload] = useState(0);
  const refreshFor = useRef<string | null>(null);
  const lookUpAgain = () => {
    refreshFor.current = trackId;
    setReload((n) => n + 1);
  };

  useEffect(() => {
    if (!trackId) {
      setLoad({ status: "idle" });
      return;
    }
    let cancelled = false;
    setLoad({ status: "loading" });
    // Only an explicit "look up again" for this track bypasses the cache.
    const refresh = refreshFor.current === trackId;
    refreshFor.current = null;
    api
      .getLyrics(trackId, refresh)
      .then((lyrics) => !cancelled && setLoad({ status: "done", lyrics }))
      .catch((e) => !cancelled && setLoad({ status: "error", message: toApiError(e).message }));
    return () => {
      cancelled = true;
    };
  }, [trackId, reload]);

  const lyrics = load.status === "done" ? load.lyrics : null;
  return { load, lyrics, lookUpAgain, showSynced: !!lyrics?.synced?.length };
}

export function LyricsPanel({ docked = true }: { docked?: boolean }) {
  const track = usePlayer((s) => s.currentTrack());
  const toggleLyrics = useUi((s) => s.toggleLyrics);
  const { load, lyrics, lookUpAgain } = useLyrics(track?.id ?? null);
  // Synced lines are Genius's words whenever Genius has the song.
  const showSynced = !!lyrics?.synced?.length;

  return (
    <aside className={cn("flex flex-col border-l bg-sidebar", docked ? "w-80 shrink-0 xl:w-96" : "min-w-0 flex-1")} aria-label="Lyrics">
      <header className="flex items-center gap-2 border-b px-4 py-3">
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold">Lyrics</h2>
          {track && (
            <p className="truncate text-xs text-muted-foreground">
              {track.title} · {track.artist}
            </p>
          )}
        </div>
        {track && (
          <Button
            variant="ghost"
            size="icon"
            className="size-8"
            aria-label="Look up lyrics again"
            title="Look up again"
            disabled={load.status === "loading"}
            onClick={lookUpAgain}
          >
            <RefreshCw className="size-4" />
          </Button>
        )}
        <Button variant="ghost" size="icon" className="size-8" aria-label="Close lyrics" onClick={toggleLyrics}>
          <X className="size-4" />
        </Button>
      </header>

      <div className="min-h-0 flex-1">
        {!track ? (
          <Empty>Play a song to see its lyrics.</Empty>
        ) : load.status === "loading" || load.status === "idle" ? (
          <div className="grid h-full place-items-center">
            <Loader2 className="size-5 animate-spin text-muted-foreground" />
          </div>
        ) : load.status === "error" ? (
          <Empty>
            <AlertCircle className="mx-auto mb-2 size-5 text-destructive" />
            <p>Couldn't load lyrics.</p>
            <p className="mt-1 text-xs" data-selectable>
              {load.message}
            </p>
            <Button variant="outline" size="sm" className="mt-3" onClick={lookUpAgain}>
              Try again
            </Button>
          </Empty>
        ) : !lyrics ? (
          <Empty>No lyrics found for this song.</Empty>
        ) : lyrics.instrumental ? (
          <Empty>
            <Music2 className="mx-auto mb-2 size-5" />
            Instrumental
          </Empty>
        ) : showSynced ? (
          <SyncedLyrics key={track.id} lyrics={lyrics} trackId={track.id} />
        ) : (
          <PlainLyrics text={lyrics.plain ?? ""} />
        )}
      </div>

      {lyrics && (lyrics.geniusUrl || lyrics.plainSource) && (
        <footer className="space-y-0.5 border-t px-4 py-2 text-[11px] text-muted-foreground">
          {lyrics.plainSource === "genius" && lyrics.geniusUrl ? (
            <button className="underline-offset-2 hover:underline" onClick={() => void openUrl(lyrics.geniusUrl!)}>
              Lyrics from Genius
            </button>
          ) : lyrics.plainSource ? (
            <p>Lyrics from {SOURCE_NAME[lyrics.plainSource]}</p>
          ) : null}
          {showSynced && lyrics.syncSource && lyrics.syncSource !== lyrics.plainSource && <p>Timing from {SOURCE_NAME[lyrics.syncSource]}</p>}
          {lyrics.plainSource !== "genius" && lyrics.geniusUrl && (
            <button className="block underline-offset-2 hover:underline" onClick={() => void openUrl(lyrics.geniusUrl!)}>
              View on Genius
            </button>
          )}
        </footer>
      )}
    </aside>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <div className="grid h-full place-items-center p-6 text-center text-sm text-muted-foreground">
      <div>{children}</div>
    </div>
  );
}

export function PlainLyrics({ text }: { text: string }) {
  return (
    <div className="lyrics-scroll h-full overflow-y-auto px-5 py-4" data-selectable>
      {text.split("\n").map((line, i) =>
        line.trim() === "" ? (
          <div key={i} className="h-4" />
        ) : isSectionHeader(line) ? (
          <p key={i} className="mt-1 text-xs font-medium text-muted-foreground">
            {line}
          </p>
        ) : (
          <p key={i} className="text-[15px] leading-relaxed">
            {line}
          </p>
        ),
      )}
    </div>
  );
}

/** `big`: the full-screen view's larger lines. */
export function SyncedLyrics({ lyrics, trackId, big = false }: { lyrics: Lyrics; trackId: string; big?: boolean }) {
  const lines = lyrics.synced ?? [];
  const positionMs = usePlayer((s) => s.positionMs);
  const loaded = usePlayer((s) => s.loadedTrackId === trackId);
  // Any line of the current track can be clicked: it jumps there (starting the track if needed).
  const canSeek = usePlayer((s) => s.canPlayFromLyric(trackId));
  const playFromLyric = usePlayer((s) => s.playFromLyric);
  const { index: active, singing } = loaded ? activeLine(lines, positionMs) : { index: -1, singing: false };
  const containerRef = useRef<HTMLDivElement>(null);
  const lineRefs = useRef<(HTMLElement | null)[]>([]);
  const manualUntil = useRef(0);

  useEffect(() => {
    if (Date.now() < manualUntil.current) return;
    const el = lineRefs.current[Math.max(active, 0)];
    const box = containerRef.current;
    if (!el || !box) return;
    const top = el.offsetTop - box.clientHeight / 2 + el.clientHeight / 2;
    box.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
  }, [active]);

  const userScrolled = () => {
    manualUntil.current = Date.now() + MANUAL_SCROLL_PAUSE_MS;
  };
  const setRef = (i: number) => (el: HTMLElement | null) => {
    lineRefs.current[i] = el;
  };

  return (
    <div
      ref={containerRef}
      className={cn("lyrics-scroll relative h-full overflow-y-auto", big ? "px-8 py-[45vh]" : "px-5 py-[40%]")}
      onWheel={userScrolled}
      onTouchMove={userScrolled}
      onKeyDown={userScrolled}
    >
      {lines.map((line, i) => {
        if (line.header) {
          return (
            <p key={i} ref={setRef(i)} className={cn("pb-1 font-medium text-muted-foreground", big ? "text-sm" : "text-xs", i > 0 && "pt-5")}>
              {line.text}
            </p>
          );
        }
        const Tag = canSeek ? "button" : "p";
        const current = i === active && singing;
        return (
          <Tag
            key={i}
            ref={setRef(i)}
            onClick={
              canSeek
                ? () => {
                    manualUntil.current = 0;
                    void playFromLyric(trackId, line.timeMs);
                  }
                : undefined
            }
            title={canSeek ? `Play from ${formatDuration(line.timeMs)}` : undefined}
            aria-current={current ? "true" : undefined}
            className={cn(
              "block w-full text-left font-semibold leading-snug transition-colors duration-300",
              big ? "py-2.5 text-3xl" : "py-1.5 text-lg",
              current ? "text-foreground" : i <= active ? "text-muted-foreground/60" : "text-muted-foreground",
              canSeek && "-mx-2 w-[calc(100%+1rem)] cursor-pointer rounded-md px-2 hover:bg-foreground/5 hover:text-foreground/90",
            )}
          >
            {line.text || "♪"}
          </Tag>
        );
      })}
    </div>
  );
}
