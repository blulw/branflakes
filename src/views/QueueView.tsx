import { ArrowDown, ArrowUp, X } from "lucide-react";
import { Artwork } from "@/components/Artwork";
import { SourceBadge } from "@/components/SourceBadge";
import { Button } from "@/components/ui/button";
import { formatDuration } from "@/lib/format";
import { cn } from "@/lib/utils";
import { usePlayer } from "@/playback/store";

export function QueueView() {
  const { queue, currentIndex, playAt, removeAt, move, clearQueue } = usePlayer();
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-end justify-between gap-3 px-4 pb-3 pt-5 @xl:px-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Queue</h1>
          <p className="text-xs text-muted-foreground">
            {queue.length} tracks
          </p>
        </div>
        <Button size="sm" variant="secondary" disabled={!queue.length} onClick={() => void clearQueue()}>
          Clear
        </Button>
      </header>
      <ol className="min-h-0 flex-1 overflow-y-auto px-2 pb-4 @xl:px-4">
        {queue.map((t, i) => (
          <li
            key={`${t.id}-${i}`}
            onDoubleClick={() => void playAt(i)}
            className={cn("group flex items-center gap-3 rounded-md px-3 py-1.5 hover:bg-accent", i === currentIndex && "bg-primary/10", t.availability === "unavailable" && "opacity-50")}
            title={t.availability === "unavailable" ? t.unavailableReason ?? "" : undefined}
          >
            <span className="w-6 text-right text-xs tabular-nums text-muted-foreground">{i + 1}</span>
            <Artwork src={t.artwork} className="size-8" />
            <div className="min-w-0 flex-1">
              <div className={cn("truncate text-sm font-medium", i === currentIndex && "text-primary")}>{t.title}</div>
              <div className="truncate text-xs text-muted-foreground">{t.artist}</div>
            </div>
            <span className="hidden @xl:contents">
              <SourceBadge source={t.source} />
            </span>
            <span className="hidden w-12 text-right @sm:block text-xs tabular-nums text-muted-foreground">{formatDuration(t.durationMs)}</span>
            <div className="flex opacity-0 group-hover:opacity-100">
              <Button variant="ghost" size="icon" className="size-7" disabled={i === 0} onClick={() => void move(i, i - 1)} aria-label="Move up">
                <ArrowUp className="size-3.5" />
              </Button>
              <Button variant="ghost" size="icon" className="size-7" disabled={i === queue.length - 1} onClick={() => void move(i, i + 1)} aria-label="Move down">
                <ArrowDown className="size-3.5" />
              </Button>
              <Button variant="ghost" size="icon" className="size-7" onClick={() => void removeAt(i)} aria-label="Remove from queue">
                <X className="size-3.5" />
              </Button>
            </div>
          </li>
        ))}
        {queue.length === 0 && <p className="p-6 text-sm text-muted-foreground">The queue is empty. Double-click a track to start playing.</p>}
      </ol>
    </div>
  );
}
