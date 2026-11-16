// The Windows taskbar thumbnail toolbar (and SMTC) send `media-control`
// events; these must drive the existing controller, respect capabilities,
// keep the toolbar state current, and never raise a Windows notification.
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoked: { cmd: string; args: any }[] = [];
let mediaHandler: ((e: { payload: { action: string; positionMs?: number } }) => void) | null = null;
let queueIndex = 0;
let caps: Record<string, boolean>;

const track = (i: number) => ({
  id: `t${i}`,
  source: "local",
  providerId: `/m/${i}.mp3`,
  title: `Song ${i}`,
  artist: "A",
  album: "X",
  albumArtist: "",
  durationMs: 100_000,
  availability: "available",
  artwork: null,
  filePath: `C:/m/${i}.mp3`,
});
const items = [track(0), track(1), track(2)];
const snap = () => ({
  items,
  state: { currentIndex: queueIndex, positionMs: 0, volume: 0.5, shuffle: false, repeatMode: "off", spotifyDeviceId: null },
});

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (p: string) => `asset://${p}`,
  invoke: vi.fn(async (cmd: string, args: any) => {
    invoked.push({ cmd, args });
    switch (cmd) {
      case "providers_status":
        return [{ provider: "local", connected: true, capabilities: caps, limitations: [] }];
      case "queue_get":
      case "queue_replace":
      case "queue_set_current":
        return snap();
      case "queue_advance":
        queueIndex = args.direction === "next" ? Math.min(queueIndex + 1, items.length - 1) : Math.max(queueIndex - 1, 0);
        return snap();
      default:
        return null;
    }
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, cb: any) => {
    if (name === "media-control") mediaHandler = cb;
    return () => undefined;
  }),
}));

// jsdom has no media playback: a minimal audio element.
class FakeAudio extends EventTarget {
  src = "";
  currentTime = 0;
  duration = 100;
  volume = 1;
  paused = true;
  preload = "";
  crossOrigin: string | null = null;
  async play() {
    this.paused = false;
    this.dispatchEvent(new Event("playing"));
  }
  pause() {
    this.paused = true;
    this.dispatchEvent(new Event("pause"));
  }
  load() {}
  removeAttribute() {}
  setAttribute() {}
}
vi.stubGlobal("Audio", FakeAudio);
const origCreate = document.createElement.bind(document);
vi.spyOn(document, "createElement").mockImplementation(((tag: string, o?: any) =>
  tag === "audio" ? (new FakeAudio() as any) : origCreate(tag, o)) as any);

const mediaUpdates = () => invoked.filter((c) => c.cmd === "media_update").map((c) => c.args.update);

describe("taskbar thumbnail toolbar and SMTC controls", () => {
  beforeEach(() => {
    invoked.length = 0;
    queueIndex = 0;
    caps = { play: true, pause: true, next: true, previous: true, seek: true, volume: true, queue: true, deviceSelection: false, playbackPosition: true, duration: true };
  });

  it("drives the existing controller and keeps the toolbar state current", async () => {
    const { usePlayer, mediaFlags } = await import("./store");
    await usePlayer.getState().init();
    expect(mediaHandler).toBeTruthy();
    await usePlayer.getState().playTracks(items as any, 0);
    expect(usePlayer.getState().loadedTrackId).toBe("t0");

    // Toolbar state is pushed with the resolved capabilities.
    const last = mediaUpdates().at(-1);
    expect(last).toMatchObject({ canPlayPause: true, canNext: true, title: "Song 0" });
    expect(mediaFlags(usePlayer.getState()).canPlayPause).toBe(true);

    // "Next" button → the controller advances the unified queue.
    mediaHandler!({ payload: { action: "next" } });
    await vi.waitFor(() => expect(usePlayer.getState().loadedTrackId).toBe("t1"));
    expect(invoked.some((c) => c.cmd === "queue_advance" && c.args.direction === "next")).toBe(true);
    expect(mediaUpdates().at(-1)).toMatchObject({ title: "Song 1" });

    // "Previous" button.
    mediaHandler!({ payload: { action: "previous" } });
    await vi.waitFor(() => expect(usePlayer.getState().loadedTrackId).toBe("t0"));

    // Play/Pause button → toggles, and the toolbar shows the new state.
    await vi.waitFor(() => expect(usePlayer.getState().status).toBe("playing"));
    mediaHandler!({ payload: { action: "toggle" } });
    await vi.waitFor(() => expect(usePlayer.getState().status).toBe("paused"));
    await vi.waitFor(() => expect(mediaUpdates().at(-1)?.status).toBe("paused"));
  });

  it("disables and ignores actions that are not possible right now", async () => {
    const { usePlayer } = await import("./store");
    // Next/previous follow the unified app queue (by design, independent of
    // the provider): at the end of the queue Next is unavailable.
    queueIndex = 2;
    await usePlayer.getState().playAt(2);
    await vi.waitFor(() => expect(mediaUpdates().at(-1)).toMatchObject({ canNext: false, title: "Song 2" }));
    const before = invoked.filter((c) => c.cmd === "queue_advance").length;
    mediaHandler!({ payload: { action: "next" } });
    await new Promise((r) => setTimeout(r, 20));
    expect(invoked.filter((c) => c.cmd === "queue_advance").length).toBe(before);
    expect(usePlayer.getState().loadedTrackId).toBe("t2");

    // A source that can't play (e.g. disconnected provider): Play/Pause is
    // disabled on the toolbar and a press is ignored.
    caps = { ...caps, play: false, pause: false };
    await usePlayer.getState().refreshProviders();
    await vi.waitFor(() => expect(mediaUpdates().at(-1)).toMatchObject({ canPlayPause: false }));
    const status = usePlayer.getState().status;
    mediaHandler!({ payload: { action: "toggle" } });
    await new Promise((r) => setTimeout(r, 20));
    expect(usePlayer.getState().status).toBe(status);
  });

  it("restores the in-app Spotify device unless its own scopes are missing", async () => {
    const { sdkScopesMissing } = await import("./store");
    // A pre-sync sign-in lacks only the playlist scopes: the device still works.
    expect(sdkScopesMissing({ missingScopes: ["playlist-modify-public", "playlist-modify-private", "ugc-image-upload"] })).toBe(false);
    expect(sdkScopesMissing({ missingScopes: ["streaming"] })).toBe(true);
    expect(sdkScopesMissing({ missingScopes: [] })).toBe(false);
  });

  it("never shows a Windows playback notification", async () => {
    // The notification plugin is no longer a dependency at all.
    const mods = import.meta.glob("/src/**/*.{ts,tsx}", { query: "?raw", import: "default", eager: true }) as Record<string, string>;
    const offenders = Object.entries(mods)
      .filter(([path]) => !path.endsWith(".test.ts"))
      .filter(([, src]) => src.includes("plugin-notification") || src.includes("sendNotification"));
    expect(offenders.map(([p]) => p)).toEqual([]);
    // Track changes, play/pause and next/previous issue no notification commands.
    const { usePlayer } = await import("./store");
    await usePlayer.getState().playTracks(items as any, 0);
    mediaHandler!({ payload: { action: "next" } });
    mediaHandler!({ payload: { action: "toggle" } });
    await new Promise((r) => setTimeout(r, 20));
    expect(invoked.some((c) => c.cmd.includes("notification"))).toBe(false);
  });
});
