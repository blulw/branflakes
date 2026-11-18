/**
 * Theme definitions. A theme is five colours plus a few options; `resolveTheme`
 * in ./engine.ts turns those into the app's CSS tokens.
 */

export type ThemeFont = "system" | "rounded" | "serif" | "mono" | "display";

export interface ThemeOptions {
  /** Opacity of the sidebar, player bar, lyrics panel and cards (0.2–1). */
  opacity?: number;
  /** Backdrop blur behind translucent surfaces, in px. */
  blur?: number;
  /** Glow strength, 0–1. */
  glow?: number;
  /** Glow uses the accent colour instead of the primary one. */
  glowAccent?: boolean;
  /** CSS `background` layers painted over the background colour. */
  overlay?: string;
  font?: ThemeFont;
  /** Corner radius in rem (default 0.6). */
  radius?: number;
}

export interface ThemePreset extends ThemeOptions {
  id: string;
  name: string;
  /** Three or four words describing the theme. */
  words: string[];
  background: string;
  /** Sidebar / player bar / cards. */
  surface: string;
  primary: string;
  /** Second colour: hover tints, glow, atmosphere. */
  accent: string;
  foreground: string;
}

export interface CustomTheme {
  background: string;
  surface: string;
  primary: string;
  accent: string;
  foreground: string;
  opacity: number;
  blur: number;
  glow: number;
  glowAccent: boolean;
  gradient: "none" | "linear" | "radial";
  gradientColor: string;
  gradientAngle: number;
  imageFit: "cover" | "contain" | "tile";
  /** Darken/lighten the background image toward the background colour, 0–0.9. */
  imageDim: number;
  imageBlur: number;
  radius: number;
  font: ThemeFont;
}

export const DEFAULT_THEME_ID = "branflakes";
export const CUSTOM_THEME_ID = "custom";

export const DEFAULT_CUSTOM: CustomTheme = {
  background: "#14121c",
  surface: "#1d1a29",
  primary: "#9b7bff",
  accent: "#ff7ac6",
  foreground: "#f1eefa",
  opacity: 0.85,
  blur: 12,
  glow: 0.3,
  glowAccent: false,
  gradient: "radial",
  gradientColor: "#2a1f4a",
  gradientAngle: 160,
  imageFit: "cover",
  imageDim: 0.45,
  imageBlur: 0,
  radius: 0.6,
  font: "system",
};

const stars = (c: string) =>
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

const glowAt = (c: string, at: string, size = "900px 600px") => `radial-gradient(${size} at ${at}, ${c}, transparent 65%)`;
const scanlines = (c: string) => `repeating-linear-gradient(0deg, ${c} 0 1px, transparent 1px 3px)`;
const grid = (c: string, s = 24) => `linear-gradient(${c} 1px, transparent 1px) 0 0 / ${s}px ${s}px, linear-gradient(90deg, ${c} 1px, transparent 1px) 0 0 / ${s}px ${s}px`;

export const PRESETS: ThemePreset[] = [
  { id: "branflakes", name: "Branflakes", words: ["Classic", "Calm", "Minty", "Dark"], background: "#121318", surface: "#0d0e12", primary: "#45d3a3", accent: "#2b2c33", foreground: "#f2f3f6" },
  {
    id: "candy-floss", name: "Candy Floss", words: ["Sweet", "Pastel", "Fluffy", "Dreamy"],
    background: "#fde6f3", surface: "#ffffff", primary: "#ff5fae", accent: "#6ec3ff", foreground: "#4a2340",
    opacity: 0.6, blur: 16, glow: 0.35, radius: 0.9, font: "rounded",
    overlay: `${glowAt("#ffb6dccc", "10% 0%")}, ${glowAt("#a9dcffcc", "95% 100%")}`,
  },
  {
    id: "galaxy", name: "Galaxy", words: ["Cosmic", "Starry", "Deep", "Mystic"],
    background: "#07051a", surface: "#130d33", primary: "#b47cff", accent: "#4fc3ff", foreground: "#ece6ff",
    opacity: 0.55, blur: 14, glow: 0.6,
    overlay: `${stars("#ffffffcc")}, ${glowAt("#7b3cff55", "15% 10%")}, ${glowAt("#1e8cff40", "85% 85%")}, ${glowAt("#ff4fd830", "60% 40%", "600px 400px")}`,
  },
  { id: "midnight", name: "Midnight", words: ["Quiet", "Blue", "Late", "Focused"], background: "#0a0f1c", surface: "#0f1629", primary: "#5b8cff", accent: "#9db4ff", foreground: "#e3e9f7" },
  {
    id: "neon-nights", name: "Neon Nights", words: ["Electric", "Bold", "Bright", "Nightlife"],
    background: "#0a0a12", surface: "#12121f", primary: "#ff2fd0", accent: "#00f0ff", foreground: "#f5f5ff",
    opacity: 0.8, glow: 1, overlay: `${glowAt("#ff2fd022", "0% 0%")}, ${glowAt("#00f0ff1c", "100% 100%")}`,
  },
  {
    id: "synthwave", name: "Synthwave", words: ["Retro", "Eighties", "Sunset", "Neon"],
    background: "#1a0b2e", surface: "#22103f", primary: "#ff7edb", accent: "#fede5d", foreground: "#f8eaff",
    opacity: 0.7, blur: 10, glow: 0.8, font: "display",
    overlay: `linear-gradient(180deg, transparent 0%, #2b0f5480 55%, #7a1e6a70 82%, #ff8a5c40 100%), ${grid("#ff7edb10", 32)}`,
  },
  {
    id: "cyberpunk", name: "Cyberpunk", words: ["Gritty", "Yellow", "Techy", "Loud"],
    background: "#0d0d0d", surface: "#161616", primary: "#fcee0a", accent: "#00f0ff", foreground: "#f2f2f2",
    glow: 0.6, font: "display", radius: 0.15, overlay: grid("#fcee0a08", 40),
  },
  {
    id: "matrix", name: "Matrix", words: ["Code", "Green", "Hacker", "Terminal"],
    background: "#000a03", surface: "#001408", primary: "#00ff66", accent: "#00b347", foreground: "#c8ffd9",
    glow: 0.7, font: "mono", radius: 0.2, overlay: scanlines("#00ff6608"),
  },
  {
    id: "ocean", name: "Ocean", words: ["Fresh", "Flowing", "Teal", "Breezy"],
    background: "#04202e", surface: "#072b3d", primary: "#2ec4d6", accent: "#4fa3ff", foreground: "#dff6fb",
    opacity: 0.8, blur: 10, overlay: `linear-gradient(170deg, #0a3a5060 0%, transparent 50%, #02121c 100%)`,
  },
  {
    id: "deep-sea", name: "Deep Sea", words: ["Abyssal", "Glowing", "Still", "Mysterious"],
    background: "#020b16", surface: "#06162a", primary: "#00c2a8", accent: "#2f6bff", foreground: "#cfe9f2",
    opacity: 0.65, blur: 14, glow: 0.4, overlay: `${glowAt("#00c2a825", "50% 0%", "1200px 500px")}, ${stars("#7ef2e033")}`,
  },
  { id: "forest", name: "Forest", words: ["Earthy", "Green", "Grounded", "Calm"], background: "#0f1a12", surface: "#15241a", primary: "#6fcf7a", accent: "#c9a96b", foreground: "#e4efe2" },
  { id: "moss", name: "Moss", words: ["Soft", "Olive", "Natural", "Muted"], background: "#1b1f16", surface: "#23291c", primary: "#a3c26b", accent: "#d8c48f", foreground: "#eceadb", radius: 0.9 },
  {
    id: "sunset", name: "Sunset", words: ["Warm", "Golden", "Hazy", "Romantic"],
    background: "#1f0f17", surface: "#2b1520", primary: "#ff8a5c", accent: "#ffcc66", foreground: "#fff0e6",
    opacity: 0.75, blur: 12, glow: 0.3, overlay: `linear-gradient(160deg, #3d1a4a 0%, #1f0f17 45%, #5a2a1c 100%)`,
  },
  {
    id: "sunrise", name: "Sunrise", words: ["Bright", "Hopeful", "Peachy", "Morning"],
    background: "#fff4e8", surface: "#ffe7d1", primary: "#ff6a2b", accent: "#ffb347", foreground: "#3d2416",
    opacity: 0.8, overlay: glowAt("#ffd0a080", "50% 0%", "1400px 600px"),
  },
  { id: "autumn", name: "Autumn", words: ["Cosy", "Rustic", "Amber", "Crisp"], background: "#1e130c", surface: "#2a1a10", primary: "#e07a2e", accent: "#c9a227", foreground: "#f6e8da" },
  {
    id: "cherry-blossom", name: "Cherry Blossom", words: ["Delicate", "Floral", "Spring", "Light"],
    background: "#fff0f4", surface: "#ffe1ea", primary: "#d94a7c", accent: "#8fbf7f", foreground: "#4b2433",
    opacity: 0.75, blur: 10, radius: 0.9, overlay: `${glowAt("#ffc4d680", "90% 0%")}, ${glowAt("#e9f5e080", "0% 100%")}`,
  },
  {
    id: "sakura-night", name: "Sakura Night", words: ["Soft", "Pink", "Moonlit", "Gentle"],
    background: "#1a1018", surface: "#251724", primary: "#ff9ec4", accent: "#c7a6ff", foreground: "#fbe9f1",
    opacity: 0.8, glow: 0.3, overlay: glowAt("#ff9ec41a", "80% 10%"),
  },
  { id: "lavender", name: "Lavender", words: ["Light", "Floral", "Airy", "Relaxed"], background: "#f3effb", surface: "#e8e0f7", primary: "#7a52cc", accent: "#d16ba5", foreground: "#2f2546", radius: 0.9 },
  {
    id: "lavender-haze", name: "Lavender Haze", words: ["Dreamy", "Purple", "Soft", "Hazy"],
    background: "#17131f", surface: "#201a2c", primary: "#b79cff", accent: "#ff9ecf", foreground: "#eee8fa",
    opacity: 0.75, blur: 16, glow: 0.25, overlay: `${glowAt("#b79cff26", "20% 20%")}, ${glowAt("#ff9ecf1c", "90% 90%")}`,
  },
  { id: "mint", name: "Mint", words: ["Clean", "Cool", "Fresh", "Light"], background: "#eefbf5", surface: "#dcf5e9", primary: "#13945f", accent: "#4cc3d9", foreground: "#173a2c" },
  { id: "coffee", name: "Coffee", words: ["Rich", "Roasted", "Cosy", "Brown"], background: "#1c1512", surface: "#261c18", primary: "#c89f7a", accent: "#e6c79c", foreground: "#f1e6dc", font: "serif" },
  { id: "latte", name: "Latte", words: ["Creamy", "Warm", "Light", "Mellow"], background: "#f6efe6", surface: "#ece1d2", primary: "#94582c", accent: "#c8925a", foreground: "#3b2a1e" },
  {
    id: "frosted-glass", name: "Frosted Glass", words: ["Clear", "Modern", "Layered", "Sleek"],
    background: "#10141c", surface: "#2a3346", primary: "#8ab4ff", accent: "#c79bff", foreground: "#eef2fa",
    opacity: 0.35, blur: 22, radius: 1, overlay: `${glowAt("#3b5bdb66", "10% 10%")}, ${glowAt("#9c36b555", "90% 30%")}, ${glowAt("#0ca67844", "40% 100%")}`,
  },
  { id: "paper", name: "Paper", words: ["Plain", "Crisp", "Readable", "Bright"], background: "#f8f7f4", surface: "#efeee9", primary: "#2f6fed", accent: "#d9480f", foreground: "#1d1d1f", font: "serif" },
  {
    id: "snow", name: "Snow", words: ["Icy", "Clean", "Bright", "Cool"],
    background: "#f5f8fc", surface: "#ffffff", primary: "#2563eb", accent: "#06b6d4", foreground: "#0f172a",
    opacity: 0.7, blur: 14, overlay: glowAt("#cfe3ff99", "100% 0%"),
  },
  {
    id: "arctic", name: "Arctic", words: ["Frozen", "Pale", "Crisp", "Glacial"],
    background: "#0e1720", surface: "#142230", primary: "#88d8ff", accent: "#c3f0ff", foreground: "#e8f6ff",
    opacity: 0.8, glow: 0.3, overlay: glowAt("#88d8ff18", "50% 0%", "1400px 500px"),
  },
  {
    id: "ember", name: "Ember", words: ["Smouldering", "Fiery", "Warm", "Intense"],
    background: "#140806", surface: "#1f0d09", primary: "#ff5a1f", accent: "#ffb02e", foreground: "#ffe9dd",
    glow: 0.6, overlay: glowAt("#ff5a1f22", "50% 120%", "1400px 700px"),
  },
  {
    id: "lava", name: "Lava", words: ["Molten", "Red", "Volcanic", "Hot"],
    background: "#120404", surface: "#1d0807", primary: "#ff3b2f", accent: "#ff9f1c", foreground: "#ffe3dc",
    opacity: 0.8, glow: 0.8, overlay: `${glowAt("#ff3b2f30", "20% 100%")}, ${glowAt("#ff9f1c20", "90% 90%")}`,
  },
  {
    id: "toxic", name: "Toxic", words: ["Radioactive", "Acid", "Loud", "Lime"],
    background: "#0b1004", surface: "#141c08", primary: "#b6ff00", accent: "#39ff14", foreground: "#effbd8",
    glow: 0.9, overlay: glowAt("#b6ff0014", "50% 50%", "1200px 800px"),
  },
  {
    id: "bubblegum", name: "Bubblegum", words: ["Playful", "Pop", "Pink", "Fun"],
    background: "#2a1030", surface: "#36163e", primary: "#ff6ad5", accent: "#6ae8ff", foreground: "#ffe8fa",
    glow: 0.4, radius: 1.1, font: "rounded",
  },
  { id: "lemonade", name: "Lemonade", words: ["Zesty", "Sunny", "Light", "Cheerful"], background: "#fffbe6", surface: "#fff3c2", primary: "#c98a00", accent: "#ff6f91", foreground: "#3a3215", radius: 0.9 },
  { id: "peach", name: "Peach", words: ["Soft", "Fruity", "Warm", "Gentle"], background: "#fff1ea", surface: "#ffe2d4", primary: "#e8653f", accent: "#ffb199", foreground: "#43281f", radius: 0.9, font: "rounded" },
  {
    id: "royal", name: "Royal", words: ["Regal", "Golden", "Opulent", "Purple"],
    background: "#120e24", surface: "#1b1535", primary: "#f5c542", accent: "#8c6cff", foreground: "#f3eefe",
    glow: 0.3, font: "serif",
  },
  { id: "emerald", name: "Emerald", words: ["Lush", "Jewelled", "Green", "Elegant"], background: "#06150f", surface: "#0b2119", primary: "#2ee59d", accent: "#f5d76e", foreground: "#e2f7ee", glow: 0.25 },
  { id: "ruby", name: "Ruby", words: ["Passionate", "Red", "Jewelled", "Rich"], background: "#18070c", surface: "#240c13", primary: "#ff4d6d", accent: "#ffb3c1", foreground: "#ffe6eb", glow: 0.25 },
  { id: "sapphire", name: "Sapphire", words: ["Deep", "Blue", "Jewelled", "Cool"], background: "#060d1f", surface: "#0b1631", primary: "#3d7bff", accent: "#7ee0ff", foreground: "#e3ecff", glow: 0.25 },
  {
    id: "amethyst", name: "Amethyst", words: ["Mystic", "Violet", "Jewelled", "Glowing"],
    background: "#130a1e", surface: "#1d1030", primary: "#b26bff", accent: "#ff7ad9", foreground: "#f2e6ff",
    glow: 0.6, overlay: glowAt("#b26bff1f", "30% 0%"),
  },
  {
    id: "gold-rush", name: "Gold Rush", words: ["Luxe", "Shiny", "Warm", "Bold"],
    background: "#14110a", surface: "#1e1a10", primary: "#f2c14e", accent: "#e08e45", foreground: "#f7efd9",
    glow: 0.5, overlay: glowAt("#f2c14e14", "100% 0%"),
  },
  { id: "monochrome", name: "Monochrome", words: ["Minimal", "Stark", "Grey", "Clean"], background: "#0d0d0d", surface: "#151515", primary: "#ffffff", accent: "#9a9a9a", foreground: "#f0f0f0", radius: 0.3 },
  { id: "slate", name: "Slate", words: ["Neutral", "Steady", "Cool", "Professional"], background: "#1a1f26", surface: "#212833", primary: "#7aa2c8", accent: "#a3b8cc", foreground: "#e4e9ef" },
  { id: "fjord", name: "Fjord", words: ["Nordic", "Frosty", "Muted", "Serene"], background: "#1f2630", surface: "#27303c", primary: "#7fb8c9", accent: "#a7c4a0", foreground: "#e6ebf0" },
  {
    id: "vaporwave", name: "Vaporwave", words: ["Aesthetic", "Pastel", "Retro", "Surreal"],
    background: "#1c1033", surface: "#271848", primary: "#ff71ce", accent: "#01cdfe", foreground: "#fff0fb",
    opacity: 0.7, blur: 8, glow: 0.7,
    overlay: `linear-gradient(180deg, #1c1033 0%, #3a1c5c 60%, #01cdfe30 100%), ${grid("#01cdfe14", 36)}`,
  },
  {
    id: "aurora", name: "Aurora", words: ["Shimmering", "Polar", "Vivid", "Flowing"],
    background: "#071219", surface: "#0c1d27", primary: "#3dffb0", accent: "#a77bff", foreground: "#e2fbf3",
    opacity: 0.6, blur: 16, glow: 0.6,
    overlay: `${glowAt("#3dffb02e", "20% 0%", "1100px 450px")}, ${glowAt("#a77bff2e", "80% 10%", "1000px 500px")}, ${stars("#ffffff66")}`,
  },
  { id: "desert", name: "Desert", words: ["Sandy", "Dry", "Sunbaked", "Warm"], background: "#241a12", surface: "#2e2218", primary: "#e2a35d", accent: "#c96f4a", foreground: "#f7eadb" },
  {
    id: "retro-terminal", name: "Retro Terminal", words: ["Amber", "Vintage", "Phosphor", "Mono"],
    background: "#0c0a00", surface: "#151200", primary: "#ffb000", accent: "#ffcc4d", foreground: "#ffd98a",
    glow: 0.7, font: "mono", radius: 0.1, overlay: scanlines("#ffb0000a"),
  },
  {
    id: "blueprint", name: "Blueprint", words: ["Technical", "Precise", "Gridded", "Blue"],
    background: "#0b2a4a", surface: "#0f3560", primary: "#ffffff", accent: "#8ec5ff", foreground: "#e6f2ff",
    opacity: 0.85, font: "mono", radius: 0.15, overlay: `${grid("#ffffff10", 24)}, ${grid("#ffffff0a", 120)}`,
  },
  { id: "rose-gold", name: "Rose Gold", words: ["Elegant", "Blush", "Metallic", "Soft"], background: "#1c1416", surface: "#261b1e", primary: "#e8a598", accent: "#f5cdb6", foreground: "#fbefea", glow: 0.2, font: "serif" },
  {
    id: "grape-soda", name: "Grape Soda", words: ["Fizzy", "Purple", "Playful", "Sweet"],
    background: "#1d0d2b", surface: "#29123d", primary: "#c86bff", accent: "#ff9f43", foreground: "#f7e9ff",
    glow: 0.35, radius: 1, font: "rounded",
  },
  { id: "honey", name: "Honey", words: ["Golden", "Sticky", "Warm", "Sweet"], background: "#1a1407", surface: "#241c0b", primary: "#ffb703", accent: "#fb8500", foreground: "#fff3d6", glow: 0.2, radius: 0.9 },
];

export const CUSTOM_WORDS = ["Yours", "Personal", "Anything", "Unique"];
