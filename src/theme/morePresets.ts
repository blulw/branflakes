/**
 * 200 more theme presets (added in 0.6.8), in ten groups of twenty.
 * Written compactly: `t(name, words, background, surface, primary, accent, foreground, options)`.
 */

import { glowAt, grid, scanlines, stars } from "./patterns";
import type { ThemeFont, ThemePreset } from "./themes";

type Fx = "glow" | "dual" | "stars" | "grid" | "scan" | "top";

interface Opts {
  /** glow 0–1 */
  g?: number;
  /** surface opacity */
  o?: number;
  /** blur px */
  b?: number;
  f?: ThemeFont;
  /** radius rem */
  r?: number;
  /** glow uses the accent colour */
  ga?: boolean;
  fx?: Fx;
}

const slug = (name: string) =>
  name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");

function overlay(fx: Fx | undefined, bg: string, primary: string, accent: string): string | undefined {
  switch (fx) {
    case "glow":
      return glowAt(`${primary}22`, "15% 0%");
    case "dual":
      return `${glowAt(`${primary}26`, "0% 0%")}, ${glowAt(`${accent}22`, "100% 100%")}`;
    case "stars":
      return `${stars("#ffffffaa")}, ${glowAt(`${primary}33`, "20% 10%")}, ${glowAt(`${accent}26`, "85% 90%")}`;
    case "grid":
      return grid(`${primary}12`, 28);
    case "scan":
      return scanlines(`${primary}0c`);
    case "top":
      return `linear-gradient(180deg, ${primary}1c 0%, ${bg} 45%)`;
    default:
      return undefined;
  }
}

function t(name: string, words: string, background: string, surface: string, primary: string, accent: string, foreground: string, o: Opts = {}): ThemePreset {
  const p: ThemePreset = { id: slug(name), name, words: words.split(" "), background, surface, primary, accent, foreground };
  if (o.g != null) p.glow = o.g;
  if (o.o != null) p.opacity = o.o;
  if (o.b != null) p.blur = o.b;
  if (o.f) p.font = o.f;
  if (o.r != null) p.radius = o.r;
  if (o.ga) p.glowAccent = true;
  const ov = overlay(o.fx, background, primary, accent);
  if (ov) p.overlay = ov;
  return p;
}

const MUSIC = [
  t("Lo-Fi", "Chill Dusty Mellow Cosy", "#1e1b24", "#26222e", "#e8a87c", "#85cdca", "#ede6df", { r: 0.8, fx: "glow" }),
  t("Jazz Club", "Smoky Brassy Late Smooth", "#141012", "#1e171a", "#d4a24c", "#8c2f39", "#f2e6d8", { f: "serif", g: 0.25, fx: "glow" }),
  t("Blues", "Soulful Deep Moody Blue", "#0c1424", "#131d33", "#4f86f7", "#9fb7e8", "#e3eaf8", { fx: "top" }),
  t("Punk", "Loud Raw Rebellious Pink", "#0d0d0d", "#171717", "#ff2e88", "#f5f500", "#f7f7f7", { r: 0.1, f: "display" }),
  t("Disco", "Glittery Groovy Bright Seventies", "#1a0f24", "#251636", "#ffcc33", "#ff5ea8", "#fff3e0", { g: 0.7, o: 0.75, b: 10, fx: "dual" }),
  t("Reggae", "Sunny Laidback Earthy Rhythmic", "#121a10", "#1a2417", "#f7c600", "#e03a2f", "#f2f0e1", { fx: "glow" }),
  t("Grunge", "Fuzzy Worn Flannel Muted", "#1b1a17", "#24221e", "#a3a86b", "#8a4b3a", "#e6e2d6", { r: 0.2 }),
  t("Techno", "Pulsing Minimal Industrial Dark", "#08090c", "#101217", "#00e5ff", "#7b61ff", "#e8f6fa", { g: 0.8, r: 0.15, f: "mono", fx: "scan" }),
  t("House", "Warm Driving Deep Groovy", "#120c1c", "#1b1229", "#ff7a59", "#c86bfa", "#f6ecf8", { g: 0.45, fx: "dual" }),
  t("Ambient", "Floating Soft Spacious Calm", "#10171c", "#172128", "#8fd3e8", "#b9a7e8", "#e4eef2", { o: 0.65, b: 18, g: 0.25, fx: "dual" }),
  t("Metal", "Heavy Steel Dark Fierce", "#0b0b0c", "#141416", "#c9ccd1", "#b3261e", "#eeeeef", { r: 0.05, f: "display" }),
  t("Funk", "Groovy Bold Orange Fun", "#1c1008", "#28180d", "#ff8c1a", "#ffd23f", "#fff0dc", { r: 1, f: "rounded" }),
  t("Soul", "Warm Velvet Rich Heartfelt", "#1c0f12", "#28161a", "#e5735c", "#f2c078", "#f8e8e2", { f: "serif" }),
  t("Country", "Rustic Denim Dusty Warm", "#1d1914", "#27211a", "#d19a55", "#5f86a8", "#f1e8da", { f: "serif" }),
  t("Hip Hop", "Bold Urban Gold Confident", "#0e0e10", "#18181c", "#f5c542", "#e2463f", "#f5f3ec", { f: "display", g: 0.3, fx: "glow" }),
  t("Drum and Bass", "Fast Neon Rolling Energetic", "#05100d", "#0b1a16", "#39ff88", "#ffe14d", "#e6fff1", { g: 0.9, f: "mono", fx: "dual" }),
  t("Shoegaze", "Hazy Dreamy Washed Pink", "#1d1622", "#271e2e", "#f4a6c9", "#a6c8f4", "#f4ecf6", { o: 0.6, b: 20, g: 0.4, fx: "dual" }),
  t("Opera", "Grand Velvet Gilded Dramatic", "#1a0507", "#26090d", "#e0b04f", "#c22839", "#f8eadc", { f: "serif", g: 0.3, fx: "glow" }),
  t("Folk", "Acoustic Wooden Simple Honest", "#1a1611", "#231e17", "#c9a26b", "#7f9a6a", "#efe7da", { f: "serif" }),
  t("K-Pop", "Glossy Bright Candy Bold", "#fff0f7", "#ffffff", "#e91e8c", "#5b4bff", "#2b1530", { r: 1, f: "rounded", o: 0.85, b: 10, fx: "dual" }),
];

const NATURE = [
  t("Rainforest", "Lush Humid Green Alive", "#07150e", "#0d2016", "#3ccf7a", "#f2c14e", "#e3f4e8", { fx: "glow" }),
  t("Glacier", "Icy Clear Blue Still", "#0c1b24", "#122733", "#7fe0ff", "#c4f1ff", "#e6f7fc", { o: 0.7, b: 14, fx: "top" }),
  t("Tundra", "Sparse Cold Pale Quiet", "#1c2224", "#242c2f", "#9cc0b4", "#c9b79c", "#e8eeee"),
  t("Volcano", "Fiery Molten Black Intense", "#120807", "#1d0d0b", "#ff5a1f", "#ffb300", "#fbe7dc", { g: 0.7, fx: "dual" }),
  t("Canyon", "Red Rocky Vast Sunbaked", "#20120c", "#2c1911", "#e06c3c", "#f0b67f", "#f7e6da"),
  t("Savanna", "Golden Wild Open Warm", "#1d180b", "#282111", "#e3b23c", "#a8763e", "#f6efd9"),
  t("Bamboo", "Fresh Calm Green Zen", "#0f1a0e", "#162515", "#8fd16a", "#d8e8a0", "#eaf5e1", { r: 0.8 }),
  t("Coral Reef", "Vivid Tropical Underwater Playful", "#06202a", "#0b2c39", "#ff7f6b", "#3ee0d0", "#e2f8f8", { o: 0.75, b: 12, fx: "dual" }),
  t("Lagoon", "Turquoise Clear Warm Tropical", "#062224", "#0b2f32", "#2fe0c9", "#ffd38a", "#e0faf6", { fx: "glow" }),
  t("Meadow", "Bright Grassy Fresh Light", "#f1f7e8", "#ffffff", "#3f8a2e", "#e0a526", "#1f2b18", { r: 0.8 }),
  t("Redwood", "Towering Earthy Deep Ancient", "#170f0b", "#22160f", "#c4643a", "#7a9a5a", "#f2e3d8"),
  t("Marsh", "Murky Reedy Damp Muted", "#141810", "#1c2117", "#9aa86a", "#6b8f8a", "#e4e8d8"),
  t("Dunes", "Rolling Sandy Soft Golden", "#f6ead6", "#fffaf1", "#b5651d", "#d9a05b", "#3a2712", { r: 0.9 }),
  t("Highlands", "Misty Heather Rugged Wild", "#151619", "#1d1f24", "#a77bca", "#7fa37a", "#e8e6ee", { fx: "top" }),
  t("Bayou", "Sultry Mossy Dark Mysterious", "#0d1410", "#141e18", "#7fbf7f", "#d9a441", "#e3ece4", { g: 0.3, fx: "glow" }),
  t("Cliffside", "Windswept Stony Coastal Bold", "#141a1f", "#1c242b", "#5fa8d3", "#d9cbb3", "#e7edf1"),
  t("Waterfall", "Rushing Misty Cool Fresh", "#0a1a1e", "#10262b", "#5ee0e6", "#9be3a8", "#e2f6f6", { o: 0.7, b: 16, fx: "top" }),
  t("Prairie", "Open Golden Breezy Gentle", "#1c1a10", "#262317", "#d6b85a", "#8fae6b", "#f2eedb"),
  t("Mangrove", "Tangled Brackish Green Deep", "#0b1612", "#11201b", "#56c596", "#c0a36e", "#e1f0e8"),
  t("Alpine", "Crisp Snowy Pine Clean", "#eef3f6", "#ffffff", "#2a6f97", "#3f8f5f", "#16232c"),
];

const LIGHT = [
  t("Cloud", "Airy White Soft Calm", "#eef2f7", "#ffffff", "#4a7bd0", "#8fb3e8", "#1d2633", { o: 0.8, b: 14, r: 0.9 }),
  t("Linen", "Natural Woven Warm Simple", "#f3eee6", "#fbf8f3", "#8a6a45", "#b8956a", "#2e261c", { f: "serif" }),
  t("Porcelain", "Delicate Cool Smooth Pale", "#f2f5f8", "#ffffff", "#2f5d8c", "#7a9cc0", "#1a2430"),
  t("Seashell", "Pearly Pink Gentle Coastal", "#fbefea", "#fffaf7", "#c05a4e", "#e8a598", "#3a221d", { r: 0.9 }),
  t("Vanilla", "Creamy Sweet Mellow Light", "#fbf5e6", "#fffdf6", "#b07a1f", "#d9a95b", "#33281a", { f: "rounded" }),
  t("Pistachio", "Nutty Green Fresh Light", "#eef5e6", "#fbfdf7", "#4f7d2b", "#a3c76b", "#1f2a17"),
  t("Sky", "Bright Blue Open Light", "#e8f3fc", "#ffffff", "#1c6fc2", "#5fb0f0", "#12263a", { fx: "top" }),
  t("Lilac Morning", "Soft Purple Fresh Light", "#f3eefa", "#fdfbff", "#7b4fc2", "#c59be8", "#26193a", { r: 0.9 }),
  t("Parchment", "Old Inked Warm Scholarly", "#f2e8d5", "#f9f2e4", "#7a4a1d", "#a6713f", "#2c2012", { f: "serif", r: 0.3 }),
  t("Cotton", "Fluffy Soft Neutral Light", "#f5f4f2", "#ffffff", "#5d6b7a", "#b5a99a", "#22252a", { r: 1, f: "rounded" }),
  t("Oat Milk", "Beige Cosy Calm Light", "#efe9e0", "#faf7f2", "#8a6b4e", "#a89a84", "#2b241d"),
  t("Sage Light", "Herbal Muted Calm Light", "#eaf0e8", "#f8faf6", "#4c7155", "#9ab49a", "#1d2a20"),
  t("Daylight", "Clean Bright Crisp Neutral", "#f7f8fa", "#ffffff", "#2563eb", "#14b8a6", "#111827"),
  t("Blossom", "Pink Floral Spring Light", "#fdeef3", "#fffafc", "#c2367a", "#f28bb5", "#36152a", { r: 0.9, fx: "dual" }),
  t("Lemon Sorbet", "Zesty Yellow Fresh Light", "#fdf9e1", "#fffef5", "#a07c00", "#e0b400", "#2e2a12", { f: "rounded" }),
  t("Ice Blue", "Frosty Pale Cool Light", "#eaf6fa", "#ffffff", "#137c9c", "#7fd0e6", "#10282f", { o: 0.8, b: 12 }),
  t("Sandstone", "Warm Earthy Layered Light", "#f1e4d4", "#faf2e9", "#a3542b", "#c98d5a", "#33200f"),
  t("Chalk", "Matte Grey Plain Light", "#ececea", "#f8f8f6", "#3f4a55", "#8a949e", "#1c1f22", { r: 0.2 }),
  t("Pearl", "Lustrous Soft Elegant Light", "#f4f1f4", "#ffffff", "#8a5a8f", "#b9a6c9", "#2a2030", { f: "serif", o: 0.85, b: 10 }),
  t("Apricot", "Peachy Warm Juicy Light", "#fdeee2", "#fff9f4", "#c4561d", "#f2a36b", "#3a1f10", { r: 0.9 }),
];

const FOOD = [
  t("Espresso", "Strong Dark Roasted Rich", "#130d09", "#1d140e", "#c88a55", "#e8c9a4", "#f1e6dc"),
  t("Matcha", "Earthy Green Whisked Calm", "#141a10", "#1c2417", "#a3c45a", "#e3e0c2", "#ecf1e1", { r: 0.8 }),
  t("Chai", "Spiced Milky Warm Cosy", "#1c140e", "#271c14", "#d69a5e", "#b5654a", "#f5e9dd", { f: "serif" }),
  t("Blueberry", "Juicy Purple Blue Sweet", "#0f0f24", "#171734", "#6f74ff", "#b07bff", "#e8e8fb", { r: 0.9, fx: "glow" }),
  t("Raspberry", "Tart Pink Bright Juicy", "#1d0a12", "#2a0f1b", "#ff4f8b", "#ffa3c2", "#fbe7ee", { r: 0.9 }),
  t("Mango", "Tropical Ripe Orange Sunny", "#1d1206", "#2a1a09", "#ffa62b", "#ffd166", "#fff0d9", { f: "rounded" }),
  t("Watermelon", "Fresh Pink Green Summery", "#12190f", "#1a2316", "#ff5c75", "#6fd36f", "#f3f7ee", { r: 1, f: "rounded" }),
  t("Licorice", "Black Glossy Sharp Bold", "#0a0a0a", "#141414", "#e0343d", "#5c5c5c", "#efefef", { r: 0.3 }),
  t("Caramel", "Golden Buttery Sweet Smooth", "#1c1208", "#28190b", "#e0a050", "#f2cf94", "#f8ecdc", { r: 0.9 }),
  t("Cola", "Fizzy Dark Red Classic", "#140806", "#200d0a", "#e23b2e", "#c98a5a", "#f6e6e0"),
  t("Tiramisu", "Creamy Cocoa Layered Italian", "#1b1511", "#261d18", "#c9a27e", "#7a4e33", "#f3e9e0", { f: "serif" }),
  t("Pumpkin Spice", "Autumnal Spiced Orange Cosy", "#1d1209", "#29190d", "#f08a24", "#c0603b", "#f8e8d8"),
  t("Mint Choc", "Cool Minty Chocolate Sweet", "#16110e", "#211915", "#7be0b5", "#a5704f", "#eef6f0", { r: 0.9 }),
  t("Cherry Cola", "Fizzy Sweet Red Retro", "#1a070c", "#260a12", "#ff3d68", "#ffb3c4", "#fbe7ec", { g: 0.35, fx: "glow" }),
  t("Red Wine", "Rich Velvet Deep Elegant", "#17070b", "#230b11", "#c23b5a", "#e8b4a0", "#f6e5e8", { f: "serif" }),
  t("Whisky", "Smoky Amber Oaky Warm", "#160f07", "#22170b", "#d98e32", "#8c5a2b", "#f5e8d4", { f: "serif", g: 0.2, fx: "glow" }),
  t("Sorbet", "Fruity Pastel Cool Sweet", "#fdf0f2", "#ffffff", "#d6336c", "#f59f00", "#3a1a24", { r: 1, f: "rounded", fx: "dual" }),
  t("Kiwi", "Zingy Green Fresh Tangy", "#10170a", "#18220f", "#9be15d", "#7a5a2e", "#eef6e4"),
  t("Blackcurrant", "Dark Purple Tart Rich", "#160a1a", "#211026", "#c254d6", "#7fdb8a", "#f3e7f5"),
  t("Dark Chocolate", "Bitter Rich Smooth Deep", "#100a08", "#1a110d", "#b9774e", "#e6b98f", "#efe3da", { r: 0.7 }),
];

const GEMS = [
  t("Onyx", "Black Polished Sleek Minimal", "#09090b", "#121215", "#d6d6dc", "#6b6b78", "#efeff2", { r: 0.3 }),
  t("Jade", "Green Smooth Precious Calm", "#08180f", "#0e2418", "#3fbf7f", "#a8e0c0", "#e2f5ea", { g: 0.25, fx: "glow" }),
  t("Topaz", "Golden Warm Sparkling Rich", "#1a1206", "#261a09", "#ffb347", "#ffd98a", "#fff1dc", { g: 0.45, fx: "glow" }),
  t("Opal", "Iridescent Milky Shifting Soft", "#16151f", "#1f1d2b", "#8fe3d6", "#f4a6d0", "#f0eef8", { o: 0.6, b: 18, g: 0.45, fx: "dual" }),
  t("Garnet", "Deep Red Jewelled Rich", "#16060a", "#220a10", "#d8344f", "#f2a65a", "#f7e4e7", { g: 0.3, fx: "glow" }),
  t("Obsidian", "Volcanic Glassy Dark Sharp", "#07070a", "#0f0f14", "#8f7bff", "#3c3c4a", "#e9e8f2", { r: 0.2, g: 0.3 }),
  t("Turquoise", "Bright Blue Green Bold", "#061b1d", "#0b2729", "#29d3c9", "#f5b86b", "#e1f8f6"),
  t("Moonstone", "Pale Glowing Mystic Soft", "#151922", "#1d2230", "#a9c1ff", "#e3e7f5", "#eceff8", { o: 0.65, b: 16, g: 0.45, fx: "glow" }),
  t("Amber", "Golden Fossil Warm Glowing", "#170f04", "#231705", "#ff9f1c", "#ffbf69", "#fdecd5", { g: 0.6, fx: "glow" }),
  t("Quartz", "Rosy Crystal Clear Soft", "#1c1519", "#271d23", "#f2a7c3", "#e8dbe1", "#f7eef2", { o: 0.7, b: 14 }),
  t("Peridot", "Lime Bright Fresh Crystal", "#11170a", "#19220f", "#b5e550", "#e6f59a", "#f0f7e3", { g: 0.4, fx: "glow" }),
  t("Tanzanite", "Violet Blue Rare Deep", "#0d0b22", "#151233", "#7d6bff", "#4fa3ff", "#e8e6fb", { g: 0.45, fx: "dual" }),
  t("Malachite", "Banded Green Bold Rich", "#061710", "#0b2319", "#21b573", "#8de0b5", "#e0f5eb", { fx: "grid" }),
  t("Copper", "Metallic Warm Burnished Rustic", "#170e0a", "#22150f", "#d9804f", "#59b6a8", "#f5e6dc"),
  t("Bronze", "Antique Metallic Warm Earthy", "#16120c", "#211b12", "#c59a5b", "#8a6a3b", "#f2eadc", { f: "serif" }),
  t("Platinum", "Silvery Cool Premium Sleek", "#17191c", "#202328", "#dfe4ea", "#9aa6b2", "#f2f4f6", { g: 0.2, fx: "top" }),
  t("Graphite", "Grey Matte Technical Quiet", "#18191b", "#202224", "#9fb0c0", "#6e7a86", "#e6e8ea", { r: 0.25 }),
  t("Marble", "Veined White Classic Elegant", "#f2f1ef", "#ffffff", "#4a4f57", "#a8a29a", "#1d1f22", { f: "serif" }),
  t("Lapis", "Royal Blue Gold Ancient", "#0a1030", "#111944", "#3a63e8", "#e8c25a", "#e5e9fa", { g: 0.3, fx: "dual" }),
  t("Citrine", "Sunny Yellow Bright Warm", "#191506", "#241e09", "#ffd23f", "#ff9f43", "#fff6d6", { g: 0.4 }),
];

const WEATHER = [
  t("Thunderstorm", "Electric Dark Dramatic Charged", "#0b0e16", "#121725", "#c7d2ff", "#ffe066", "#e6eaf7", { g: 0.6, ga: true, fx: "dual" }),
  t("Monsoon", "Heavy Wet Grey Green", "#0f1714", "#16211c", "#5fbf99", "#8fa8b8", "#e2ede8"),
  t("Fog", "Hazy Grey Soft Muffled", "#1c1f22", "#24282c", "#b4c0cb", "#8c97a1", "#e8ecef", { o: 0.6, b: 22 }),
  t("Drizzle", "Grey Cool Quiet Damp", "#151a20", "#1d242c", "#79a7d3", "#a9b8c6", "#e3e9ef"),
  t("Heatwave", "Scorching Red Orange Hazy", "#1c0b06", "#290f08", "#ff5e3a", "#ffc23a", "#fde8dd", { g: 0.6, fx: "top" }),
  t("Blizzard", "White Whirling Cold Bright", "#e9eef2", "#f8fafc", "#3b6e8f", "#9fb8c8", "#16222b", { o: 0.75, b: 16 }),
  t("Spring Rain", "Fresh Green Gentle Damp", "#0f1915", "#16231e", "#6fdc9f", "#9cc6f0", "#e3f3ea"),
  t("Midsummer", "Bright Long Golden Happy", "#fff7e3", "#fffdf6", "#c46a00", "#3f9d4a", "#33240a", { r: 0.9 }),
  t("Harvest", "Golden Ripe Rustic Plenty", "#1b1408", "#271d0c", "#e5a83a", "#b0532d", "#f6ecd9"),
  t("Winter Solstice", "Long Night Cold Still", "#070b16", "#0d1322", "#9ec5ff", "#e8d28a", "#e5ecf8", { fx: "stars", o: 0.7, b: 12 }),
  t("Golden Hour", "Glowing Warm Soft Cinematic", "#1d120a", "#2a1a0f", "#ffb35c", "#ff7a6b", "#fdeedd", { g: 0.5, fx: "dual" }),
  t("Blue Hour", "Twilight Cool Calm Deep", "#0c1226", "#141c38", "#7f9cff", "#f2a8c8", "#e6ebfb", { fx: "top" }),
  t("Dusk", "Fading Purple Warm Quiet", "#1a1224", "#241932", "#e58fb0", "#ffb37a", "#f6ebf3", { fx: "dual" }),
  t("Dawn Chorus", "Early Bright Hopeful Soft", "#fff2ea", "#fffaf6", "#d9534f", "#f0ad4e", "#3a2420", { fx: "top" }),
  t("Twilight", "Violet Dim Dreamy Calm", "#120f22", "#1b1732", "#a98bff", "#ff9ec7", "#ece8fb", { fx: "stars", o: 0.75, b: 10 }),
  t("Overcast", "Grey Flat Calm Neutral", "#202326", "#282c30", "#a0aab4", "#c9c2b6", "#eaecee"),
  t("Rainbow", "Colourful Bright Joyful Bold", "#141220", "#1d1a2e", "#ff6b6b", "#4ecdc4", "#f5f3fb", { r: 1, f: "rounded", g: 0.4, fx: "dual" }),
  t("Hailstorm", "Icy Sharp Grey Fierce", "#11161b", "#182027", "#d7e6f2", "#6fa0c8", "#eaf1f6", { r: 0.2 }),
  t("Frost", "Crisp Sparkling Pale Cold", "#0f1a22", "#16252f", "#b8ecff", "#e1f6ff", "#e8f6fb", { o: 0.65, b: 16, g: 0.3, fx: "glow" }),
  t("Polar Night", "Endless Dark Blue Quiet", "#05080f", "#0b1019", "#5fa8ff", "#7ff0d0", "#e2eaf5", { fx: "stars", o: 0.7, b: 12 }),
];

const SPACE = [
  t("Nebula", "Swirling Pink Cosmic Vivid", "#0d0718", "#170d29", "#ff6bd6", "#7b8cff", "#f3e9fb", { o: 0.6, b: 14, g: 0.6, fx: "stars" }),
  t("Supernova", "Explosive Bright Hot White", "#0f0805", "#1a0e08", "#ffd166", "#ff6b3d", "#fff3e6", { g: 0.9, fx: "dual" }),
  t("Black Hole", "Void Dark Heavy Infinite", "#030305", "#0a0a0e", "#ff9e3d", "#6f5cff", "#e9e7ef", { g: 0.5, fx: "glow" }),
  t("Mars", "Red Dusty Barren Bold", "#1a0c08", "#26120c", "#e8643c", "#d9a07a", "#f8e4dc"),
  t("Jupiter", "Banded Stormy Giant Warm", "#1b140f", "#271d16", "#e0a56e", "#c9573c", "#f6ece2", { fx: "grid" }),
  t("Saturn Rings", "Golden Ringed Elegant Vast", "#12100c", "#1c1912", "#e8cf8a", "#a6b8d8", "#f4efe2", { fx: "stars" }),
  t("Moonbase", "Lunar Grey Clinical Quiet", "#141619", "#1c1f23", "#e6e6e6", "#7fc8ff", "#ececec", { f: "mono", r: 0.2, fx: "grid" }),
  t("Comet", "Streaking Icy Blue Fast", "#060c18", "#0c1526", "#6fd6ff", "#ffffff", "#e3f2fb", { g: 0.6, fx: "stars" }),
  t("Starship", "Sleek Command Blue Futuristic", "#080d14", "#0e1520", "#3fa9ff", "#ff9f40", "#e4edf7", { f: "display", r: 0.2, g: 0.4, fx: "grid" }),
  t("Hologram", "Shimmering Cyan Translucent Future", "#06121a", "#0b1d28", "#5ff7ff", "#ff7bf2", "#e2fbff", { o: 0.5, b: 18, g: 0.8, fx: "scan" }),
  t("Plasma", "Charged Magenta Hot Electric", "#12051a", "#1d0a29", "#ff3dd5", "#7a5cff", "#fbe6f8", { g: 1, fx: "dual" }),
  t("Quasar", "Blazing Distant Bright Blue", "#05081a", "#0b102a", "#8fb3ff", "#ffe08a", "#e6ecfb", { g: 0.7, fx: "stars" }),
  t("Event Horizon", "Warped Dark Orange Edge", "#050304", "#0e090a", "#ff7a2f", "#ffd59a", "#f5e8e0", { g: 0.6, fx: "top" }),
  t("Andromeda", "Spiral Distant Violet Vast", "#090818", "#121026", "#b59cff", "#7fd0ff", "#ebe8fb", { fx: "stars", o: 0.65, b: 12 }),
  t("Solar Flare", "Blazing Orange Radiant Hot", "#170904", "#240e06", "#ff8a1f", "#ffe14d", "#fff0dc", { g: 0.8, fx: "glow" }),
  t("Orbit", "Circling Clean Blue Calm", "#0a1220", "#111b2e", "#59a5ff", "#a3f0d8", "#e5edf9", { r: 1 }),
  t("Pulsar", "Beating Teal Rhythmic Bright", "#041214", "#091d20", "#21f0d0", "#ff5fa2", "#dffaf5", { g: 0.8, fx: "dual" }),
  t("Cosmic Dust", "Sparkly Muted Mauve Soft", "#16121c", "#201a28", "#d9a7e0", "#a8b8e6", "#f1ebf5", { fx: "stars" }),
  t("Stardust", "Glittering Gold Night Magical", "#0a0912", "#13111f", "#f2d27a", "#c9a7ff", "#f4efe2", { fx: "stars", g: 0.4 }),
  t("Eclipse", "Shadowed Ring Gold Dark", "#060507", "#0f0d11", "#ffcf66", "#ff6e4a", "#efe9e2", { g: 0.7, fx: "glow" }),
];

const RETRO = [
  t("Arcade", "Pixel Neon Loud Fun", "#0a0716", "#140f26", "#ffe600", "#ff2fa0", "#f5f0ff", { f: "mono", r: 0.1, g: 0.8, fx: "scan" }),
  t("Pocket Console", "Green Pixel Handheld Retro", "#1b2a16", "#22351c", "#9bbc0f", "#8bac0f", "#e0f0c0", { f: "mono", r: 0.1 }),
  t("CRT", "Glowing Curved Scanline Vintage", "#071006", "#0c190b", "#5cff6e", "#b4ffbf", "#c8ffd0", { f: "mono", g: 0.8, fx: "scan" }),
  t("Eighties Mall", "Pastel Neon Shiny Retro", "#1d1430", "#281c42", "#4fe0d0", "#ff7ac6", "#f4eeff", { f: "display", g: 0.5, fx: "grid" }),
  t("Y2K", "Glossy Chrome Bubbly Futurist", "#e9ecf5", "#ffffff", "#4a3cff", "#ff4fb0", "#1c1a33", { r: 1, f: "rounded", o: 0.8, b: 12, fx: "dual" }),
  t("Chrome", "Metallic Reflective Cool Sleek", "#121417", "#1b1e22", "#cfd8e3", "#7fb0ff", "#eef2f6", { g: 0.3, fx: "top" }),
  t("Polaroid", "Faded Warm Snapshot Nostalgic", "#f4efe4", "#fffcf5", "#c2582b", "#5f8a8b", "#2c261c", { r: 0.2 }),
  t("VHS", "Fuzzy Tracking Retro Warm", "#120d14", "#1b141f", "#ff5f6d", "#5fd3ff", "#f2e8ef", { f: "mono", fx: "scan" }),
  t("Cassette", "Mixtape Brown Orange Analog", "#1a140e", "#251c14", "#ff8f3d", "#d9c39a", "#f4ead9", { f: "display", r: 0.3 }),
  t("Jukebox", "Chrome Red Glowing Diner", "#170708", "#230b0d", "#ff4040", "#ffcf4d", "#fbe8e6", { g: 0.6, r: 1, fx: "dual" }),
  t("Pinball", "Flashing Busy Bright Loud", "#0c0a18", "#151229", "#ff9f1a", "#22d3ff", "#f4f2fb", { g: 0.7, fx: "dual" }),
  t("Dot Matrix", "Printed Tractor Feed Plain", "#efeee6", "#f9f8f2", "#2f3a2f", "#8a9a6a", "#1c201b", { f: "mono", r: 0.1, fx: "scan" }),
  t("Teletext", "Blocky Bold Primary Retro", "#000000", "#0a0a14", "#ffff00", "#00ffff", "#ffffff", { f: "mono", r: 0 }),
  t("Beige Box", "Nineties Office Plastic Grey", "#d8d4c8", "#e8e5dc", "#0a246a", "#3a6ea5", "#1a1a1a", { r: 0, f: "system" }),
  t("Pixel", "Blocky Bright Playful Retro", "#141a2e", "#1c2440", "#ff6188", "#a9dc76", "#f2f2f2", { f: "mono", r: 0, fx: "grid" }),
  t("Glitch", "Broken Shifted Neon Chaotic", "#0a0a0f", "#13131b", "#ff2d6f", "#2dffe1", "#f2f2f8", { g: 0.8, f: "mono", r: 0.1, fx: "scan" }),
  t("Hacker", "Terminal Green Dark Focused", "#030a05", "#07130a", "#33ff66", "#1f9e45", "#c6f7d2", { f: "mono", r: 0.1, g: 0.5 }),
  t("Mainframe", "Blue Text Institutional Retro", "#00102a", "#001a40", "#7fb2ff", "#ffffff", "#d6e6ff", { f: "mono", r: 0.05 }),
  t("Radar", "Sweeping Green Military Dark", "#041008", "#081a0e", "#4dff88", "#a8ffc4", "#d2f8de", { g: 0.6, f: "mono", fx: "grid" }),
  t("Circuit Board", "Green Traced Technical Gold", "#06150c", "#0b2014", "#e8c25a", "#3fcf7a", "#e1f2e6", { f: "mono", fx: "grid" }),
];

const PLACES = [
  t("Tokyo Night", "Neon Rainy Urban Electric", "#0b0a16", "#141226", "#ff4f9a", "#45c7ff", "#efeefb", { g: 0.7, fx: "dual" }),
  t("Paris", "Romantic Chic Soft Classic", "#1a1518", "#241d21", "#e8a3b8", "#c9b37e", "#f6eef1", { f: "serif" }),
  t("Havana", "Colourful Warm Faded Lively", "#1c120d", "#281a13", "#ff7a59", "#3ec9b2", "#f8ebe2", { r: 0.8 }),
  t("Marrakech", "Spiced Terracotta Ornate Warm", "#1c0e09", "#29150d", "#e06030", "#2fa3a3", "#f8e6da", { fx: "grid" }),
  t("Reykjavik", "Cool Volcanic Nordic Clean", "#12161a", "#1a2025", "#6fd0e0", "#e86f5a", "#e8eef1"),
  t("Santorini", "White Blue Sunny Coastal", "#eef5fb", "#ffffff", "#1356a8", "#f2a63a", "#13233a", { r: 0.9 }),
  t("Kyoto", "Temple Red Calm Traditional", "#161011", "#211718", "#d6453d", "#d9b77a", "#f5ebe6", { f: "serif" }),
  t("Venice", "Canal Teal Faded Grand", "#0e1716", "#152221", "#4fb5a8", "#e0a86a", "#ebf2ef", { f: "serif" }),
  t("Cairo", "Desert Gold Ancient Warm", "#1a1408", "#251d0c", "#e5b04a", "#3f8fb5", "#f7eedb"),
  t("Rio", "Carnival Vibrant Green Sunny", "#081a10", "#0e2618", "#ffd23f", "#20c997", "#eef8ee", { r: 0.9, g: 0.3, fx: "dual" }),
  t("Seoul", "Modern Neon Sleek Bright", "#0d0f16", "#151824", "#5ca8ff", "#ff6fa3", "#eceff7", { g: 0.4 }),
  t("Istanbul", "Mosaic Turquoise Rich Gold", "#0a1518", "#102024", "#2fb8c8", "#e8b14a", "#e7f3f4", { fx: "grid" }),
  t("Lisbon", "Tiled Sunny Yellow Blue", "#0f1626", "#162038", "#ffcc4d", "#4f8fff", "#eef1f8"),
  t("Mumbai", "Vibrant Saffron Busy Warm", "#1a0e06", "#271509", "#ff9933", "#d63384", "#fbeadb", { g: 0.3, fx: "glow" }),
  t("Swansea Bay", "Coastal Welsh Breezy Blue", "#0c1820", "#13232e", "#4fb3d9", "#e8c87a", "#e6f1f6", { fx: "top" }),
  t("Highland Loch", "Still Dark Peaty Deep", "#0c1214", "#131b1e", "#7fb5b0", "#b08fc7", "#e3ecec"),
  t("Copenhagen", "Hygge Minimal Muted Warm", "#f1ede8", "#faf8f5", "#b04a3a", "#5a7a8a", "#24201c", { r: 0.4 }),
  t("New Orleans", "Jazzy Purple Gold Festive", "#160c1f", "#21122d", "#9b5de5", "#f2c94c", "#f3ecf8", { g: 0.35, fx: "dual" }),
  t("Bali", "Tropical Lush Temple Warm", "#0b1810", "#112418", "#3fcf8e", "#ff9f5a", "#e6f5ec", { r: 0.9 }),
  t("Sahara Night", "Desert Starry Cool Vast", "#0d0b10", "#16131b", "#f0b45a", "#8f9cff", "#f2ece4", { fx: "stars" }),
];

const FLOWERS = [
  t("Lotus", "Serene Pink Floating Calm", "#18121a", "#221a25", "#f29ec0", "#9fe0c8", "#f5ecf3", { r: 0.9, fx: "glow" }),
  t("Orchid", "Exotic Purple Elegant Rare", "#170c1c", "#221229", "#d46bf0", "#f5b5e8", "#f6e9f8", { f: "serif", g: 0.3, fx: "glow" }),
  t("Poppy", "Bold Red Bright Wild", "#1a0807", "#260c0a", "#ff4433", "#2b2b2b", "#fbe7e4"),
  t("Sunflower", "Sunny Yellow Cheerful Tall", "#1a1606", "#252009", "#ffcc00", "#8a5a1e", "#fff6d6", { r: 1, f: "rounded" }),
  t("Bluebell", "Woodland Blue Delicate Spring", "#0e1124", "#151a35", "#7d8cff", "#a8d08d", "#e8eafb"),
  t("Tulip", "Bright Spring Red Cheerful", "#fff0ee", "#fffaf9", "#d6303f", "#3f9d4a", "#3a1a1e", { r: 0.9 }),
  t("Magnolia", "Creamy Soft Elegant Pink", "#f8efee", "#fffaf9", "#b0466a", "#d9a6b5", "#33202a", { f: "serif" }),
  t("Wisteria", "Hanging Lilac Dreamy Soft", "#16121f", "#201a2d", "#b9a3ff", "#e3c3ff", "#efeaf8", { o: 0.7, b: 14, fx: "dual" }),
  t("Hibiscus", "Tropical Hot Pink Bold", "#1a0812", "#270c1b", "#ff3d9a", "#ffb03d", "#fbe7f0", { g: 0.35 }),
  t("Iris", "Violet Regal Cool Elegant", "#100d20", "#181430", "#8f6bff", "#ffd166", "#ebe7fb", { f: "serif" }),
  t("Daffodil", "Spring Yellow Bright Welsh", "#141a0c", "#1c2412", "#ffd93d", "#8fd16a", "#f6f6e2", { r: 0.9 }),
  t("Heather", "Moorland Purple Soft Wild", "#171219", "#211a24", "#c58fd6", "#9aa77a", "#f1eaf3"),
  t("Peony", "Lush Pink Romantic Full", "#1c0f15", "#28161f", "#ff8fb8", "#ffd1dc", "#fbedf2", { r: 1, f: "rounded" }),
  t("Marigold", "Orange Festive Warm Bright", "#1c1006", "#291809", "#ff9f1c", "#e85d04", "#fff0dc", { g: 0.3, fx: "glow" }),
  t("Dreamy", "Soft Floating Hazy Pastel", "#1a1724", "#232032", "#c9b3ff", "#ffc3dd", "#f3f0fa", { o: 0.55, b: 22, g: 0.35, r: 1, fx: "dual" }),
  t("Melancholy", "Blue Grey Wistful Quiet", "#12151c", "#1a1e27", "#7f93b5", "#a59ab8", "#e2e6ee", { f: "serif" }),
  t("Zen", "Balanced Minimal Stone Calm", "#e9e7e1", "#f6f5f1", "#5a6b5a", "#a39b8b", "#22241f", { r: 0.5 }),
  t("Rebel", "Defiant Black Red Loud", "#0b0707", "#160d0d", "#ff3b3b", "#f2f2f2", "#f5eaea", { r: 0.1, f: "display", g: 0.4 }),
  t("Cozy", "Warm Snug Soft Homely", "#1c1512", "#271d19", "#f0a37a", "#c9b38f", "#f6ece5", { r: 1, f: "rounded" }),
  t("Chill", "Relaxed Cool Teal Easy", "#0e1a1d", "#152529", "#6fd3c8", "#b5a8f0", "#e4f2f1", { r: 0.9, fx: "glow" }),
];

export const MORE_PRESETS: ThemePreset[] = [...MUSIC, ...NATURE, ...LIGHT, ...FOOD, ...GEMS, ...WEATHER, ...SPACE, ...RETRO, ...PLACES, ...FLOWERS];
