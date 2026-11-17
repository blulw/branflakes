import { afterEach, describe, expect, it, vi } from "vitest";

const calls: { cmd: string; args: any }[] = [];
let loudness: number | null = null;
let captureStatus: any = null;
const sdkVolumes: number[] = [];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args: any) => {
    calls.push({ cmd, args });
    if (cmd === "track_loudness") return loudness;
    if (cmd === "loudness_capture_status") return captureStatus;
    return null;
  }),
}));
vi.mock("./engines/spotifySdk", () => ({ sdkSetVolume: vi.fn(async (v: number) => void sdkVolumes.push(v)) }));

import { SpotifyConnectEngine } from "./engines/spotifyConnect";
import type { Track } from "@/lib/types";

const track = { id: "t1", source: "spotify", providerId: "x", providerUri: "spotify:track:x", title: "T", durationMs: 200_000 } as unknown as Track;
const cb = { onTime() {}, onDuration() {}, onPlaying() {}, onEnded() {}, onError() {} };

function engine(inApp: boolean) {
  return new SpotifyConnectEngine(cb, { getDeviceId: () => "dev", onState() {}, getVolume: () => 0.8, isInApp: () => inApp });
}

afterEach(() => {
  calls.length = 0;
  sdkVolumes.length = 0;
  loudness = null;
  captureStatus = null;
  vi.useRealTimers();
});

describe("Spotify volume normalization", () => {
  it("applies a known loudness (e.g. from a local copy) to a remote device", async () => {
    loudness = -8; // 6 dB louder than the -14 target → half amplitude
    const e = engine(false);
    await e.load(track, 0, true);
    const vol = calls.find((c) => c.cmd === "spotify_control" && c.args.action === "volume");
    expect(vol?.args.value).toBe(40); // 0.8 × 0.5
    expect(calls.some((c) => c.cmd === "loudness_capture_start")).toBe(false);
    await e.release();
  });

  it("measures unknown songs live on the in-app player and adjusts after a few seconds", async () => {
    vi.useFakeTimers();
    const e = engine(true);
    await e.load(track, 0, true);
    await vi.advanceTimersByTimeAsync(0);
    expect(calls.some((c) => c.cmd === "loudness_capture_start" && c.args.trackId === "t1")).toBe(true);
    captureStatus = { trackId: "t1", seconds: 7, lufs: -8, error: null };
    await vi.advanceTimersByTimeAsync(1000 + 1100);
    expect(sdkVolumes.at(-1)).toBeCloseTo(0.4, 2);
    await e.release();
    expect(calls.some((c) => c.cmd === "loudness_capture_stop")).toBe(true);
  });

  it("leaves the volume alone when normalization is off", async () => {
    loudness = -8;
    const e = engine(true);
    e.setNormalize(false);
    await e.load(track, 0, true);
    expect(calls.some((c) => c.cmd === "track_loudness")).toBe(false);
    expect(sdkVolumes.at(-1)).toBeCloseTo(0.8);
    await e.release();
  });
});
