import { describe, expect, it } from "vitest";
import { computeLayout } from "./layout";

describe("computeLayout", () => {
  it("shows everything on a wide window", () => {
    expect(computeLayout(1280, true)).toEqual({ sidebarMode: "full", sidebarWidth: 240, showMain: true, lyricsDocked: true });
    expect(computeLayout(1280, false)).toMatchObject({ sidebarMode: "full", sidebarWidth: 240, showMain: true });
  });

  it("drops the main view first when lyrics are open", () => {
    expect(computeLayout(900, true)).toMatchObject({ sidebarMode: "full", sidebarWidth: 240, showMain: false, lyricsDocked: false });
  });

  it("keeps a thin main view when lyrics are closed", () => {
    expect(computeLayout(900, false)).toMatchObject({ showMain: true, sidebarMode: "full" });
    expect(computeLayout(480, false)).toMatchObject({ showMain: true, sidebarMode: "rail" });
  });

  it("hides sidebar labels, then playlist titles", () => {
    expect(computeLayout(600, true).sidebarMode).toBe("full");
    expect(computeLayout(560, true).sidebarMode).toBe("icons");
    expect(computeLayout(500, true).sidebarMode).toBe("rail");
    expect(computeLayout(300, true)).toMatchObject({ sidebarMode: "rail", sidebarWidth: 64, showMain: false });
  });
});
