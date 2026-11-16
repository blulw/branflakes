// Repro: Spotify Connect track plays to the end -> the next queue item must start
// and the position must keep moving.
import { beforeEach, describe, expect, it, vi } from "vitest";

const calls: string[] = [];
let devicePlaying: { uri: string; startedAt: number; startMs: number; durationMs: number } | null = null;
let queueIndex = 0;

const track = (i: number) => ({
  id: `t${i}`,
  source: "spotify",
  providerId: `p${i}`,
  providerUri: `spotify:track:p${i}`,
  title: `Song ${i}`,
  artist: "A",
  album: "X",
  albumArtist: "",
  durationMs: 10_000,
  availability: "available",
  artwork: null,
});
const items = [track(0), track(1), track(2)];
const snap = () => ({
  items,
  state: { currentIndex: queueIndex, positionMs: 0, volume: 0.5, shuffle: false, repeatMode: "off", spotifyDeviceId: "dev" },
});
const caps = { play: true, pause: true, next: true, previous: true, seek: true, volume: true, queue: true, deviceSelection: true, playbackPosition: true, duration: true };

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args: any) => {
    calls.push(cmd);
    switch (cmd) {
      case "providers_status":
        return [{ provider: "spotify", connected: true, capabilities: caps, limitations: [] }];
      case "spotify_devices":
        return [{ id: "dev", name: "PC", type: "Computer", isActive: true, isRestricted: false, supportsVolume: true }];
      case "queue_get":
      case "queue_replace":
        return snap();
      case "queue_advance":
        queueIndex++;
        return snap();
      case "spotify_play_track": {
        const t = items.find((x) => x.id === args.trackId)!;
        // Track 1 is relinked by Spotify to a market-specific copy (no linked_from).
        devicePlaying = { uri: t.id === "t1" ? "spotify:track:relinked" : t.providerUri, startedAt: performance.now(), startMs: args.positionMs ?? 0, durationMs: t.durationMs };
        return null;
      }
      case "spotify_playback_state": {
        if (!devicePlaying) return null;
        const p = devicePlaying.startMs + (performance.now() - devicePlaying.startedAt);
        if (p >= devicePlaying.durationMs) {
          // Track finished, nothing else queued on the device: stopped at 0.
          return { itemUri: devicePlaying.uri, progressMs: 0, isPlaying: false, itemDurationMs: devicePlaying.durationMs };
        }
        return { itemUri: devicePlaying.uri, progressMs: Math.round(p), isPlaying: true, itemDurationMs: devicePlaying.durationMs };
      }
      case "lastfm_submit_play":
        return { queued: true, reason: null };
      case "lastfm_now_playing":
        return true;
      default:
        return null;
    }
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));

describe("spotify end of track", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date", "performance"] });
  });

  it("advances to the next track and keeps the timer moving", async () => {
    const { usePlayer } = await import("./store");
    await usePlayer.getState().init();
    await usePlayer.getState().playTracks(items as any, 0);
    expect(usePlayer.getState().status).toBe("playing");

    await vi.advanceTimersByTimeAsync(5_000);
    expect(usePlayer.getState().positionMs).toBeGreaterThan(3_000);

    await vi.advanceTimersByTimeAsync(10_000);
    expect(calls.filter((c) => c === "spotify_play_track").length).toBeGreaterThanOrEqual(2);
    expect(usePlayer.getState().loadedTrackId).not.toBe("t0");
    const p1 = usePlayer.getState().positionMs;
    await vi.advanceTimersByTimeAsync(2_000);
    expect(usePlayer.getState().positionMs).toBeGreaterThan(p1);

    // Relinked track 1 must keep playing (not be treated as a takeover) and end normally.
    expect(usePlayer.getState().loadedTrackId).toBe("t1");
    expect(usePlayer.getState().status).toBe("playing");
    await vi.advanceTimersByTimeAsync(12_000);
    expect(usePlayer.getState().loadedTrackId).toBe("t2");
    expect(usePlayer.getState().status).toBe("playing");
  });
});
