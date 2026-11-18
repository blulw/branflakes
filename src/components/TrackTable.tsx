import { useMemo, useRef, useState, type ReactNode } from "react";
import { AlertTriangle, ArrowDown, ArrowUp, MoreHorizontal, Play, Volume2 } from "lucide-react";
import { Artwork } from "./Artwork";
import { SourceBadge } from "./SourceBadge";
import { TrackMenu } from "./TrackMenu";
import { Button } from "@/components/ui/button";
import { formatDuration } from "@/lib/format";
import { cn } from "@/lib/utils";
import { usePlayer } from "@/playback/store";
import type { SortKey, Track } from "@/lib/types";

const ROW = 48;
const OVERSCAN = 10;

export interface TrackTableProps {
  tracks: Track[];
  sort?: SortKey;
  descending?: boolean;
  onSort?: (key: SortKey) => void;
  /** Extra menu items per row (e.g. "Remove from playlist"). */
  extraMenu?: (track: Track, index: number) => ReactNode;
  empty?: ReactNode;
  showAlbum?: boolean;
  /** Called when the user starts playback from this table. */
  onPlayStart?: () => void;
  /** Small per-row controls shown after the title (e.g. "Find alternative"). */
  rowAccessory?: (track: Track, index: number) => ReactNode;
}

export function TrackTable({ tracks, sort, descending, onSort, extraMenu, empty, showAlbum = true, onPlayStart, rowAccessory }: TrackTableProps) {
  const scroller = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(800);
  const playTracksRaw = usePlayer((s) => s.playTracks);
  const playTracks = (list: Track[], i: number) => {
    onPlayStart?.();
    return playTracksRaw(list, i);
  };
  const loadedTrackId = usePlayer((s) => s.loadedTrackId);
  const status = usePlayer((s) => s.status);

  const range = useMemo(() => {
    const start = Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN);
    const end = Math.min(tracks.length, Math.ceil((scrollTop + height) / ROW) + OVERSCAN);
    return { start, end };
  }, [scrollTop, height, tracks.length]);

  if (!tracks.length) {
    return <div className="grid flex-1 place-items-center p-10 text-sm text-muted-foreground">{empty ?? "No tracks"}</div>;
  }

  const header = (key: SortKey, label: string, className?: string) => (
    <button
      type="button"
      disabled={!onSort}
      onClick={() => onSort?.(key)}
      className={cn("flex items-center gap-1 text-left hover:text-foreground disabled:hover:text-inherit", className)}
    >
      {label}
      {sort === key && (descending ? <ArrowDown className="size-3" /> : <ArrowUp className="size-3" />)}
    </button>
  );

  // Columns drop out as the main view narrows: album, then source, then time.
  const cols = cn(
    "grid-cols-[2rem_minmax(0,1fr)_2rem] gap-2 px-2 @sm:grid-cols-[2rem_minmax(0,1fr)_3rem_2rem] @xl:grid-cols-[2.5rem_minmax(0,3fr)_6.5rem_4rem_2.5rem] @xl:gap-3 @xl:px-4",
    showAlbum && "@3xl:grid-cols-[2.5rem_minmax(0,3fr)_minmax(0,2fr)_6.5rem_4rem_2.5rem]",
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className={cn("grid items-center border-b py-2 text-xs font-medium text-muted-foreground", cols)}>
        <span className="text-right">#</span>
        {header("title", "Title")}
        {showAlbum && header("album", "Album", "hidden @3xl:flex")}
        {header("source", "Source", "hidden @xl:flex")}
        {header("duration", "Time", "hidden justify-end @sm:flex")}
        <span />
      </div>
      <div
        ref={(el) => {
          scroller.current = el;
          if (el && el.clientHeight !== height) setHeight(el.clientHeight);
        }}
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
        className="min-h-0 flex-1 overflow-y-auto"
        role="grid"
        aria-rowcount={tracks.length}
      >
        <div style={{ height: tracks.length * ROW, position: "relative" }}>
          {tracks.slice(range.start, range.end).map((t, k) => {
            const i = range.start + k;
            const isCurrent = t.id === loadedTrackId;
            const unavailable = t.availability === "unavailable";
            return (
              <div
                key={`${t.id}-${i}`}
                role="row"
                aria-rowindex={i + 1}
                style={{ position: "absolute", top: i * ROW, left: 0, right: 0, height: ROW }}
                onDoubleClick={() => void playTracks(tracks, i)}
                title={unavailable ? t.unavailableReason ?? "Unavailable" : undefined}
                className={cn(
                  "group grid items-center text-sm hover:bg-accent/60",
                  cols,
                  isCurrent && "bg-primary/10",
                  unavailable && "[&_.dim]:opacity-50",
                )}
              >
                <span className="relative flex justify-end text-xs tabular-nums text-muted-foreground">
                  <span className="group-hover:invisible">
                    {isCurrent && status === "playing" ? <Volume2 className="size-4 text-primary" /> : i + 1}
                  </span>
                  <button
                    type="button"
                    aria-label={`Play ${t.title}`}
                    onClick={() => void playTracks(tracks, i)}
                    className="invisible absolute right-0 top-1/2 -translate-y-1/2 group-hover:visible"
                  >
                    <Play className="size-4 fill-current" />
                  </button>
                </span>
                <div className="flex min-w-0 items-center gap-3">
                  <Artwork src={t.artwork} className="dim size-8 shrink-0" />
                  <div className="min-w-0">
                    <div className={cn("flex items-center gap-1.5 truncate font-medium", isCurrent && "text-primary")}>
                      <span className="dim truncate">{t.title}</span>
                      {unavailable && <AlertTriangle className="size-3.5 shrink-0 text-destructive" aria-label="Unavailable" />}
                      {t.availability === "preview_only" && <span className="shrink-0 rounded bg-muted px-1 text-[10px] text-muted-foreground">PREVIEW</span>}
                      {rowAccessory?.(t, i)}
                    </div>
                    <div className="dim truncate text-xs text-muted-foreground">{t.artist || "Unknown artist"}</div>
                  </div>
                </div>
                {showAlbum && <span className="dim hidden truncate text-muted-foreground @3xl:block">{t.album}</span>}
                <span className="dim hidden @xl:block">
                  <SourceBadge source={t.source} />
                </span>
                <span className="dim hidden text-right text-xs tabular-nums @sm:block text-muted-foreground">{formatDuration(t.durationMs)}</span>
                <TrackMenu track={t} extra={extraMenu?.(t, i)}>
                  <Button variant="ghost" size="icon" className="size-8 opacity-0 group-hover:opacity-100 data-[state=open]:opacity-100" aria-label="Track actions">
                    <MoreHorizontal className="size-4" />
                  </Button>
                </TrackMenu>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
