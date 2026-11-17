// Shared volume-normalization math for the playback engines.

/** Loudness every track is scaled towards when normalization is on (LUFS). */
export const NORMALIZE_TARGET_LUFS = -14;
/** Gain used until a track's loudness is known: assumes a typical modern master (~-9 LUFS). */
export const UNKNOWN_GAIN = 0.6;
/** How long a load waits for the loudness lookup before starting playback. */
export const LOUDNESS_WAIT_MS = 300;

/** Linear gain bringing a track at `lufs` to the target. Quieter tracks may get > 1. */
export function normalizationGain(lufs: number | null): number {
  if (lufs == null || !Number.isFinite(lufs)) return 1;
  const g = Math.pow(10, (NORMALIZE_TARGET_LUFS - lufs) / 20);
  return Math.min(4, Math.max(0.05, g));
}

/** Player volume for a user volume (slider) and per-track gain, clamped to 0..1. */
export function effectiveVolume(userVolume: number, gain: number): number {
  return Math.min(1, Math.max(0, userVolume * gain));
}

/** Difference between two linear gains in dB. */
export function gainDiffDb(a: number, b: number): number {
  return 20 * Math.log10(Math.max(1e-6, a) / Math.max(1e-6, b));
}
