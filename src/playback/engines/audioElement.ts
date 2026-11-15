// In-app playback through a single HTMLAudioElement:
//  * Local files via Tauri's asset protocol (scope limited to library folders).
//  * SoundCloud via the documented /streams URLs (AAC HLS through hls.js/MSE).
import Hls from "hls.js";
import { convertFileSrc } from "@/lib/tauri";
import { api } from "@/lib/api";
import type { Source, Track } from "@/lib/types";
import type { EngineCallbacks, PlaybackEngine } from "./types";

export class AudioElementEngine implements PlaybackEngine {
  readonly sources: readonly Source[] = ["local", "soundcloud"];
  private audio: HTMLAudioElement;
  private hls: Hls | null = null;
  private loadToken = 0;

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
    this.hls?.destroy();
    this.hls = null;
    this.audio.pause();
    this.audio.removeAttribute("src");
    this.audio.load();
  }

  async load(track: Track, startMs: number, autoplay: boolean): Promise<void> {
    const token = ++this.loadToken;
    this.resetSource();
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
    this.audio.volume = Math.min(1, Math.max(0, v));
  }
  async release() {
    this.loadToken++;
    this.resetSource();
  }
}
