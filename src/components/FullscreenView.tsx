import { useCallback, useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Loader2, Minimize2, Music2, Volume1, Volume2, VolumeX } from "lucide-react";
import { Artwork } from "./Artwork";
import { SyncedLyrics, useLyrics } from "./LyricsPanel";
import { Button } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import { Transport } from "./PlayerBar";
import { usePlayer } from "@/playback/store";
import { useUi } from "@/state/ui";

/**
 * Maximizes the window (title bar and taskbar stay — not exclusive full
 * screen) while the full-screen view is open, without locking it there: if
 * the user resizes or moves the window out of maximized, the view closes
 * and the window stays where they put it. Closing the view any other way
 * puts the window back the way it was.
 */
function useMaximizedWhileOpen(onLeave: () => void) {
  useEffect(() => {
    const win = getCurrentWindow();
    let active = true;
    let userLeft = false;
    const unlisteners: (() => void)[] = [];
    const check = async () => {
      if (!active) return;
      const [max, min] = await Promise.all([win.isMaximized(), win.isMinimized()]);
      if (active && !max && !min) {
        userLeft = true;
        onLeave();
      }
    };
    const setup = (async () => {
      const wasMaximized = await win.isMaximized().catch(() => true);
      await win.maximize().catch(() => undefined);
      // Listen only once maximized, so the maximize itself doesn't count as the user leaving.
      for (const listen of [win.onResized.bind(win), win.onMoved.bind(win)]) {
        const off = await listen(() => void check()).catch(() => undefined);
        if (!off) continue;
        if (active) unlisteners.push(off);
        else off();
      }
      return wasMaximized;
    })();
    return () => {
      active = false;
      unlisteners.forEach((off) => off());
      void setup.then(async (wasMaximized) => {
        if (!wasMaximized && !userLeft) await win.unmaximize().catch(() => undefined);
      });
    };
  }, [onLeave]);
}

export function FullscreenView() {
  const track = usePlayer((s) => s.currentTrack());
  const setFullscreen = useUi((s) => s.setFullscreen);
  const { load, lyrics, showSynced } = useLyrics(track?.id ?? null);
  useMaximizedWhileOpen(useCallback(() => setFullscreen(false), [setFullscreen]));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setFullscreen(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setFullscreen]);

  const info = track ? (
    <div className="flex w-[min(38vh,32vw)] min-w-72 flex-col">
      <Artwork src={track.artwork} className="aspect-square w-full shadow-2xl" alt={`${track.title} artwork`} />
      <h1 className="mt-6 text-3xl font-bold leading-tight tracking-tight" data-selectable>
        {track.title}
      </h1>
      <p className="mt-1 text-xl text-muted-foreground" data-selectable>
        {track.artist}
      </p>
      {load.status === "loading" && <Loader2 className="mt-4 size-4 animate-spin text-muted-foreground" />}
      {lyrics?.instrumental && (
        <p className="mt-4 flex items-center gap-1.5 text-sm text-muted-foreground">
          <Music2 className="size-4" /> Instrumental
        </p>
      )}
      <div className="mt-6">
        <Transport large />
      </div>
      <VolumeRow />
    </div>
  ) : (
    <div className="flex w-72 flex-col items-center gap-6">
      <p className="text-lg text-muted-foreground">Nothing playing</p>
      <Transport large />
    </div>
  );

  return (
    // Transparent: the theme's background (painted on <body>) shows through the frosted pane.
    <div className="fullscreen-pane fixed inset-0 z-50 overflow-hidden" role="dialog" aria-modal aria-label="Now playing">

      <Button
        variant="ghost"
        size="icon"
        className="absolute right-4 top-4 z-10 size-9 text-muted-foreground hover:text-foreground"
        aria-label="Exit full screen"
        title="Exit full screen (Esc)"
        onClick={() => setFullscreen(false)}
      >
        <Minimize2 className="size-5" />
      </Button>

      {track && showSynced && lyrics ? (
        <div className="relative grid h-full grid-cols-[minmax(0,2fr)_minmax(0,3fr)] gap-12 px-[6vw]">
          <div className="flex items-center justify-end">{info}</div>
          <div className="min-h-0 [mask-image:linear-gradient(transparent,black_18%,black_82%,transparent)]">
            <SyncedLyrics key={track.id} lyrics={lyrics} trackId={track.id} big />
          </div>
        </div>
      ) : (
        <div className="relative grid h-full place-items-center">{info}</div>
      )}
    </div>
  );
}

function VolumeRow() {
  const volume = usePlayer((s) => s.volume);
  const setVolume = usePlayer((s) => s.setVolume);
  const enabled = usePlayer((s) => s.controls().volume.enabled);
  if (!enabled) return null;
  const Icon = volume === 0 ? VolumeX : volume < 0.5 ? Volume1 : Volume2;
  return (
    <div className="mx-auto mt-3 flex w-48 items-center gap-2 text-muted-foreground">
      <Button variant="ghost" size="icon" className="size-8 shrink-0" aria-label={volume === 0 ? "Unmute" : "Mute"} onClick={() => void setVolume(volume === 0 ? 0.8 : 0)}>
        <Icon className="size-4" />
      </Button>
      <Slider aria-label="Volume" min={0} max={100} step={1} value={[Math.round(volume * 100)]} onValueChange={([v]) => void setVolume(v / 100)} />
    </div>
  );
}
