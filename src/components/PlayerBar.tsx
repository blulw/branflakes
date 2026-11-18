import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AlertCircle, MicVocal, Pause, Play, Repeat, Repeat1, Shuffle, SkipBack, SkipForward, Volume1, Volume2, VolumeX, Loader2 } from "lucide-react";
import { Artwork } from "./Artwork";
import { SourceBadge } from "./SourceBadge";
import { DevicePicker } from "./DevicePicker";
import { Button } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { formatDuration } from "@/lib/format";
import { cn } from "@/lib/utils";
import { usePlayer } from "@/playback/store";
import { useUi } from "@/state/ui";
import type { ControlState } from "@/playback/controls";

function Ctl({ state, label, onClick, children, className }: { state: ControlState; label: string; onClick: () => void; children: React.ReactNode; className?: string }) {
  const button = (
    <Button
      variant="ghost"
      size="icon"
      aria-label={label}
      disabled={!state.enabled}
      onClick={onClick}
      className={cn("size-9 rounded-full", className)}
    >
      {children}
    </Button>
  );
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {/* span keeps the tooltip working on disabled buttons */}
        <span tabIndex={state.enabled ? -1 : 0}>{button}</span>
      </TooltipTrigger>
      <TooltipContent>{state.enabled ? label : state.reason ?? `${label} unavailable`}</TooltipContent>
    </Tooltip>
  );
}

export function PlayerBar() {
  const st = usePlayer();
  const track = st.currentTrack();
  const c = st.controls();
  const [scrub, setScrub] = useState<number | null>(null);
  const duration = st.durationMs ?? track?.durationMs ?? null;
  const position = scrub ?? st.positionMs;
  const lyricsOpen = useUi((s) => s.lyricsOpen);
  const toggleLyrics = useUi((s) => s.toggleLyrics);

  useEffect(() => {
    setScrub(null);
  }, [track?.id]);

  const VolumeIcon = st.volume === 0 ? VolumeX : st.volume < 0.5 ? Volume1 : Volume2;
  const RepeatIcon = st.repeatMode === "one" ? Repeat1 : Repeat;

  return (
    <footer className="grid h-24 shrink-0 grid-cols-[minmax(0,1fr)_minmax(0,2fr)_minmax(0,1fr)] items-center gap-4 border-t bg-sidebar px-4">
      {/* Now playing */}
      <div className="flex min-w-0 items-center gap-3">
        {track ? (
          <>
            <Artwork src={track.artwork} className="size-14 shrink-0" alt={`${track.title} artwork`} />
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <span className="truncate font-medium" data-selectable>
                  {track.title}
                </span>
                <SourceBadge source={track.source} compact />
              </div>
              <div className="truncate text-xs text-muted-foreground" data-selectable>
                {track.artist}
              </div>
              {track.source === "soundcloud" && (
                // SoundCloud attribution requirements: credit uploader, credit SoundCloud, link to the track.
                <div className="truncate text-[11px] text-muted-foreground">
                  by{" "}
                  <button className="underline-offset-2 hover:underline" onClick={() => track.uploaderUrl && void openUrl(track.uploaderUrl)}>
                    {track.uploader ?? track.artist}
                  </button>{" "}
                  ·{" "}
                  <button className="font-medium text-source-soundcloud underline-offset-2 hover:underline" onClick={() => track.permalinkUrl && void openUrl(track.permalinkUrl)}>
                    Listen on SoundCloud
                  </button>
                </div>
              )}
              {track.source === "spotify" && (
                <button className="text-[11px] text-source-spotify underline-offset-2 hover:underline" onClick={() => track.permalinkUrl && void openUrl(track.permalinkUrl)}>
                  Open in Spotify
                </button>
              )}
            </div>
          </>
        ) : (
          <span className="text-sm text-muted-foreground">Nothing playing</span>
        )}
      </div>

      {/* Transport */}
      <div className="flex flex-col items-center gap-1">
        <div className="flex items-center gap-1">
          <Ctl state={{ enabled: st.queue.length > 1 }} label="Shuffle upcoming" onClick={() => void st.shuffleUpcoming()} className={cn(st.shuffle && "text-primary")}>
            <Shuffle className="size-4" />
          </Ctl>
          <Ctl state={c.previous} label="Previous" onClick={() => void st.previous()}>
            <SkipBack className="size-4 fill-current" />
          </Ctl>
          <Ctl
            state={c.playPause}
            label={st.status === "playing" ? "Pause" : "Play"}
            onClick={() => void st.togglePlay()}
            className="size-11 bg-foreground text-background hover:bg-foreground/90 hover:text-background disabled:bg-muted disabled:text-muted-foreground"
          >
            {st.status === "loading" ? (
              <Loader2 className="size-5 animate-spin" />
            ) : st.status === "playing" ? (
              <Pause className="size-5 fill-current" />
            ) : (
              <Play className="size-5 translate-x-px fill-current" />
            )}
          </Ctl>
          <Ctl state={c.next} label="Next" onClick={() => void st.next(true)}>
            <SkipForward className="size-4 fill-current" />
          </Ctl>
          <Ctl state={{ enabled: st.queue.length > 0 }} label={`Repeat: ${st.repeatMode}`} onClick={st.cycleRepeat} className={cn(st.repeatMode !== "off" && "text-primary")}>
            <RepeatIcon className="size-4" />
          </Ctl>
        </div>
        <div className="flex w-full max-w-xl items-center gap-2 text-[11px] tabular-nums text-muted-foreground">
          <span className="w-10 text-right">{c.showPosition ? formatDuration(position) : ""}</span>
          {c.seek.enabled ? (
            <Slider
              aria-label="Seek"
              min={0}
              max={duration ?? 1}
              step={1000}
              value={[Math.min(position, duration ?? 0)]}
              onValueChange={([v]) => setScrub(v)}
              onValueCommit={([v]) => {
                setScrub(null);
                void st.seek(v);
              }}
              className="flex-1"
            />
          ) : (
            <Tooltip>
              <TooltipTrigger asChild>
                <div className="h-1 flex-1 rounded-full bg-muted" aria-label={c.seek.reason}>
                  {c.showPosition && duration ? (
                    <div className="h-full rounded-full bg-muted-foreground/50" style={{ width: `${Math.min(100, (position / duration) * 100)}%` }} />
                  ) : null}
                </div>
              </TooltipTrigger>
              <TooltipContent>{c.seek.reason}</TooltipContent>
            </Tooltip>
          )}
          <span className="w-10">{c.showDuration ? formatDuration(duration) : ""}</span>
        </div>
        {(c.blocker || st.error) && (
          <div className="flex items-center gap-1 text-[11px] text-destructive">
            <AlertCircle className="size-3" />
            <span className="truncate">{c.blocker ?? st.error}</span>
          </div>
        )}
      </div>

      {/* Device + volume */}
      <div className="flex items-center justify-end gap-2">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className={cn("size-8", lyricsOpen && "text-primary")}
              aria-label={lyricsOpen ? "Hide lyrics" : "Show lyrics"}
              aria-pressed={lyricsOpen}
              onClick={toggleLyrics}
            >
              <MicVocal className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{lyricsOpen ? "Hide lyrics" : "Lyrics"}</TooltipContent>
        </Tooltip>
        {track?.source === "spotify" && c.deviceSelection.enabled && <DevicePicker />}
        {c.volume.enabled ? (
          <div className="flex w-36 items-center gap-2">
            <Button variant="ghost" size="icon" className="size-8" aria-label={st.volume === 0 ? "Unmute" : "Mute"} onClick={() => void st.setVolume(st.volume === 0 ? 0.8 : 0)}>
              <VolumeIcon className="size-4" />
            </Button>
            <Slider aria-label="Volume" min={0} max={100} step={1} value={[Math.round(st.volume * 100)]} onValueChange={([v]) => void st.setVolume(v / 100)} />
          </div>
        ) : (
          track && (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="flex items-center gap-1 text-xs text-muted-foreground" tabIndex={0}>
                  <VolumeX className="size-4" /> n/a
                </span>
              </TooltipTrigger>
              <TooltipContent>{c.volume.reason}</TooltipContent>
            </Tooltip>
          )
        )}
      </div>
    </footer>
  );
}
