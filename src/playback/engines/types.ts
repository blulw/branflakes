import type { Source, Track } from "@/lib/types";

export interface EngineCallbacks {
  onTime(positionMs: number): void;
  onDuration(durationMs: number | null): void;
  onPlaying(playing: boolean): void;
  onEnded(): void;
  onError(message: string): void;
}

/**
 * A playback engine owns audio for one source. Only one engine is active at
 * a time; the controller calls `release()` on the old engine before loading
 * a track on a different one, so there is never cross-provider mixing.
 */
export interface PlaybackEngine {
  readonly sources: readonly Source[];
  load(track: Track, startMs: number, autoplay: boolean): Promise<void>;
  play(): Promise<void>;
  pause(): Promise<void>;
  seek(positionMs: number): Promise<void>;
  /** 0..1 */
  setVolume(volume: number): Promise<void>;
  /** Stop and give up ownership of output. */
  release(): Promise<void>;
}
