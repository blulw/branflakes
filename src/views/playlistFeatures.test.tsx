import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const calls: { cmd: string; args: any }[] = [];
let spotifyConnected = true;
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (p: string) => p,
  invoke: vi.fn(async (cmd: string, args: any) => {
    calls.push({ cmd, args });
    switch (cmd) {
      case "playlist_list":
      case "artists_list":
        return [];
      case "playlist_create":
        return { playlist: { id: "new", name: args.name }, spotifyError: null };
      default:
        return null;
    }
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), info: vi.fn(), error: vi.fn(), warning: vi.fn() } }));

// jsdom has no ResizeObserver (used by Radix Switch).
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as any;

import { canFindAlternative } from "./PlaylistView";
import { TrackTable } from "@/components/TrackTable";
import { Sidebar } from "@/components/Sidebar";
import { usePlayer } from "@/playback/store";

const t = (id: string, source: string, availability: string) =>
  ({ id, source, availability, title: `Song ${id}`, artist: "A", album: "", albumArtist: "", durationMs: 1000, artwork: null, providerId: id }) as any;

describe("Find alternative availability", () => {
  it("is offered only for unavailable Spotify tracks", () => {
    expect(canFindAlternative({ source: "spotify", availability: "unavailable" })).toBe(true);
    expect(canFindAlternative({ source: "spotify", availability: "available" })).toBe(false);
    expect(canFindAlternative({ source: "local", availability: "unavailable" })).toBe(false);
    expect(canFindAlternative({ source: "soundcloud", availability: "unavailable" })).toBe(false);
  });

  it("the button appears beside unavailable Spotify rows only, and is accessible", () => {
    const tracks = [t("ok", "spotify", "available"), t("gone", "spotify", "unavailable"), t("loc", "local", "available")];
    render(
      <TrackTable
        tracks={tracks}
        rowAccessory={(tr) => (canFindAlternative(tr) ? <button aria-label="Find alternative" title="Find alternative" /> : null)}
      />,
    );
    const buttons = screen.getAllByRole("button", { name: "Find alternative" });
    expect(buttons).toHaveLength(1);
    expect(buttons[0]).toHaveAttribute("title", "Find alternative");
    expect(buttons[0].closest("[role=row]")).toHaveTextContent("Song gone");
  });
});

describe("creating playlists", () => {
  beforeEach(() => {
    calls.length = 0;
    usePlayer.setState({
      providers: { spotify: { connected: spotifyConnected, provider: "spotify" } as any },
    });
  });

  it("is local-only by default", async () => {
    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "New playlist" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Playlist name" }), { target: { value: "Mine" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(calls.some((c) => c.cmd === "playlist_create")).toBe(true));
    expect(calls.find((c) => c.cmd === "playlist_create")!.args).toEqual({ name: "Mine", syncWithSpotify: false });
  });

  it("creates on Spotify only when the user turns the option on", async () => {
    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "New playlist" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Playlist name" }), { target: { value: "Shared" } });
    expect(screen.getByText(/Also create it on Spotify \(private\)/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch"));
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(calls.some((c) => c.cmd === "playlist_create")).toBe(true));
    expect(calls.find((c) => c.cmd === "playlist_create")!.args).toEqual({ name: "Shared", syncWithSpotify: true });
  });

  it("can't request Spotify sync while Spotify is disconnected", async () => {
    usePlayer.setState({ providers: {} });
    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "New playlist" }));
    expect(screen.getByRole("switch")).toBeDisabled();
    expect(screen.getByText(/Connect Spotify in Settings/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "Playlist name" }), { target: { value: "Offline" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(calls.find((c) => c.cmd === "playlist_create")?.args).toEqual({ name: "Offline", syncWithSpotify: false }));
  });
});
