import { describe, expect, it } from "vitest";
import { resolveControls, type ControlInput } from "./controls";
import type { PlaybackCapabilities, Track } from "@/lib/types";

const ALL: PlaybackCapabilities = {
  play: true,
  pause: true,
  next: true,
  previous: true,
  seek: true,
  volume: true,
  queue: true,
  deviceSelection: false,
  playbackPosition: true,
  duration: true,
};
const NONE: PlaybackCapabilities = Object.fromEntries(Object.keys(ALL).map((k) => [k, false])) as unknown as PlaybackCapabilities;

function track(over: Partial<Track> = {}): Track {
  return {
    id: "t",
    source: "local",
    providerId: "p",
    providerUri: null,
    title: "T",
    artist: "A",
    album: "",
    albumArtist: "",
    trackNumber: null,
    discNumber: null,
    year: null,
    genre: "",
    durationMs: 180000,
    artwork: null,
    filePath: null,
    fileFormat: null,
    availability: "available",
    unavailableReason: null,
    permalinkUrl: null,
    uploader: null,
    uploaderUrl: null,
    inLibrary: true,
    hasOverrides: false,
    addedAt: "",
    ...over,
  };
}

const base = (over: Partial<ControlInput> = {}): ControlInput => ({
  track: track(),
  caps: ALL,
  queueLength: 3,
  currentIndex: 1,
  repeatMode: "off",
  positionMs: 0,
  durationMs: 180000,
  ...over,
});

describe("resolveControls", () => {
  it("disables everything with no track", () => {
    const c = resolveControls(base({ track: null }));
    expect(c.playPause.enabled).toBe(false);
    expect(c.seek.enabled).toBe(false);
  });

  it("enables all supported controls for local playback", () => {
    const c = resolveControls(base());
    expect(c.playPause.enabled && c.next.enabled && c.previous.enabled && c.seek.enabled && c.volume.enabled).toBe(true);
    expect(c.deviceSelection.enabled).toBe(false);
  });

  it("never enables controls a provider does not declare", () => {
    const c = resolveControls(base({ caps: { ...ALL, seek: false, volume: false } }));
    expect(c.seek.enabled).toBe(false);
    expect(c.volume.enabled).toBe(false);
  });

  it("blocks disconnected providers but still allows skipping via the queue", () => {
    const c = resolveControls(base({ track: track({ source: "spotify" }), caps: NONE }));
    expect(c.playPause.enabled).toBe(false);
    expect(c.blocker).toMatch(/Spotify/);
    expect(c.next.enabled).toBe(true);
    expect(c.seek.enabled).toBe(false);
  });

  it("requires a Spotify device and respects device volume support", () => {
    const spotifyCaps = { ...ALL, deviceSelection: true };
    const t = track({ source: "spotify" });
    expect(resolveControls(base({ track: t, caps: spotifyCaps, spotifyDevice: null })).playPause.enabled).toBe(false);
    const dev = { id: "d", name: "Phone", type: "Smartphone", isActive: true, isRestricted: false, supportsVolume: false, volumePercent: null };
    const c = resolveControls(base({ track: t, caps: spotifyCaps, spotifyDevice: dev }));
    expect(c.playPause.enabled).toBe(true);
    expect(c.volume.enabled).toBe(false);
    expect(c.deviceSelection.enabled).toBe(true);
    const d = resolveControls(base({ track: t, caps: spotifyCaps, spotifyDevice: dev, spotifyDisallows: ["seeking"] }));
    expect(d.seek.enabled).toBe(false);
  });

  it("blocks unavailable tracks with the provider's reason", () => {
    const c = resolveControls(base({ track: track({ availability: "unavailable", unavailableReason: "File not found" }) }));
    expect(c.playPause.enabled).toBe(false);
    expect(c.blocker).toBe("File not found");
  });

  it("handles queue edges and repeat-all", () => {
    expect(resolveControls(base({ currentIndex: 2 })).next.enabled).toBe(false);
    expect(resolveControls(base({ currentIndex: 2, repeatMode: "all" })).next.enabled).toBe(true);
    expect(resolveControls(base({ currentIndex: 0 })).previous.enabled).toBe(false);
    expect(resolveControls(base({ currentIndex: 0, positionMs: 5000 })).previous.enabled).toBe(true);
  });

  it("disables seek when duration is unknown", () => {
    expect(resolveControls(base({ durationMs: null })).seek.enabled).toBe(false);
  });
});
