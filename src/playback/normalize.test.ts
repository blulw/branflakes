import { describe, expect, it } from "vitest";
import { NORMALIZE_TARGET_LUFS, effectiveVolume, gainDiffDb, normalizationGain } from "./normalize";

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
  it("measures gain changes in dB", () => {
    expect(gainDiffDb(1, 0.5)).toBeCloseTo(6.02, 2);
  });
});
