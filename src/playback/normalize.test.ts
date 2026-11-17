import { describe, expect, it, vi } from "vitest";

vi.mock("@/lib/api", () => ({ api: {} }));
vi.mock("@/lib/tauri", () => ({ convertFileSrc: (p: string) => p }));

import { NORMALIZE_TARGET_LUFS, effectiveVolume, normalizationGain } from "./engines/audioElement";

describe("volume normalization", () => {
  it("leaves a track at the target untouched", () => {
    expect(normalizationGain(NORMALIZE_TARGET_LUFS)).toBeCloseTo(1);
  });
  it("turns a track 6 dB too loud down to half amplitude", () => {
    expect(normalizationGain(NORMALIZE_TARGET_LUFS + 6.0206)).toBeCloseTo(0.5, 3);
  });
  it("uses unity gain when loudness is unknown", () => {
    expect(normalizationGain(null)).toBe(1);
  });
  it("scales the slider volume by the track gain and clamps to 0..1", () => {
    expect(effectiveVolume(0.8, 0.5)).toBeCloseTo(0.4);
    expect(effectiveVolume(0.8, 2)).toBe(1);
    expect(effectiveVolume(0, 2)).toBe(0);
  });
});
