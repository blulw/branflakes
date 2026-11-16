// Playback controller. Owns the unified queue (persisted in SQLite via the
// backend) and guarantees a single active engine at any time.
import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { api, toApiError } from "@/lib/api";
import { artworkForSystem } from "@/lib/format";
import type {
  PlaybackCapabilities,
  ProviderStatus,
  QueueSnapshot,
  RepeatMode,
  Source,
  SpotifyDevice,
  SpotifyPlaybackState,
  Track,
} from "@/lib/types";
import { resolveControls, type ResolvedControls } from "./controls";
import { AudioElementEngine } from "./engines/audioElement";
import { SpotifyConnectEngine } from "./engines/spotifyConnect";
import type { EngineCallbacks, PlaybackEngine } from "./engines/types";
import { activateSdkElement, onSdkDeviceLost, sdkPreferred, startSdkDevice } from "./engines/spotifySdk";

export type PlayerStatus = "idle" | "loading" | "playing" | "paused" | "error";

interface ScrobbleSession {
  trackId: string;
  source: Source;
  startedAt: number; // unix seconds
  playedMs: number;
  lastPos: number;
}

export interface PlayerState {
  queue: Track[];
  currentIndex: number | null;
  repeatMode: RepeatMode;
  shuffle: boolean;
  volume: number;
  status: PlayerStatus;
  positionMs: number;
  durationMs: number | null;
  error: string | null;
  loadedTrackId: string | null;
  providers: Partial<Record<string, ProviderStatus>>;
  spotifyDeviceId: string | null;
  spotifyDevices: SpotifyDevice[];
  spotifyState: SpotifyPlaybackState | null;
  sdkDeviceId: string | null;

  init(): Promise<void>;
  refreshProviders(): Promise<void>;
  applySnapshot(s: QueueSnapshot): void;
  playTracks(tracks: Track[], startIndex: number): Promise<void>;
  playAt(index: number): Promise<void>;
  enqueue(tracks: Track[], playNext: boolean): Promise<void>;
  removeAt(position: number): Promise<void>;
  move(from: number, to: number): Promise<void>;
  clearQueue(): Promise<void>;
  togglePlay(): Promise<void>;
  next(userInitiated?: boolean): Promise<void>;
  previous(): Promise<void>;
  seek(ms: number): Promise<void>;
  setVolume(v: number): Promise<void>;
  cycleRepeat(): void;
  shuffleUpcoming(): Promise<void>;
  refreshSpotifyDevices(): Promise<void>;
  selectSpotifyDevice(id: string | null): Promise<void>;
  setSdkDevice(id: string | null): void;
  /** Start the Web Playback SDK device. `automatic` = restore on startup (silent unless it fails). */
  startInAppSpotify(automatic: boolean): Promise<boolean>;
  controls(): ResolvedControls;
  currentTrack(): Track | null;
}

let scrobble: ScrobbleSession | null = null;
let activeEngine: PlaybackEngine | null = null;
let lastPersist = 0;
let lastMediaPush = 0;
let consecutiveSkips = 0;
let volumeTimer: number | null = null;
let lastMediaFlags = "";
let usePlayer_subscribe: (() => () => void) | null = null;

/** Scopes the Web Playback SDK checks. Other missing scopes (e.g. playlist sync) don't affect it. */
export const SDK_SCOPES = ["streaming", "user-read-email", "user-read-private"];

export function sdkScopesMissing(sp: Pick<ProviderStatus, "missingScopes">): boolean {
  return (sp.missingScopes ?? []).some((s) => SDK_SCOPES.includes(s));
}

/** Which transport actions the active source allows right now (taskbar toolbar). */
export function mediaFlags(s: Pick<PlayerState, "controls" | "status">) {
  const c = s.controls();
  return { canPrevious: c.previous.enabled, canPlayPause: c.playPause.enabled, canNext: c.next.enabled, playing: s.status === "playing" };
}

function mediaFlagsKey(f: ReturnType<typeof mediaFlags>): string {
  return `${+f.canPrevious}${+f.canPlayPause}${+f.canNext}${+f.playing}`;
}

const NO_CAPS: PlaybackCapabilities = {
  play: false,
  pause: false,
  next: false,
  previous: false,
  seek: false,
  volume: false,
  queue: false,
  deviceSelection: false,
  playbackPosition: false,
  duration: false,
};

export const usePlayer = create<PlayerState>((set, get) => {
  const callbacks: EngineCallbacks = {
    onTime(pos) {
      const s = get();
      if (scrobble && s.status === "playing") {
        const delta = pos - scrobble.lastPos;
        // Count only real elapsed listening time, not seeks.
        if (delta > 0 && delta < 2500) scrobble.playedMs += delta;
        scrobble.lastPos = pos;
      } else if (scrobble) {
        scrobble.lastPos = pos;
      }
      set({ positionMs: pos });
      const now = Date.now();
      if (now - lastPersist > 5000) {
        lastPersist = now;
        void persist();
      }
      if (now - lastMediaPush > 10000) {
        lastMediaPush = now;
        pushMedia();
      }
    },
    onDuration(d) {
      set({ durationMs: d ?? get().currentTrack()?.durationMs ?? null });
    },
    onPlaying(playing) {
      const st = get().status;
      if (playing && st !== "playing") consecutiveSkips = 0;
      set({ status: playing ? "playing" : st === "loading" ? "loading" : "paused" });
      pushMedia();
    },
    onEnded() {
      void finalizeScrobble();
      void get().next(false);
    },
    onError(message) {
      set({ status: "error", error: message });
      toast.error(message, { description: get().currentTrack()?.title });
      pushMedia();
    },
  };

  const audioEngine = new AudioElementEngine(callbacks);
  const spotifyEngine = new SpotifyConnectEngine(callbacks, {
    getDeviceId: () => get().spotifyDeviceId,
    onState: (state) => set({ spotifyState: state }),
  });

  const engineFor = (source: Source): PlaybackEngine => (source === "spotify" ? spotifyEngine : audioEngine);

  async function persist() {
    const s = get();
    try {
      await api.savePlaybackState({
        currentIndex: s.currentIndex,
        positionMs: Math.round(s.positionMs),
        volume: s.volume,
        shuffle: s.shuffle,
        repeatMode: s.repeatMode,
        spotifyDeviceId: s.spotifyDeviceId,
      });
    } catch {
      /* non-fatal */
    }
  }

  function pushMedia() {
    const s = get();
    const t = s.currentTrack();
    const flags = mediaFlags(s);
    lastMediaFlags = mediaFlagsKey(flags);
    if (!t) {
      void api.mediaUpdate({ status: "stopped", ...flags }).catch(() => undefined);
      return;
    }
    void api
      .mediaUpdate({
        title: t.title,
        artist: t.artist,
        album: t.album,
        coverUrl: artworkForSystem(t.artwork),
        durationMs: s.durationMs ?? t.durationMs ?? undefined,
        status: s.status === "playing" ? "playing" : s.status === "paused" ? "paused" : "stopped",
        positionMs: Math.round(s.positionMs),
        ...flags,
      })
      .catch(() => undefined);
  }

  // Queue edits, provider status or Spotify device changes can enable or
  // disable previous/next without a track change. Push only when the
  // resolved flags actually change (no polling).
  usePlayer_subscribe = () =>
    usePlayer.subscribe((s) => {
      const key = mediaFlagsKey(mediaFlags(s));
      if (key !== lastMediaFlags) pushMedia();
    });

  async function finalizeScrobble() {
    const sess = scrobble;
    scrobble = null;
    // The backend applies the source policy (including the "Scrobble Spotify
    // tracks" setting) and the Last.fm eligibility rules.
    if (!sess || sess.playedMs < 1000) return;
    try {
      await api.lastfmSubmitPlay(sess.trackId, sess.startedAt, Math.round(sess.playedMs));
    } catch {
      /* the backend logs; scrobbling must never break playback */
    }
  }

  async function startCurrent(autoplay: boolean, startMs = 0) {
    const s = get();
    const idx = s.currentIndex;
    const track = idx != null ? s.queue[idx] : null;
    await finalizeScrobble();
    if (!track) {
      await activeEngine?.release();
      activeEngine = null;
      set({ status: "idle", loadedTrackId: null, positionMs: 0 });
      pushMedia();
      return;
    }
    const caps = s.providers[track.source]?.capabilities ?? NO_CAPS;
    if (track.availability === "unavailable" || !caps.play) {
      const why = track.availability === "unavailable" ? track.unavailableReason ?? "Track unavailable" : `${track.source} is not connected`;
      set({ status: "error", error: why, loadedTrackId: null, positionMs: 0, durationMs: track.durationMs });
      if (autoplay && consecutiveSkips < s.queue.length) {
        consecutiveSkips++;
        toast.warning(`Skipped “${track.title}”`, { description: why });
        await get().next(false);
      }
      return;
    }
    const engine = engineFor(track.source);
    if (activeEngine && activeEngine !== engine) {
      await activeEngine.release();
    }
    activeEngine = engine;
    set({ status: autoplay ? "loading" : "paused", error: null, loadedTrackId: track.id, positionMs: startMs, durationMs: track.durationMs });
    try {
      if (engine === audioEngine) await engine.setVolume(s.volume);
      await engine.load(track, startMs, autoplay);
      if (autoplay) {
        scrobble = { trackId: track.id, source: track.source, startedAt: Math.floor(Date.now() / 1000), playedMs: 0, lastPos: startMs };
        void api.lastfmNowPlaying(track.id).catch(() => undefined);
      }
      pushMedia();
    } catch (e) {
      const err = toApiError(e);
      set({ status: "error", error: err.message, loadedTrackId: null });
      toast.error(`Could not play “${track.title}”`, { description: err.message });
      if (err.kind === "premium_required" || err.kind === "not_connected" || err.kind === "auth") void get().refreshProviders();
    }
  }

  return {
    queue: [],
    currentIndex: null,
    repeatMode: "off",
    shuffle: false,
    volume: 0.8,
    status: "idle",
    positionMs: 0,
    durationMs: null,
    error: null,
    loadedTrackId: null,
    providers: {},
    spotifyDeviceId: null,
    spotifyDevices: [],
    spotifyState: null,
    sdkDeviceId: null,

    async init() {
      await get().refreshProviders();
      onSdkDeviceLost((reason) => {
        const s = get();
        set({ sdkDeviceId: null, spotifyDeviceId: s.spotifyDeviceId === s.sdkDeviceId ? null : s.spotifyDeviceId });
        toast.error("In-app Spotify device stopped", { description: reason, duration: 10000 });
        void get().refreshSpotifyDevices();
      });
      void get().startInAppSpotify(true);
      const snap = await api.getQueue();
      get().applySnapshot(snap);
      set({
        volume: snap.state.volume,
        repeatMode: snap.state.repeatMode,
        shuffle: snap.state.shuffle,
        positionMs: snap.state.positionMs,
        spotifyDeviceId: snap.state.spotifyDeviceId,
        durationMs: snap.state.currentIndex != null ? snap.items[snap.state.currentIndex]?.durationMs ?? null : null,
      });
      await listen<{ action: string; positionMs?: number }>("media-control", (e) => {
        const c = get().controls();
        switch (e.payload.action) {
          case "play":
          case "pause":
          case "toggle":
            if (c.playPause.enabled) void get().togglePlay();
            break;
          case "next":
            if (c.next.enabled) void get().next(true);
            break;
          case "previous":
            if (c.previous.enabled) void get().previous();
            break;
          case "stop":
            if (get().status === "playing") void activeEngine?.pause();
            break;
          case "setPosition":
            if (c.seek.enabled && e.payload.positionMs != null) void get().seek(e.payload.positionMs);
            break;
          case "seekForward":
            if (c.seek.enabled) void get().seek(get().positionMs + 10000);
            break;
          case "seekBackward":
            if (c.seek.enabled) void get().seek(Math.max(0, get().positionMs - 10000));
            break;
        }
      });
      usePlayer_subscribe?.();
      pushMedia();
      window.addEventListener("beforeunload", () => {
        void persist();
      });
    },

    async refreshProviders() {
      try {
        const list = await api.providersStatus();
        set({ providers: Object.fromEntries(list.map((p) => [p.provider, p])) });
        if (list.find((p) => p.provider === "spotify")?.connected) void get().refreshSpotifyDevices();
      } catch (e) {
        toast.error("Could not read provider status", { description: toApiError(e).message });
      }
    },

    applySnapshot(s) {
      set({ queue: s.items, currentIndex: s.state.currentIndex });
    },

    async playTracks(tracks, startIndex) {
      if (!tracks.length) return;
      activateSdkElement();
      consecutiveSkips = 0;
      const snap = await api.replaceQueue(
        tracks.map((t) => t.id),
        startIndex,
      );
      get().applySnapshot(snap);
      await startCurrent(true);
    },

    async playAt(index) {
      activateSdkElement();
      consecutiveSkips = 0;
      const snap = await api.setQueueCurrent(index);
      get().applySnapshot(snap);
      await startCurrent(true);
    },

    async enqueue(tracks, playNext) {
      const snap = await api.addToQueue(
        tracks.map((t) => t.id),
        playNext,
      );
      get().applySnapshot(snap);
      toast.success(playNext ? "Playing next" : "Added to queue", { description: tracks.length === 1 ? tracks[0].title : `${tracks.length} tracks` });
    },

    async removeAt(position) {
      const wasCurrent = position === get().currentIndex;
      const snap = await api.removeFromQueue(position);
      get().applySnapshot(snap);
      if (wasCurrent) await startCurrent(get().status === "playing");
    },

    async move(from, to) {
      get().applySnapshot(await api.moveInQueue(from, to));
    },

    async clearQueue() {
      get().applySnapshot(await api.clearQueue());
      await startCurrent(false);
    },

    async togglePlay() {
      activateSdkElement();
      const s = get();
      const t = s.currentTrack();
      if (!t) return;
      if (s.loadedTrackId !== t.id || !activeEngine || s.status === "error") {
        await startCurrent(true, s.loadedTrackId == null ? s.positionMs : 0);
        return;
      }
      try {
        if (s.status === "playing") {
          await activeEngine.pause();
        } else {
          await activeEngine.play();
          if (!scrobble) scrobble = { trackId: t.id, source: t.source, startedAt: Math.floor(Date.now() / 1000), playedMs: 0, lastPos: s.positionMs };
        }
      } catch (e) {
        const err = toApiError(e);
        toast.error(err.message);
        if (err.kind === "premium_required") void get().refreshProviders();
      }
    },

    async next(userInitiated = true) {
      if (userInitiated) await finalizeScrobble();
      const snap = await api.advanceQueue("next", userInitiated);
      if (snap.state.currentIndex == null) {
        // End of queue.
        await activeEngine?.release();
        activeEngine = null;
        set({ queue: snap.items, status: "idle", loadedTrackId: null, positionMs: 0 });
        pushMedia();
        return;
      }
      get().applySnapshot(snap);
      await startCurrent(true);
    },

    async previous() {
      const s = get();
      const caps = s.providers[s.currentTrack()?.source ?? "local"]?.capabilities;
      if (s.positionMs > 3000 && caps?.seek && activeEngine && s.loadedTrackId) {
        await get().seek(0);
        return;
      }
      await finalizeScrobble();
      get().applySnapshot(await api.advanceQueue("previous", true));
      await startCurrent(true);
    },

    async seek(ms) {
      if (!activeEngine || !get().controls().seek.enabled) return;
      const d = get().durationMs;
      const target = Math.max(0, d ? Math.min(ms, d - 500) : ms);
      try {
        await activeEngine.seek(target);
        if (scrobble) scrobble.lastPos = target;
        set({ positionMs: target });
        pushMedia();
      } catch (e) {
        toast.error(toApiError(e).message);
      }
    },

    async setVolume(v) {
      const vol = Math.min(1, Math.max(0, v));
      set({ volume: vol });
      const engine = activeEngine;
      if (engine && get().controls().volume.enabled) {
        const apply = async () => {
          try {
            await engine.setVolume(vol);
          } catch (e) {
            toast.error(toApiError(e).message);
          }
        };
        if (engine === spotifyEngine) {
          // Each Spotify volume change is a Web API call: debounce slider drags.
          if (volumeTimer != null) window.clearTimeout(volumeTimer);
          volumeTimer = window.setTimeout(() => void apply(), 350);
        } else {
          await apply();
        }
      }
      void persist();
    },

    cycleRepeat() {
      const order: RepeatMode[] = ["off", "all", "one"];
      set({ repeatMode: order[(order.indexOf(get().repeatMode) + 1) % order.length] });
      void persist();
    },

    async shuffleUpcoming() {
      get().applySnapshot(await api.shuffleUpcoming());
      set({ shuffle: true });
    },

    async refreshSpotifyDevices() {
      try {
        let devices = await api.spotifyDevices();
        const s = get();
        // Spotify's device list can lag behind the SDK's "ready" event; keep
        // the in-app device available instead of dropping the selection.
        if (s.sdkDeviceId && !devices.some((d) => d.id === s.sdkDeviceId)) {
          devices = [
            ...devices,
            { id: s.sdkDeviceId, name: "branflakes (this PC)", type: "Computer", isActive: false, isRestricted: false, supportsVolume: true, volumePercent: null },
          ];
        }
        let id = s.spotifyDeviceId;
        if (id && !devices.some((d) => d.id === id)) id = null;
        if (!id) id = devices.find((d) => d.isActive && !d.isRestricted)?.id ?? s.sdkDeviceId ?? null;
        set({ spotifyDevices: devices, spotifyDeviceId: id });
      } catch {
        /* not fatal: shown as "no devices" */
      }
    },

    async selectSpotifyDevice(id) {
      set({ spotifyDeviceId: id });
      void persist();
      const s = get();
      // If a Spotify track is loaded, move playback to the new device.
      if (id && activeEngine === spotifyEngine && s.status === "playing") {
        try {
          await api.spotifyControl("transfer", id, 1);
        } catch (e) {
          toast.error(toApiError(e).message);
        }
      }
    },

    async startInAppSpotify(automatic) {
      const sp = get().providers.spotify;
      if (!sp?.connected) {
        if (!automatic) toast.error("Connect Spotify in Settings first");
        return false;
      }
      // On startup, only restore the device if the user enabled it before and
      // the sign-in already has the required permissions.
      if (automatic && (!sdkPreferred() || sdkScopesMissing(sp))) return false;
      const r = await startSdkDevice();
      if (r.ok) {
        get().setSdkDevice(r.deviceId);
        if (!automatic) toast.success("In-app Spotify device is ready");
        return true;
      }
      toast.error("In-app Spotify device unavailable", { description: r.reason, duration: 12000 });
      return false;
    },

    setSdkDevice(id) {
      set({ sdkDeviceId: id });
      if (id) {
        set({ spotifyDeviceId: id });
        void get().refreshSpotifyDevices();
      }
    },

    currentTrack() {
      const { queue, currentIndex } = get();
      return currentIndex != null ? queue[currentIndex] ?? null : null;
    },

    controls() {
      const s = get();
      const t = s.currentTrack();
      const device = s.spotifyDevices.find((d) => d.id === s.spotifyDeviceId) ?? null;
      return resolveControls({
        track: t,
        caps: t ? s.providers[t.source]?.capabilities ?? null : null,
        queueLength: s.queue.length,
        currentIndex: s.currentIndex,
        repeatMode: s.repeatMode,
        positionMs: s.positionMs,
        durationMs: s.durationMs,
        spotifyDevice: device,
        spotifyDisallows: s.spotifyState?.disallows,
      });
    },
  };
});
