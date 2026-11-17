// In-app playback through a single HTMLAudioElement:
//  * Local files via Tauri's asset protocol (scope limited to library folders).
//  * SoundCloud via the documented /streams URLs (AAC HLS through hls.js/MSE).
import Hls from "hls.js";
import { convertFileSrc } from "@/lib/tauri";
import { api } from "@/lib/api";
import type { Source, Track } from "@/lib/types";
import type { EngineCallbacks, PlaybackEngine } from "./types";
import { LOUDNESS_WAIT_MS, UNKNOWN_GAIN, effectiveVolume, normalizationGain } from "../normalize";

const RAMP_MS = 500;

export class AudioElementEngine implements PlaybackEngine {
  readonly sources: readonly Source[] = ["local", "soundcloud"];
  private audio: HTMLAudioElement;
  private hls: Hls | null = null;
  private loadToken = 0;
  private userVolume = 1;
  private normalize = true;
  /** Per-track normalization gain currently applied (ramped). */
  private gain = 1;
  private current: Track | null = null;
  private ramp: number | null = null;

  constructor(private cb: EngineCallbacks) {
    this.audio = new Audio();
    this.audio.preload = "auto";
    this.audio.addEventListener("timeupdate", () => cb.onTime(this.audio.currentTime * 1000));
    this.audio.addEventListener("durationchange", () => {
      const d = this.audio.duration;
      cb.onDuration(Number.isFinite(d) && d > 0 ? d * 1000 : null);
    });
    this.audio.addEventListener("playing", () => cb.onPlaying(true));
    this.audio.addEventListener("pause", () => cb.onPlaying(false));
    this.audio.addEventListener("ended", () => cb.onEnded());
    this.audio.addEventListener("error", () => {
      if (!this.audio.src && !this.hls) return;
      const code = this.audio.error?.code;
      const msg =
        code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED
          ? "This file format or codec cannot be played"
          : code === MediaError.MEDIA_ERR_NETWORK
            ? "Network error while streaming"
            : code === MediaError.MEDIA_ERR_DECODE
              ? "The audio could not be decoded"
              : "Playback failed";
      cb.onError(msg);
    });
  }

  private resetSource() {
    if (this.ramp != null) cancelAnimationFrame(this.ramp);
    this.ramp = null;
    this.hls?.destroy();
    this.hls = null;
    this.audio.pause();
    this.audio.removeAttribute("src");
    this.audio.load();
  }

  private applyVolume() {
    this.audio.volume = effectiveVolume(this.userVolume, this.normalize ? this.gain : 1);
  }

  /** Move the track gain to `g`, smoothly unless `instant`. */
  private setGain(g: number, instant: boolean) {
    if (this.ramp != null) cancelAnimationFrame(this.ramp);
    this.ramp = null;
    if (instant || typeof requestAnimationFrame !== "function") {
      this.gain = g;
      this.applyVolume();
      return;
    }
    const from = this.gain;
    const start = performance.now();
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / RAMP_MS);
      this.gain = from + (g - from) * t;
      this.applyVolume();
      this.ramp = t < 1 ? requestAnimationFrame(step) : null;
    };
    this.ramp = requestAnimationFrame(step);
  }

  /** Turn loudness normalization on/off (affects the current track immediately). */
  setNormalize(on: boolean) {
    if (on === this.normalize) return;
    this.normalize = on;
    if (on && this.current) {
      // Mid-track: keep the current level until the measured gain arrives, then glide.
      this.setGain(1, true);
      void this.startLoudness(this.current, this.loadToken, false);
    } else {
      this.applyVolume();
    }
  }

  /** Look up the track's loudness; resolves once the gain is known (or the wait times out). */
  private startLoudness(track: Track, token: number, fresh = true): Promise<void> {
    if (fresh) this.setGain(UNKNOWN_GAIN, true);
    const lookup = api
      .trackLoudness(track.id)
      .then((lufs) => {
        if (token === this.loadToken) this.setGain(normalizationGain(lufs), false);
      })
      .catch(() => {
        if (token === this.loadToken) this.setGain(1, false);
      });
    return Promise.race([lookup, new Promise<void>((r) => setTimeout(r, LOUDNESS_WAIT_MS))]);
  }

  async load(track: Track, startMs: number, autoplay: boolean): Promise<void> {
    const token = ++this.loadToken;
    this.resetSource();
    this.current = track;
    // Gain known before the first sample when cached; otherwise ramps in once analysed.
    const loudness = this.normalize ? this.startLoudness(track, token) : (this.setGain(1, true), Promise.resolve());
    if (track.source === "local") {
      if (!track.filePath) throw new Error("Local track has no file path");
      this.audio.src = convertFileSrc(track.filePath);
    } else if (track.source === "soundcloud") {
      const stream = await api.soundcloudResolveStream(track.id);
      if (token !== this.loadToken) return; // superseded
      if (stream.protocol === "hls" && !this.audio.canPlayType("application/vnd.apple.mpegurl")) {
        if (!Hls.isSupported()) throw new Error("HLS playback is not supported by this WebView");
        const hls = new Hls({ enableWorker: true, lowLatencyMode: false });
        hls.on(Hls.Events.ERROR, (_e, data) => {
          if (data.fatal) this.cb.onError(`Stream error: ${data.details}`);
        });
        hls.loadSource(stream.url);
        hls.attachMedia(this.audio);
        this.hls = hls;
      } else {
        this.audio.src = stream.url;
      }
    } else {
      throw new Error(`AudioElementEngine cannot play ${track.source}`);
    }
    if (startMs > 0) {
      const seekTo = () => {
        this.audio.currentTime = startMs / 1000;
      };
      this.audio.addEventListener("loadedmetadata", seekTo, { once: true });
    }
    await loudness;
    if (token !== this.loadToken) return;
    if (this.ramp == null) this.applyVolume();
    if (autoplay) await this.play();
  }

  async play() {
    try {
      await this.audio.play();
    } catch (e) {
      // AbortError happens when a new load interrupts play(); not a real failure.
      if (e instanceof DOMException && e.name === "AbortError") return;
      throw e;
    }
  }
  async pause() {
    this.audio.pause();
  }
  async seek(ms: number) {
    this.audio.currentTime = Math.max(0, ms / 1000);
  }
  async setVolume(v: number) {
    this.userVolume = Math.min(1, Math.max(0, v));
    this.applyVolume();
  }
  async release() {
    this.loadToken++;
    this.current = null;
    this.resetSource();
  }
}
