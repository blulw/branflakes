// Optional in-app Spotify Connect device using Spotify's official Web
// Playback SDK. It needs Widevine DRM in WebView2 and a token carrying the
// scopes the SDK checks (streaming, user-read-email, user-read-private).
import { api, toApiError } from "@/lib/api";

const SDK_URL = "https://sdk.scdn.co/spotify-player.js";
const ENABLED_KEY = "unified-music.spotify-sdk-enabled";

export type SdkResult = { ok: true; deviceId: string } | { ok: false; reason: string };

interface SpotifyPlayer {
  connect(): Promise<boolean>;
  disconnect(): void;
  activateElement?(): Promise<void>;
  addListener(event: string, cb: (payload: any) => void): void;
}

declare global {
  interface Window {
    onSpotifyWebPlaybackSDKReady?: () => void;
    Spotify?: { Player: new (opts: { name: string; getOAuthToken: (cb: (t: string) => void) => void; volume?: number }) => SpotifyPlayer };
  }
}

let player: SpotifyPlayer | null = null;
let current: Promise<SdkResult> | null = null;
let onLost: ((reason: string) => void) | null = null;

/** Whether the user turned the in-app device on (remembered across restarts). */
export function sdkPreferred(): boolean {
  try {
    return localStorage.getItem(ENABLED_KEY) === "1";
  } catch {
    return false;
  }
}

function setPreferred(on: boolean) {
  try {
    if (on) localStorage.setItem(ENABLED_KEY, "1");
    else localStorage.removeItem(ENABLED_KEY);
  } catch {
    /* storage unavailable: just don't remember */
  }
}

/** Called if the device goes offline after it was ready. */
export function onSdkDeviceLost(cb: (reason: string) => void) {
  onLost = cb;
}

export async function widevineAvailable(): Promise<boolean> {
  if (!navigator.requestMediaKeySystemAccess) return false;
  try {
    await navigator.requestMediaKeySystemAccess("com.widevine.alpha", [
      { initDataTypes: ["cenc"], audioCapabilities: [{ contentType: 'audio/mp4; codecs="mp4a.40.2"' }] },
    ]);
    return true;
  } catch {
    return false;
  }
}

function loadScript(): Promise<void> {
  return new Promise((resolve, reject) => {
    if (window.Spotify) return resolve();
    window.onSpotifyWebPlaybackSDKReady = () => resolve();
    const s = document.createElement("script");
    s.src = SDK_URL;
    s.async = true;
    s.onerror = () => reject(new Error("Could not load the Spotify Web Playback SDK (check your internet connection)"));
    document.head.appendChild(s);
  });
}

function describeAuthError(message: string): string {
  if (/scope/i.test(message)) {
    return "Your Spotify sign-in doesn't include the permissions the in-app player needs. Go to Settings → Accounts and click Reconnect Spotify.";
  }
  return `Spotify rejected the sign-in: ${message}. Try Settings → Accounts → Reconnect Spotify.`;
}

export function startSdkDevice(): Promise<SdkResult> {
  if (current) return current;
  current = (async (): Promise<SdkResult> => {
    if (!(await widevineAvailable())) {
      return {
        ok: false,
        reason:
          "This WebView2 runtime does not expose Widevine DRM, which the Spotify Web Playback SDK requires. Use another Spotify Connect device instead.",
      };
    }
    // Validate the token (and its scopes) before involving the SDK, so the
    // user gets a precise, actionable error instead of a generic SDK failure.
    try {
      await api.spotifySdkToken();
    } catch (e) {
      return { ok: false, reason: toApiError(e).message };
    }
    try {
      await loadScript();
    } catch (e) {
      return { ok: false, reason: (e as Error).message };
    }
    if (!window.Spotify) return { ok: false, reason: "The Spotify SDK failed to initialise" };
    const p = new window.Spotify.Player({
      name: "branflakes (this PC)",
      volume: 0.8,
      getOAuthToken: (cb) => {
        api.spotifySdkToken().then(cb, (e) => onLost?.(toApiError(e).message));
      },
    });
    player = p;
    return new Promise<SdkResult>((resolve) => {
      let settled = false;
      const done = (r: SdkResult) => {
        if (!settled) {
          settled = true;
          resolve(r);
        }
      };
      p.addListener("ready", ({ device_id }: { device_id: string }) => done({ ok: true, deviceId: device_id }));
      p.addListener("not_ready", () => {
        if (settled) onLost?.("The in-app Spotify device went offline");
      });
      p.addListener("initialization_error", ({ message }: { message: string }) => done({ ok: false, reason: `The Spotify SDK could not start: ${message}` }));
      p.addListener("authentication_error", ({ message }: { message: string }) => {
        const reason = describeAuthError(message);
        if (settled) onLost?.(reason);
        done({ ok: false, reason });
      });
      p.addListener("account_error", () => done({ ok: false, reason: "The in-app Spotify player requires Spotify Premium" }));
      setTimeout(() => done({ ok: false, reason: "Timed out waiting for the Spotify SDK device" }), 20000);
      p.connect().then((ok) => {
        if (!ok) done({ ok: false, reason: "The Spotify SDK could not connect" });
      });
    });
  })().then((r) => {
    if (r.ok) {
      setPreferred(true);
    } else {
      player?.disconnect();
      player = null;
      current = null;
    }
    return r;
  });
  return current;
}

/**
 * Must run synchronously inside a user gesture (click / media key) so that
 * Chromium's autoplay policy lets the SDK start audio.
 */
export function activateSdkElement() {
  void player?.activateElement?.().catch(() => undefined);
}

export function stopSdkDevice() {
  setPreferred(false);
  player?.disconnect();
  player = null;
  current = null;
}
