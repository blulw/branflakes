// Spotify playback through Spotify Connect remote control (official Web API
// /me/player endpoints). Audio plays on the selected Connect device; this app
// never receives or decodes Spotify audio. Position is polled and
// interpolated between polls, so it is approximate.
import { api } from "@/lib/api";
import type { Source, SpotifyPlaybackState, Track } from "@/lib/types";
import type { EngineCallbacks, PlaybackEngine } from "./types";

const POLL_MS = 1500;
/** How long after our own play command an unexpected track URI is treated as
 *  Spotify relinking (same song, market-specific copy) rather than a takeover. */
const ADOPT_WINDOW_MS = 6000;

export interface SpotifyConnectHooks {
  getDeviceId(): string | null;
  onState(state: SpotifyPlaybackState | null): void;
}

export class SpotifyConnectEngine implements PlaybackEngine {
  readonly sources: readonly Source[] = ["spotify"];
  private timer: number | null = null;
  private tick: number | null = null;
  private uri: string | null = null;
  private trackId: string | null = null;
  private durationMs: number | null = null;
  private lastProgress = 0;
  private lastPollAt = 0;
  private playing = false;
  private nearEnd = false;
  private active = false;
  /** Until this time (performance.now), a different track Spotify reports is
   *  assumed to be a relinked copy of the one we just asked for. */
  private adoptUntil = 0;
  /** The track that was loaded before this one (Spotify may still report it briefly). */
  private prevUri: string | null = null;

  constructor(
    private cb: EngineCallbacks,
    private hooks: SpotifyConnectHooks,
  ) {}

  async load(track: Track, startMs: number, autoplay: boolean) {
    this.active = true;
    this.trackId = track.id;
    this.prevUri = this.uri;
    this.adoptUntil = 0;
    this.uri = track.providerUri ?? `spotify:track:${track.providerId}`;
    this.durationMs = track.durationMs;
    this.cb.onDuration(track.durationMs);
    this.lastProgress = startMs;
    this.nearEnd = false;
    this.cb.onTime(startMs);
    if (autoplay) {
      await api.spotifyPlayTrack(track.id, this.hooks.getDeviceId(), startMs);
      this.adoptUntil = performance.now() + ADOPT_WINDOW_MS;
      this.playing = true;
      this.cb.onPlaying(true);
    }
    this.startPolling();
  }

  private startPolling() {
    this.stopPolling();
    this.lastPollAt = performance.now();
    this.timer = window.setInterval(() => void this.poll(), POLL_MS);
    // Smooth the progress bar between polls.
    this.tick = window.setInterval(() => {
      if (!this.playing) return;
      const est = this.lastProgress + (performance.now() - this.lastPollAt);
      this.cb.onTime(this.durationMs ? Math.min(est, this.durationMs) : est);
    }, 250);
  }

  private stopPolling() {
    if (this.timer != null) window.clearInterval(this.timer);
    if (this.tick != null) window.clearInterval(this.tick);
    this.timer = this.tick = null;
  }

  private async poll() {
    if (!this.active) return;
    let s: SpotifyPlaybackState | null;
    try {
      s = await api.spotifyPlaybackState();
    } catch (e) {
      // Transient errors (rate limits, network) shouldn't stop playback tracking.
      return;
    }
    if (!this.active) return;
    this.hooks.onState(s);
    // Spotify may relink the requested track to a market-specific copy with a
    // different URI. Accept it via `linked_from`, or, if Spotify omits that,
    // when it shows up right after our own play command.
    if (s?.itemUri && s.itemUri !== this.uri) {
      const fresh = performance.now() < this.adoptUntil && s.itemUri !== this.prevUri;
      if (s.linkedFromUri === this.uri || fresh) this.uri = s.itemUri;
    }
    if (s?.itemUri === this.uri) this.adoptUntil = 0;
    const ours = s?.itemUri === this.uri;
    const progress = s?.progressMs ?? 0;
    // End detection: we were near the end of our track and now Spotify has
    // moved on (different item, rewound, or stopped).
    if (this.nearEnd && (!ours || progress < this.lastProgress || !s?.isPlaying)) {
      this.nearEnd = false;
      this.playing = false;
      this.active = false;
      this.stopPolling();
      this.cb.onEnded();
      return;
    }
    if (ours && s) {
      this.lastProgress = progress;
      this.lastPollAt = performance.now();
      if (s.itemDurationMs) this.durationMs = s.itemDurationMs;
      this.nearEnd = !!this.durationMs && progress >= this.durationMs - POLL_MS * 2;
      if (s.isPlaying !== this.playing) {
        this.playing = s.isPlaying;
        this.cb.onPlaying(s.isPlaying);
      }
      this.cb.onTime(progress);
    } else if (s && !ours && this.playing) {
      // Something else took over the Spotify session (e.g. the user picked a
      // song on their phone). Stop claiming ownership in this app.
      this.playing = false;
      this.cb.onPlaying(false);
    }
  }

  async play() {
    if (!this.playing && this.uri) {
      // Resume if our track is still loaded on the device, otherwise restart it.
      const s = await api.spotifyPlaybackState().catch(() => null);
      if (s?.itemUri === this.uri) {
        await api.spotifyControl("resume", this.hooks.getDeviceId());
      } else if (this.trackId) {
        await api.spotifyPlayTrack(this.trackId, this.hooks.getDeviceId(), Math.round(this.lastProgress));
        this.adoptUntil = performance.now() + ADOPT_WINDOW_MS;
      }
      this.active = true;
      if (this.timer == null) this.startPolling();
      this.playing = true;
      this.lastPollAt = performance.now();
      this.cb.onPlaying(true);
    }
  }

  async pause() {
    await api.spotifyControl("pause", this.hooks.getDeviceId());
    this.playing = false;
    this.cb.onPlaying(false);
  }

  async seek(ms: number) {
    await api.spotifyControl("seek", this.hooks.getDeviceId(), Math.round(ms));
    this.lastProgress = ms;
    this.lastPollAt = performance.now();
    this.cb.onTime(ms);
  }

  async setVolume(v: number) {
    await api.spotifyControl("volume", this.hooks.getDeviceId(), Math.round(v * 100));
  }

  async release() {
    const wasPlaying = this.playing;
    this.active = false;
    this.playing = false;
    this.stopPolling();
    if (wasPlaying) {
      // Single-owner rule: when another source takes over, pause Spotify.
      await api.spotifyControl("pause", this.hooks.getDeviceId()).catch(() => undefined);
    }
  }
}
