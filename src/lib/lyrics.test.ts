import { describe, expect, it } from "vitest";
import { activeLineIndex, isSectionHeader, LYRIC_LEAD_MS } from "./lyrics";

const lines = [
  { timeMs: 1000, text: "a" },
  { timeMs: 5000, text: "b" },
  { timeMs: 9000, text: "" },
  { timeMs: 12000, text: "c" },
];

describe("activeLineIndex", () => {
  it("finds the line being sung", () => {
    expect(activeLineIndex(lines, 0)).toBe(-1);
    expect(activeLineIndex(lines, 1000)).toBe(0);
    expect(activeLineIndex(lines, 4999 - LYRIC_LEAD_MS)).toBe(0);
    expect(activeLineIndex(lines, 5000 - LYRIC_LEAD_MS)).toBe(1);
    expect(activeLineIndex(lines, 10000)).toBe(2);
    expect(activeLineIndex(lines, 999999)).toBe(3);
    expect(activeLineIndex([], 1000)).toBe(-1);
  });
});

describe("isSectionHeader", () => {
  it("recognises Genius headers only", () => {
    expect(isSectionHeader("[Chorus]")).toBe(true);
    expect(isSectionHeader(" [Verse 2: Someone] ")).toBe(true);
    expect(isSectionHeader("[Chorus] and words")).toBe(false);
    expect(isSectionHeader("words")).toBe(false);
  });
});
