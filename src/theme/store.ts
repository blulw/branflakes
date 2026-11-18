import { create } from "zustand";
import { applyTheme, mixHex, presetById } from "./engine";
import { CUSTOM_THEME_ID, DEFAULT_CUSTOM, DEFAULT_THEME_ID, type CustomTheme } from "./themes";

const KEY = "unified-music.theme";
const IMAGE_KEY = "unified-music.theme-image";

interface ThemeState {
  id: string;
  custom: CustomTheme;
  /** Custom background image as a data URL (kept apart from the settings because it is large). */
  image: string | null;
  setTheme(id: string): void;
  updateCustom(patch: Partial<CustomTheme>): void;
  /** Returns false if the image could not be saved (it still shows until restart). */
  setImage(dataUrl: string | null): boolean;
  /** Start the Custom theme from a preset's colours and options. */
  customiseFrom(id: string): void;
}

function load(): Pick<ThemeState, "id" | "custom" | "image"> {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? "null") as { id?: string; custom?: Partial<CustomTheme> } | null;
    const id = raw?.id && (raw.id === CUSTOM_THEME_ID || presetById(raw.id)) ? raw.id : DEFAULT_THEME_ID;
    return { id, custom: { ...DEFAULT_CUSTOM, ...raw?.custom }, image: localStorage.getItem(IMAGE_KEY) };
  } catch {
    return { id: DEFAULT_THEME_ID, custom: DEFAULT_CUSTOM, image: null };
  }
}

function save(s: Pick<ThemeState, "id" | "custom">) {
  try {
    localStorage.setItem(KEY, JSON.stringify({ id: s.id, custom: s.custom }));
  } catch {
    /* storage unavailable: the theme still applies for this session */
  }
}

export const useTheme = create<ThemeState>((set, get) => ({
  ...load(),
  setTheme(id) {
    set({ id });
    save(get());
  },
  updateCustom(patch) {
    set((s) => ({ custom: { ...s.custom, ...patch }, id: CUSTOM_THEME_ID }));
    save(get());
  },
  setImage(image) {
    set({ image, id: CUSTOM_THEME_ID });
    save(get());
    try {
      if (image) localStorage.setItem(IMAGE_KEY, image);
      else localStorage.removeItem(IMAGE_KEY);
      return true;
    } catch {
      return false;
    }
  },
  customiseFrom(id) {
    const p = presetById(id);
    if (!p) return;
    get().updateCustom({
      background: p.background,
      surface: p.surface,
      primary: p.primary,
      accent: p.accent,
      foreground: p.foreground,
      opacity: p.opacity ?? 1,
      blur: p.blur ?? 12,
      glow: p.glow ?? 0,
      glowAccent: !!p.glowAccent,
      font: p.font ?? "system",
      radius: p.radius ?? 0.6,
      gradient: p.overlay ? "radial" : "none",
      gradientColor: mixHex(p.background, p.accent, 0.25),
    });
  },
}));

/** Apply the saved theme now and whenever it changes. Call once before the first render. */
export function initTheme() {
  const paint = (s: ThemeState) => applyTheme(s.id, s.custom, s.image);
  paint(useTheme.getState());
  useTheme.subscribe(paint);
}
