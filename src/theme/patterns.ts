/** CSS background patterns shared by the theme presets. */

export const stars = (c: string) =>
  (
    [
      [13, 27, 180],
      [71, 113, 180],
      [140, 41, 180],
      [37, 151, 240],
      [199, 89, 240],
      [111, 203, 240],
      [167, 17, 300],
      [251, 173, 300],
      [61, 262, 300],
    ] as const
  )
    .map(([x, y, s]) => `radial-gradient(1.3px 1.3px at ${x}px ${y}px, ${c}, transparent) 0 0 / ${s}px ${s}px`)
    .join(", ");

export const glowAt = (c: string, at: string, size = "900px 600px") => `radial-gradient(${size} at ${at}, ${c}, transparent 65%)`;
export const scanlines = (c: string) => `repeating-linear-gradient(0deg, ${c} 0 1px, transparent 1px 3px)`;
export const grid = (c: string, s = 24) => `linear-gradient(${c} 1px, transparent 1px) 0 0 / ${s}px ${s}px, linear-gradient(90deg, ${c} 1px, transparent 1px) 0 0 / ${s}px ${s}px`;
