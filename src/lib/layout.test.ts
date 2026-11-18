import { describe, expect, it } from "vitest";
import { computeLayout } from "./layout";

describe("computeLayout", () => {
  it("shows everything on a wide window", () => {
    expect(computeLayout(1280, true)).toEqual({ sidebarWidth: 240, sidebarLabels: true, playlistTitles: true, showMain: true, lyricsDocked: true });
    expect(computeLayout(1280, false)).toMatchObject({ sidebarWidth: 240, showMain: true });
  });

  it("drops the main view first when lyrics are open", () => {
    expect(computeLayout(900, true)).toMatchObject({ sidebarWidth: 240, showMain: false, lyricsDocked: false });
  });

  it("keeps a thin main view when lyrics are closed", () => {
    expect(computeLayout(900, false)).toMatchObject({ showMain: true, sidebarWidth: 240 });
    expect(computeLayout(480, false)).toMatchObject({ showMain: true, sidebarWidth: 64, playlistTitles: false });
  });

  it("shrinks the sidebar smoothly, dropping labels then playlist titles", () => {
    const widths = [660, 640, 620, 600, 580, 560, 540, 520, 500, 480].map((w) => computeLayout(w, true).sidebarWidth);
    for (let i = 1; i < widths.length; i++) expect(widths[i - 1] - widths[i]).toBeLessThanOrEqual(40);
    expect(computeLayout(620, true)).toMatchObject({ sidebarLabels: true, playlistTitles: true });
    expect(computeLayout(590, true)).toMatchObject({ sidebarLabels: false, playlistTitles: true });
    expect(computeLayout(540, true)).toMatchObject({ sidebarLabels: false, playlistTitles: false });
  });
});
