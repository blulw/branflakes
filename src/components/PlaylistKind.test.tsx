import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { PlaylistKindIcon, playlistKind, playlistKindLabel } from "./PlaylistKind";
import { relativeTime } from "@/views/PlaylistView";

const sync = { spotifyPlaylistId: "sp", status: "ok" as const, lastSyncedAt: null, lastError: null, lastNotice: null, canEditDetails: true };

describe("playlist kind indicator", () => {
  it("distinguishes ordinary, Spotify Synced and Branflakes-only playlists", () => {
    expect(playlistKind({ sync: null, forkedFrom: null })).toBe("plain");
    expect(playlistKind({ sync, forkedFrom: null })).toBe("synced");
    expect(playlistKind({ sync: null, forkedFrom: "p1" })).toBe("fork");
    expect(playlistKindLabel({ sync, forkedFrom: null, forkedFromName: null })).toBe("Spotify Synced");
    expect(playlistKindLabel({ sync: null, forkedFrom: "p1", forkedFromName: "Road trip" })).toBe("Branflakes-only copy of “Road trip”");
  });

  it("renders a labelled icon only for synced playlists and copies", () => {
    const { rerender, container } = render(<PlaylistKindIcon playlist={{ sync, forkedFrom: null, forkedFromName: null }} />);
    expect(screen.getByRole("img", { name: "Spotify Synced" })).toHaveAttribute("title", "Spotify Synced");
    rerender(<PlaylistKindIcon playlist={{ sync: null, forkedFrom: "p", forkedFromName: "X" }} />);
    expect(screen.getByRole("img", { name: "Branflakes-only copy of “X”" })).toBeInTheDocument();
    rerender(<PlaylistKindIcon playlist={{ sync: null, forkedFrom: null, forkedFromName: null }} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("formats the last-synced time", () => {
    const now = Date.parse("2026-11-15T12:00:00Z");
    expect(relativeTime("2026-11-15T11:59:50Z", now)).toBe("just now");
    expect(relativeTime("2026-11-15T11:55:00Z", now)).toBe("5 min ago");
    expect(relativeTime("2026-11-15T09:00:00Z", now)).toBe("3 h ago");
  });
});
