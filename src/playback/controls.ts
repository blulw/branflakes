// Pure capability → UI control resolution. The player bar renders exactly
// what this function returns; nothing else decides whether a control works.
import type { PlaybackCapabilities, RepeatMode, SpotifyDevice, Track } from "@/lib/types";

export interface ControlInput {
  track: Track | null;
  /** Provider capabilities from the backend for `track.source`. */
  caps: PlaybackCapabilities | null;
  queueLength: number;
  currentIndex: number | null;
  repeatMode: RepeatMode;
  positionMs: number;
  durationMs: number | null;
  /** Spotify only: the device that would receive commands. */
  spotifyDevice?: SpotifyDevice | null;
  /** Spotify only: actions Spotify reports as disallowed (e.g. "seeking"). */
  spotifyDisallows?: string[];
}

export interface ControlState {
  enabled: boolean;
  /** Why the control is disabled (tooltip). */
  reason?: string;
}

export interface ResolvedControls {
  playPause: ControlState;
  next: ControlState;
  previous: ControlState;
  seek: ControlState;
  volume: ControlState;
  deviceSelection: ControlState;
  showPosition: boolean;
  showDuration: boolean;
  /** Message shown in the player bar when playback is impossible. */
  blocker?: string;
}

const off = (reason: string): ControlState => ({ enabled: false, reason });
const on: ControlState = { enabled: true };

export function resolveControls(i: ControlInput): ResolvedControls {
  const none: ResolvedControls = {
    playPause: off("Nothing is queued"),
    next: off("Nothing is queued"),
    previous: off("Nothing is queued"),
    seek: off("Nothing is playing"),
    volume: off("Nothing is playing"),
    deviceSelection: off("Nothing is playing"),
    showPosition: false,
    showDuration: false,
  };
  if (!i.track) return none;
  const caps = i.caps;
  if (!caps || !caps.play) {
    const why =
      i.track.source === "local"
        ? "Local playback unavailable"
        : i.track.source === "spotify"
          ? "Connect Spotify (Premium required for playback) in Settings to play this track"
          : "Connect SoundCloud or turn on SoundCloud web mode in Settings to play this track";
    return { ...none, playPause: off(why), blocker: why, ...queueNav(i, caps) };
  }

  let blocker: string | undefined;
  let playPause: ControlState = on;
  if (i.track.availability === "unavailable") {
    blocker = i.track.unavailableReason ?? "This track is unavailable";
    playPause = off(blocker);
  }

  const isSpotify = i.track.source === "spotify";
  const disallows = i.spotifyDisallows ?? [];
  if (isSpotify && caps.deviceSelection && !i.spotifyDevice && !blocker) {
    blocker = "Choose a Spotify Connect device to play Spotify tracks";
    playPause = off(blocker);
  }
  if (isSpotify && i.spotifyDevice?.isRestricted && !blocker) {
    blocker = `${i.spotifyDevice.name} does not accept remote commands`;
    playPause = off(blocker);
  }

  const hasDuration = caps.duration && i.durationMs != null && i.durationMs > 0;
  let seek: ControlState = caps.seek ? (hasDuration ? on : off("Duration unknown")) : off("This source cannot seek");
  if (isSpotify && disallows.includes("seeking")) seek = off("Spotify does not allow seeking here");
  if (blocker) seek = off(blocker);

  let volume: ControlState = caps.volume ? on : off("This source has no volume control");
  if (isSpotify && caps.volume) {
    volume = i.spotifyDevice?.supportsVolume ? on : off("The selected Spotify device does not support remote volume");
  }

  return {
    playPause,
    ...queueNav(i, caps),
    seek,
    volume,
    deviceSelection: caps.deviceSelection ? on : off("This source plays inside the app"),
    showPosition: caps.playbackPosition,
    showDuration: caps.duration,
    blocker,
  };
}

function queueNav(i: ControlInput, caps: PlaybackCapabilities | null): Pick<ResolvedControls, "next" | "previous"> {
  // Next/previous move through the unified app queue, so they work even when
  // the current track's provider is unavailable (that's how you skip it).
  const idx = i.currentIndex ?? -1;
  const hasNext = idx + 1 < i.queueLength || (i.repeatMode === "all" && i.queueLength > 0);
  const hasPrev = idx > 0 || (i.repeatMode === "all" && i.queueLength > 1);
  const canRestart = !!caps?.seek && i.positionMs > 3000;
  return {
    next: hasNext ? on : off("End of queue"),
    previous: hasPrev || canRestart ? on : off("Start of queue"),
  };
}
