import { describe, expect, it } from "vitest";
import { applyTheme, luminance, onColor, resolveCustom, resolvePreset, themeVars } from "./engine";
import { CUSTOM_THEME_ID, DEFAULT_CUSTOM, DEFAULT_THEME_ID, PRESETS } from "./themes";

const HEX = /^#[0-9a-f]{6}$/i;

describe("theme presets", () => {
  it("has 250 themes including the requested ones", () => {
    expect(PRESETS.length + 1).toBe(250);
    const names = PRESETS.map((p) => p.name);
    expect(names).toContain("Candy Floss");
    expect(names).toContain("Galaxy");
    expect(names).not.toContain("Custom");
  });

  it("has unique ids, 3-4 words and valid colours", () => {
    expect(new Set(PRESETS.map((p) => p.id)).size).toBe(PRESETS.length);
    for (const p of PRESETS) {
      expect(p.words.length, p.name).toBeGreaterThanOrEqual(3);
      expect(p.words.length, p.name).toBeLessThanOrEqual(4);
      for (const c of [p.background, p.surface, p.primary, p.accent, p.foreground]) expect(c, p.name).toMatch(HEX);
    }
  });

  it("has unique names", () => {
    expect(new Set(PRESETS.map((p) => p.name.toLowerCase())).size).toBe(PRESETS.length);
  });

  it("keeps text readable and the primary colour visible", () => {
    const ratio = (a: string, b: string) => {
      const [x, y] = [luminance(a), luminance(b)].sort((m, n) => n - m);
      return (x + 0.05) / (y + 0.05);
    };
    const bad: string[] = [];
    for (const p of PRESETS) {
      if (ratio(p.foreground, p.background) < 7) bad.push(`${p.name}: text on background ${ratio(p.foreground, p.background).toFixed(2)}`);
      if (ratio(p.foreground, p.surface) < 7) bad.push(`${p.name}: text on surface ${ratio(p.foreground, p.surface).toFixed(2)}`);
      if (ratio(p.primary, p.background) < 2.2) bad.push(`${p.name}: primary on background ${ratio(p.primary, p.background).toFixed(2)}`);
    }
    expect(bad).toEqual([]);
  });

  it("includes some glowing and some translucent themes", () => {
    expect(PRESETS.filter((p) => (p.glow ?? 0) > 0).length).toBeGreaterThan(5);
    expect(PRESETS.filter((p) => (p.opacity ?? 1) < 1).length).toBeGreaterThan(5);
  });
});

describe("theme engine", () => {
  it("picks readable text on primary", () => {
    expect(onColor("#ffffff")).toBe("#141414");
    expect(onColor("#1a1a8a")).toBe("#ffffff");
  });

  it("applies a preset and clears it for the default theme", () => {
    const root = document.createElement("div");
    applyTheme("galaxy", DEFAULT_CUSTOM, null, root);
    expect(root.style.getPropertyValue("--primary")).toBe("#b47cff");
    expect(root.dataset.theme).toBe("galaxy");
    expect(root.dataset.glow).toBe("");
    applyTheme(DEFAULT_THEME_ID, DEFAULT_CUSTOM, null, root);
    expect(root.style.getPropertyValue("--primary")).toBe("");
    expect(root.dataset.glow).toBeUndefined();
  });

  it("puts the custom image behind a tint", () => {
    const t = resolveCustom({ ...DEFAULT_CUSTOM, gradient: "none", imageDim: 0.5 }, "data:image/jpeg;base64,AAAA");
    const vars = themeVars(t);
    expect(vars["--app-image"]).toContain("data:image/jpeg");
    expect(vars["--app-overlay"]).toContain("50%");
    const root = document.createElement("div");
    applyTheme(CUSTOM_THEME_ID, DEFAULT_CUSTOM, null, root);
    expect(root.dataset.theme).toBe(CUSTOM_THEME_ID);
  });

  it("marks translucent themes", () => {
    expect(themeVars(resolvePreset(PRESETS.find((p) => p.id === "frosted-glass")!))["--surface-blur"]).not.toBe("0px");
  });
});
