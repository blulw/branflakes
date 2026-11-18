import { describe, expect, it } from "vitest";
import { activeLine, isSectionHeader, LYRIC_LEAD_MS } from "./lyrics";

const lines = [
  { timeMs: 1000, text: "[Verse 1]", header: true },
  { timeMs: 1000, text: "a", endMs: 5000 },
  { timeMs: 5000, text: "b", endMs: 8000 },
  { timeMs: 12000, text: "[Chorus]", header: true },
  { timeMs: 12000, text: "c" },
];

describe("activeLine", () => {
  it("finds the line being sung", () => {
    expect(activeLine(lines, 0)).toEqual({ index: -1, singing: false });
    expect(activeLine(lines, 1000)).toEqual({ index: 1, singing: true });
    expect(activeLine(lines, 4999 - LYRIC_LEAD_MS)).toEqual({ index: 1, singing: true });
    expect(activeLine(lines, 5000 - LYRIC_LEAD_MS)).toEqual({ index: 2, singing: true });
    expect(activeLine(lines, 999999)).toEqual({ index: 4, singing: true });
    expect(activeLine([], 1000)).toEqual({ index: -1, singing: false });
  });

  it("skips section headers and knows about gaps", () => {
    expect(activeLine(lines, 10000)).toEqual({ index: 2, singing: false });
    expect(activeLine(lines, 12000 - LYRIC_LEAD_MS)).toEqual({ index: 4, singing: true });
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
