import { describe, expect, it } from "vitest";
import { timeAgo } from "./FollowedArtists";

describe("timeAgo", () => {
  const now = Date.parse("2026-11-21T12:00:00Z");
  it("formats sync times", () => {
    expect(timeAgo(null, now)).toBe("never");
    expect(timeAgo("2026-11-21T11:59:30Z", now)).toBe("just now");
    expect(timeAgo("2026-11-21T11:45:00Z", now)).toBe("15 min ago");
    expect(timeAgo("2026-11-21T09:00:00Z", now)).toBe("3 h ago");
    expect(timeAgo("2026-11-20T12:00:00Z", now)).toBe("1 day ago");
    expect(timeAgo("2026-11-18T12:00:00Z", now)).toBe("3 days ago");
  });
});
