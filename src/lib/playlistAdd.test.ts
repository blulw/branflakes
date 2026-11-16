import { beforeEach, describe, expect, it, vi } from "vitest";

const toastCalls: { kind: string; msg: string; opts?: any }[] = [];
vi.mock("sonner", () => ({
  toast: {
    success: (msg: string, opts?: any) => toastCalls.push({ kind: "success", msg, opts }),
    info: (msg: string, opts?: any) => toastCalls.push({ kind: "info", msg, opts }),
    warning: (msg: string, opts?: any) => toastCalls.push({ kind: "warning", msg, opts }),
    error: (msg: string, opts?: any) => toastCalls.push({ kind: "error", msg, opts }),
  },
}));
let next: any;
const invokeMock = vi.fn(async () => {
  if (next instanceof Error) throw { kind: "not_found", message: next.message };
  return next;
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: any[]) => (invokeMock as any)(...a) }));

import { addTracksToPlaylist } from "./playlistAdd";

const synced = { id: "p1", name: "Road trip" };
const localTrack = { id: "t-local", title: "Demo.mp3" };

describe("addTracksToPlaylist", () => {
  beforeEach(() => {
    toastCalls.length = 0;
    invokeMock.mockClear();
  });

  it("passes the intended position to the backend", async () => {
    next = { kind: "Added", added: 1, skippedDuplicates: 0 };
    await addTracksToPlaylist(synced, [localTrack], { position: 3 });
    expect(invokeMock).toHaveBeenCalledWith("playlist_add_tracks", { id: "p1", trackIds: ["t-local"], position: 3 });
    expect(toastCalls[0]).toMatchObject({ kind: "success", msg: "Added to Road trip" });
  });

  it("explains a Branflakes-only copy in a friendly in-app message", async () => {
    const copy = { id: "p2", name: "Road trip (Branflakes)" };
    next = { kind: "Forked", playlist: copy, added: 1, notOnSpotify: 1, uncopyable: [], fromLatest: true, note: "" };
    const onOpen = vi.fn();
    const r = await addTracksToPlaylist(synced, [localTrack], { onOpen });
    expect(r.target).toEqual(copy);
    expect(toastCalls).toHaveLength(1);
    expect(toastCalls[0].kind).toBe("info");
    expect(toastCalls[0].msg).toContain("isn't on Spotify");
    expect(toastCalls[0].msg).toContain("local-only copy");
    expect(toastCalls[0].opts.description).toContain("Road trip (Branflakes)");
    toastCalls[0].opts.action.onClick();
    expect(onOpen).toHaveBeenCalledWith(copy);
  });

  it("warns when the copy is incomplete or based on the last sync", async () => {
    const copy = { id: "p2", name: "Road trip (Branflakes)" };
    next = {
      kind: "Forked",
      playlist: copy,
      added: 1,
      notOnSpotify: 1,
      uncopyable: [{ kind: "episode", name: "Ep 1" }, { kind: "spotify_local_file", name: null }],
      fromLatest: false,
      note: "",
    };
    await addTracksToPlaylist(synced, [localTrack]);
    expect(toastCalls[0].kind).toBe("warning");
    expect(toastCalls[0].opts.description).toContain("based on the last sync");
    expect(toastCalls[0].opts.description).toContain("2 Spotify entries");
    expect(toastCalls[0].opts.description).toContain("couldn't be copied");
  });

  it("reports duplicates that were skipped", async () => {
    next = { kind: "Added", added: 0, skippedDuplicates: 1 };
    await addTracksToPlaylist(synced, [{ id: "s", title: "S" }]);
    expect(toastCalls[0]).toMatchObject({ kind: "info", msg: "Already in “Road trip”" });
  });

  it("surfaces backend errors", async () => {
    next = new Error("playlist p1 not found");
    const r = await addTracksToPlaylist(synced, [localTrack]);
    expect(r.outcome).toBeNull();
    expect(toastCalls[0]).toMatchObject({ kind: "error", msg: "playlist p1 not found" });
  });
});
