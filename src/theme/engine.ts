import type { CSSProperties } from "react";
import { CUSTOM_THEME_ID, DEFAULT_THEME_ID, PRESETS, type CustomTheme, type ThemeFont, type ThemePreset } from "./themes";

/** Everything needed to paint a theme. */
export interface ResolvedTheme {
  background: string;
  surface: string;
  primary: string;
  accent: string;
  foreground: string;
  opacity: number;
  blur: number;
  glow: number;
  glowAccent: boolean;
  overlay: string;
  font: ThemeFont;
  radius: number;
  image?: { url: string; fit: CustomTheme["imageFit"]; blur: number };
}

export const FONTS: Record<ThemeFont, { label: string; stack: string }> = {
  system: { label: "System", stack: '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif' },
  rounded: { label: "Rounded", stack: '"Arial Rounded MT Bold", "Segoe UI Variable Display", "Segoe UI", system-ui, sans-serif' },
  serif: { label: "Serif", stack: 'Georgia, Cambria, "Times New Roman", serif' },
  mono: { label: "Monospace", stack: '"Cascadia Code", "Cascadia Mono", Consolas, monospace' },
  display: { label: "Display", stack: 'Bahnschrift, "Segoe UI Variable Display", "Segoe UI", sans-serif' },
};

function rgb(hex: string): [number, number, number] {
  let h = hex.replace("#", "").trim();
  if (h.length === 3 || h.length === 4) h = [...h].map((c) => c + c).join("");
  const n = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16));
  return n.some(Number.isNaN) ? [0, 0, 0] : (n as [number, number, number]);
}

/** Mix two hex colours in sRGB; `t` is the share of `b`. Returns #rrggbb. */
export function mixHex(a: string, b: string, t: number): string {
  const [x, y] = [rgb(a), rgb(b)];
  return "#" + x.map((v, i) => Math.round(v + (y[i] - v) * t).toString(16).padStart(2, "0")).join("");
}

/** WCAG relative luminance, 0–1. */
export function luminance(hex: string): number {
  const [r, g, b] = rgb(hex).map((v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

const contrast = (a: number, b: number) => (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);

export const isLight = (hex: string) => luminance(hex) > 0.4;

/** Readable text colour for a filled `primary` (near-white or near-black). */
export function onColor(hex: string): string {
  const l = luminance(hex);
  return contrast(l, 1) >= contrast(l, luminance("#141414")) ? "#ffffff" : "#141414";
}

const alpha = (c: string, pct: number) => (pct >= 100 ? c : `color-mix(in srgb, ${c} ${Math.round(pct)}%, transparent)`);
const mix = (a: string, b: string, pctB: number) => `color-mix(in srgb, ${a}, ${b} ${pctB}%)`;
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function presetById(id: string): ThemePreset | undefined {
  return PRESETS.find((p) => p.id === id);
}

export function resolvePreset(p: ThemePreset): ResolvedTheme {
  return {
    background: p.background,
    surface: p.surface,
    primary: p.primary,
    accent: p.accent,
    foreground: p.foreground,
    opacity: p.opacity ?? 1,
    blur: p.blur ?? 12,
    glow: p.glow ?? 0,
    glowAccent: !!p.glowAccent,
    overlay: p.overlay ?? `radial-gradient(1200px 600px at 100% 0%, ${alpha(p.accent, 9)}, transparent 65%)`,
    font: p.font ?? "system",
    radius: p.radius ?? 0.6,
  };
}

export function customOverlay(c: CustomTheme): string {
  switch (c.gradient) {
    case "linear":
      return `linear-gradient(${c.gradientAngle}deg, ${c.background} 0%, ${c.gradientColor} 100%)`;
    case "radial":
      return `radial-gradient(1200px 800px at 30% 0%, ${c.gradientColor}, transparent 70%)`;
    default:
      return "none";
  }
}

export function resolveCustom(c: CustomTheme, image: string | null): ResolvedTheme {
  const dim = clamp(c.imageDim, 0, 0.95);
  return {
    background: c.background,
    surface: c.surface,
    primary: c.primary,
    accent: c.accent,
    foreground: c.foreground,
    opacity: clamp(c.opacity, 0.1, 1),
    blur: c.blur,
    glow: clamp(c.glow, 0, 1),
    glowAccent: c.glowAccent,
    // With an image, the overlay tints it toward the background colour (plus the gradient on top, if any).
    overlay: image
      ? [c.gradient !== "none" ? customOverlay({ ...c, background: alpha(c.background, 60) }) : null, `linear-gradient(${alpha(c.background, dim * 100)}, ${alpha(c.background, dim * 100)})`]
          .filter(Boolean)
          .join(", ")
      : customOverlay(c),
    font: c.font,
    radius: c.radius,
    image: image ? { url: image, fit: c.imageFit, blur: c.imageBlur } : undefined,
  };
}

/** CSS custom properties for a resolved theme. */
export function themeVars(t: ResolvedTheme): Record<string, string> {
  const light = isLight(t.background);
  const op = t.opacity * 100;
  const glowColor = t.glowAccent ? t.accent : t.primary;
  const translucent = t.opacity < 1 || !!t.image;
  return {
    "--background": t.background,
    "--foreground": t.foreground,
    "--card": alpha(mix(t.surface, t.foreground, 4), op),
    "--card-foreground": t.foreground,
    "--popover": alpha(mix(t.surface, t.foreground, 5), Math.max(op, 92)),
    "--popover-foreground": t.foreground,
    "--primary": t.primary,
    "--primary-foreground": onColor(t.primary),
    "--secondary": alpha(mix(t.surface, t.foreground, 10), Math.max(op, 70)),
    "--secondary-foreground": t.foreground,
    "--muted": alpha(mix(t.surface, t.foreground, 7), Math.max(op, 60)),
    "--muted-foreground": mix(t.foreground, t.background, 38),
    "--accent": alpha(mix(mix(t.surface, t.foreground, 8), t.accent, 14), Math.max(op, 75)),
    "--accent-foreground": t.foreground,
    "--border": t.glow >= 0.5 ? alpha(glowColor, 10 + t.glow * 18) : alpha(t.foreground, light ? 14 : 10),
    "--input": alpha(t.foreground, light ? 18 : 13),
    "--ring": alpha(t.primary, 60),
    "--sidebar": alpha(t.surface, op),
    "--radius": `${t.radius}rem`,
    "--glow": String(t.glow),
    "--glow-color": glowColor,
    "--surface-blur": translucent ? `${t.blur}px` : "0px",
    "--app-font": FONTS[t.font].stack,
    "--app-overlay": t.overlay,
    "--app-image": t.image ? `url("${t.image.url}")` : "none",
    "--app-image-size": t.image?.fit === "contain" ? "contain" : t.image?.fit === "tile" ? "auto" : "cover",
    "--app-image-repeat": t.image?.fit === "tile" ? "repeat" : "no-repeat",
    "--app-image-blur": `${t.image?.blur ?? 0}px`,
  };
}

const VAR_NAMES = Object.keys(
  themeVars(resolvePreset(PRESETS[0])),
);

/** Paint a theme onto the document. The default theme clears every override. */
export function applyTheme(id: string, custom: CustomTheme, image: string | null, root: HTMLElement = document.documentElement) {
  const preset = id === CUSTOM_THEME_ID ? undefined : (presetById(id) ?? presetById(DEFAULT_THEME_ID));
  if (preset?.id === DEFAULT_THEME_ID) {
    for (const k of VAR_NAMES) root.style.removeProperty(k);
    delete root.dataset.glow;
    delete root.dataset.translucent;
    root.dataset.theme = DEFAULT_THEME_ID;
    root.style.colorScheme = "dark";
    return;
  }
  const t = preset ? resolvePreset(preset) : resolveCustom(custom, image);
  for (const [k, v] of Object.entries(themeVars(t))) root.style.setProperty(k, v);
  root.dataset.theme = preset?.id ?? CUSTOM_THEME_ID;
  if (t.glow > 0) root.dataset.glow = "";
  else delete root.dataset.glow;
  if (t.opacity < 1 || t.image) root.dataset.translucent = "";
  else delete root.dataset.translucent;
  root.style.colorScheme = isLight(t.background) ? "light" : "dark";
}

/** Inline style for a small preview tile of a theme. */
export function previewStyle(t: ResolvedTheme): CSSProperties {
  const layers = [t.overlay !== "none" ? t.overlay : null, t.image ? `url("${t.image.url}") center / cover no-repeat` : null].filter(Boolean);
  return {
    background: layers.length ? `${layers.join(", ")}, ${t.background}` : t.background,
    color: t.foreground,
    fontFamily: FONTS[t.font].stack,
  };
}
