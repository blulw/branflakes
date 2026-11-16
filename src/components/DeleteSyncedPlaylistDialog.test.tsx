import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const calls: { cmd: string; args: any }[] = [];
let fail: string | null = null;
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (p: string) => p,
  invoke: vi.fn(async (cmd: string, args: any) => {
    calls.push({ cmd, args });
    if (fail) throw { kind: "network", message: fail };
    return "“Mix” was deleted from Spotify and Branflakes.";
  }),
}));

import { DeleteSyncedPlaylistDialog } from "./DeleteSyncedPlaylistDialog";

const playlist = (canEditDetails: boolean) =>
  ({
    id: "p1",
    name: "Mix",
    sync: { spotifyPlaylistId: "sp", status: "ok", lastSyncedAt: null, lastError: null, lastNotice: null, canEditDetails },
  }) as any;

function setup(owned = true) {
  const onDeleted = vi.fn();
  const onOpenChange = vi.fn();
  render(<DeleteSyncedPlaylistDialog playlist={playlist(owned)} open onOpenChange={onOpenChange} onDeleted={onDeleted} />);
  return { onDeleted, onOpenChange };
}

describe("deleting a Spotify Synced playlist", () => {
  beforeEach(() => {
    calls.length = 0;
    fail = null;
  });

  it("explains the effect on Spotify and does nothing until confirmed", () => {
    setup(true);
    expect(screen.getByText(/deletes the actual playlist from your Spotify account/)).toBeInTheDocument();
    expect(calls).toHaveLength(0);
  });

  it("cancel leaves both playlists untouched", () => {
    const { onOpenChange, onDeleted } = setup(true);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onDeleted).not.toHaveBeenCalled();
    expect(calls).toHaveLength(0);
  });

  it("confirming deletes on Spotify too (explicit flag)", async () => {
    const { onDeleted } = setup(true);
    fireEvent.click(screen.getByRole("button", { name: "Delete from Spotify too" }));
    await waitFor(() => expect(onDeleted).toHaveBeenCalled());
    expect(calls).toEqual([{ cmd: "playlist_delete", args: { id: "p1", confirmSpotify: true, keepOnSpotify: null } }]);
  });

  it("can delete only the Branflakes copy", async () => {
    const { onDeleted } = setup(true);
    fireEvent.click(screen.getByRole("button", { name: "Keep on Spotify, delete here only" }));
    await waitFor(() => expect(onDeleted).toHaveBeenCalled());
    expect(calls[0].args).toEqual({ id: "p1", confirmSpotify: null, keepOnSpotify: true });
  });

  it("someone else's playlist is unfollowed, not deleted", () => {
    setup(false);
    expect(screen.getByText(/remove it from your Spotify library/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove from my Spotify library" })).toBeInTheDocument();
  });

  it("a failure keeps the dialog open with the reason, for a retry", async () => {
    fail = "could not connect";
    const { onDeleted, onOpenChange } = setup(true);
    fireEvent.click(screen.getByRole("button", { name: "Delete from Spotify too" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Nothing was deleted: could not connect");
    expect(onDeleted).not.toHaveBeenCalled();
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
    fail = null;
    fireEvent.click(screen.getByRole("button", { name: "Delete from Spotify too" }));
    await waitFor(() => expect(onDeleted).toHaveBeenCalled());
  });
});
