import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const calls: { cmd: string; args: any }[] = [];
let responses: Record<string, any> = {};
let addDelay: Promise<void> | null = null;
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (p: string) => `asset://${p}`,
  invoke: vi.fn(async (cmd: string, args: any) => {
    calls.push({ cmd, args });
    if (cmd === "alternatives_search") {
      const r = responses[args.provider];
      if (r instanceof Error) throw { kind: "network", message: r.message };
      return r;
    }
    if (cmd === "alternatives_add") {
      if (addDelay) await addDelay;
      return responses.add ?? { kind: "Added", position: 2 };
    }
    return null;
  }),
}));
const toasts: string[] = [];
vi.mock("sonner", () => ({
  toast: { success: (m: string) => toasts.push(m), info: (m: string) => toasts.push(m), error: (m: string) => toasts.push(m), warning: (m: string) => toasts.push(m) },
}));

import { AlternativesDialog } from "./AlternativesDialog";

const track = (id: string, source: string, title: string, artist: string, durationMs: number, artwork: string | null = null) =>
  ({ id, source, title, artist, durationMs, artwork, availability: "available", providerId: id, album: "", albumArtist: "" }) as any;
const original = { ...track("orig", "spotify", "Hello", "Adele", 295000), availability: "unavailable" };
const localHit = track("loc1", "local", "Hello", "Adele", 295500, "C:/art/hello.jpg");
const scHit = track("sc1", "soundcloud", "Hello (Live)", "Adele", 301000, "https://i1.sndcdn.com/x.jpg");

function setup(extra: Partial<Parameters<typeof AlternativesDialog>[0]> = {}) {
  const onAdded = vi.fn();
  const onOpenChange = vi.fn();
  render(
    <AlternativesDialog open playlistId="p1" original={original} position={4} synced alreadyAdded={new Set()} onAdded={onAdded} onOpenChange={onOpenChange} {...extra} />,
  );
  return { onAdded, onOpenChange };
}

describe("Find alternative dialog", () => {
  beforeEach(() => {
    calls.length = 0;
    toasts.length = 0;
    addDelay = null;
    responses = {
      local: { candidates: [{ track: localHit, score: 105 }], error: null, unavailable: null },
      soundcloud: { candidates: [{ track: scHit, score: 85 }], error: null, unavailable: null },
    };
  });

  it("searches local files first, then SoundCloud (even with local matches)", async () => {
    setup();
    await screen.findByText("Hello (Live)");
    const searches = calls.filter((c) => c.cmd === "alternatives_search").map((c) => c.args.provider);
    expect(searches).toEqual(["local", "soundcloud"]);
  });

  it("shows artwork, title, artist, duration and source, grouped by provider", async () => {
    setup();
    const localGroup = await screen.findByRole("region", { name: "On this computer results" });
    const scGroup = screen.getByRole("region", { name: "SoundCloud results" });
    await within(scGroup).findByText("Hello (Live)");
    expect(within(localGroup).getByText("Hello")).toBeInTheDocument();
    expect(within(localGroup).getByText("Adele")).toBeInTheDocument();
    expect(within(localGroup).getByText("4:55")).toBeInTheDocument();
    expect(within(localGroup).getByLabelText("Source: Local")).toBeInTheDocument();
    expect(within(scGroup).getByLabelText("Source: SoundCloud")).toBeInTheDocument();
    expect(within(scGroup).getByText("5:01")).toBeInTheDocument();
    expect(scGroup.querySelector("img")).toHaveAttribute("src", "https://i1.sndcdn.com/x.jpg");
  });

  it("adds the chosen alternative after the original, once", async () => {
    let release!: () => void;
    addDelay = new Promise((r) => (release = r));
    const { onAdded, onOpenChange } = setup();
    const use = await screen.findByRole("button", { name: "Use Hello" });
    fireEvent.click(use);
    fireEvent.click(use); // repeated click while the first is in flight
    fireEvent.click(screen.getByRole("button", { name: "Use Hello (Live)" }));
    await act(async () => release());
    await waitFor(() => expect(onAdded).toHaveBeenCalledTimes(1));
    const adds = calls.filter((c) => c.cmd === "alternatives_add");
    expect(adds).toHaveLength(1);
    expect(adds[0].args).toEqual({ playlistId: "p1", originalTrackId: "orig", alternativeTrackId: "loc1", position: 4 });
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(toasts[0]).toContain("after “Hello”");
  });

  it("marks alternatives that were already added", async () => {
    setup({ alreadyAdded: new Set(["loc1"]) });
    const btn = await screen.findByRole("button", { name: "Hello already added" });
    expect(btn).toBeDisabled();
  });

  it("shows local results and explains a SoundCloud failure", async () => {
    responses.soundcloud = new Error("could not connect");
    setup();
    expect(await screen.findByText(/Search failed: could not connect/)).toBeInTheDocument();
    expect(screen.getByText("Hello")).toBeInTheDocument();
  });

  it("explains when SoundCloud isn't set up, and when nothing matches, and can retry", async () => {
    responses.local = { candidates: [], error: null, unavailable: null };
    responses.soundcloud = { candidates: [], error: null, unavailable: "SoundCloud isn't connected." };
    setup();
    expect(await screen.findByText("SoundCloud isn't connected.")).toBeInTheDocument();
    expect(screen.getByText("No matching versions of this song were found.")).toBeInTheDocument();
    responses.local = { candidates: [{ track: localHit, score: 105 }], error: null, unavailable: null };
    fireEvent.click(screen.getByRole("button", { name: /Search again/ }));
    expect(await screen.findByText("Hello")).toBeInTheDocument();
    expect(calls.filter((c) => c.cmd === "alternatives_search")).toHaveLength(4);
  });
});
