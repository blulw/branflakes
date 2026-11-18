import { useEffect, useMemo, useRef, useState } from "react";
import { Check, ImagePlus, Palette, RotateCcw, Search, Sparkles, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import { FONTS, isLight, previewStyle, resolveCustom, resolvePreset, type ResolvedTheme } from "@/theme/engine";
import { useTheme } from "@/theme/store";
import { CUSTOM_THEME_ID, CUSTOM_WORDS, DEFAULT_CUSTOM, PRESETS, type CustomTheme, type ThemeFont } from "@/theme/themes";

const alphaOf = (c: string, a: number) => `color-mix(in srgb, ${c} ${Math.round(a * 100)}%, transparent)`;

function Swatches({ t }: { t: ResolvedTheme }) {
  const colours: [string, string][] = [
    ["Background", t.background],
    ["Surface", t.surface],
    ["Primary", t.primary],
    ["Accent", t.accent],
    ["Text", t.foreground],
  ];
  return (
    <div className="flex gap-1">
      {colours.map(([label, c]) => (
        <span key={label} title={`${label} ${c}`} className="size-4 rounded-full border border-foreground/20" style={{ background: c }} />
      ))}
    </div>
  );
}

/** A tiny mock of the app painted in the theme. */
function Preview({ t }: { t: ResolvedTheme }) {
  const glow = t.glow > 0 ? `0 0 ${t.glow * 10}px ${alphaOf(t.glowAccent ? t.accent : t.primary, 0.7)}` : undefined;
  const r = `${Math.min(t.radius, 1) * 0.5}rem`;
  return (
    <div className="relative flex h-20 overflow-hidden rounded-md" style={previewStyle(t)}>
      <div className="flex w-1/4 flex-col gap-1 p-1.5" style={{ background: alphaOf(t.surface, t.opacity), backdropFilter: t.opacity < 1 ? "blur(6px)" : undefined }}>
        <span className="h-1.5 w-4/5 rounded-full" style={{ background: t.primary, boxShadow: glow }} />
        <span className="h-1 w-3/5 rounded-full opacity-50" style={{ background: t.foreground }} />
        <span className="h-1 w-2/3 rounded-full opacity-50" style={{ background: t.foreground }} />
      </div>
      <div className="flex flex-1 flex-col gap-1.5 p-2">
        <span className="h-1.5 w-1/2 rounded-full opacity-90" style={{ background: t.foreground }} />
        <span className="h-1 w-3/4 rounded-full opacity-40" style={{ background: t.foreground }} />
        <div className="mt-auto flex items-center gap-1.5">
          <span className="h-3.5 w-10" style={{ background: t.primary, borderRadius: r, boxShadow: glow }} />
          <span className="size-3.5 rounded-full" style={{ background: t.accent }} />
        </div>
      </div>
    </div>
  );
}

function ThemeTile({ name, words, t, active, onSelect }: { name: string; words: string[]; t: ResolvedTheme; active: boolean; onSelect: () => void }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onSelect}
      className={cn(
        "group flex flex-col gap-2 rounded-lg border bg-card p-2 text-left transition-colors hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none",
        active && "border-primary ring-1 ring-primary",
      )}
    >
      <Preview t={t} />
      <div className="flex items-center gap-2 px-0.5">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">{name}</span>
        {t.glow > 0 && <Sparkles className="size-3.5 text-muted-foreground" aria-label="Glow" />}
        {active && <Check className="size-4 text-primary" aria-label="Selected" />}
      </div>
      <div className="flex items-center justify-between gap-2 px-0.5">
        <span className="truncate text-xs text-muted-foreground">{words.join(" · ")}</span>
        <Swatches t={t} />
      </div>
    </button>
  );
}

export function ThemeSettings() {
  const { id, custom, image, setTheme, customiseFrom } = useTheme();
  const [query, setQuery] = useState("");
  const presets = useMemo(() => PRESETS.map((p) => ({ p, t: resolvePreset(p) })), []);
  const q = query.trim().toLowerCase();
  const shown = presets.filter(
    ({ p, t }) =>
      !q ||
      p.name.toLowerCase().includes(q) ||
      p.words.some((w) => w.toLowerCase().includes(q)) ||
      (q === "light" && isLight(t.background)) ||
      (q === "glow" && t.glow > 0),
  );
  const current = PRESETS.find((p) => p.id === id);
  const showCustom = !q || "custom".includes(q) || CUSTOM_WORDS.some((w) => w.toLowerCase().includes(q));

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Palette className="size-5" /> Themes
          </CardTitle>
          <CardDescription>
            {PRESETS.length + 1} themes. Choose <b>Custom</b> to set your own colours, transparency, glow, fonts and a background image.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap items-center gap-2">
          <div className="relative min-w-48 flex-1">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search themes (try “pastel”, “glow” or “light”)" className="pl-8" aria-label="Search themes" />
          </div>
          {current && (
            <Button size="sm" variant="secondary" onClick={() => customiseFrom(current.id)}>
              <Palette className="size-4" /> Customise {current.name}
            </Button>
          )}
        </CardContent>
      </Card>

      {id === CUSTOM_THEME_ID && <CustomEditor />}

      <div className="grid grid-cols-2 gap-3 @2xl:grid-cols-3">
        {showCustom && <ThemeTile name="Custom" words={CUSTOM_WORDS} t={resolveCustom(custom, image)} active={id === CUSTOM_THEME_ID} onSelect={() => setTheme(CUSTOM_THEME_ID)} />}
        {shown.map(({ p, t }) => (
          <ThemeTile key={p.id} name={p.name} words={p.words} t={t} active={id === p.id} onSelect={() => setTheme(p.id)} />
        ))}
      </div>
      {!shown.length && !showCustom && <p className="text-sm text-muted-foreground">No themes match “{query}”.</p>}
    </div>
  );
}

const HEX = /^#[0-9a-f]{6}$/i;

function ColorField({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const id = `theme-${label.toLowerCase().replace(/\W+/g, "-")}`;
  return (
    <div className="grid gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      <div className="flex items-center gap-2">
        <input
          type="color"
          aria-label={`${label} colour picker`}
          value={HEX.test(value) ? value : "#000000"}
          onChange={(e) => onChange(e.target.value)}
          className="h-8 w-10 shrink-0 cursor-pointer rounded-md border bg-transparent p-0.5"
        />
        <Input
          id={id}
          value={draft}
          spellCheck={false}
          className="h-8 font-mono text-xs"
          onChange={(e) => {
            const v = e.target.value.trim();
            setDraft(v);
            const hex = v.startsWith("#") ? v : `#${v}`;
            if (HEX.test(hex)) onChange(hex.toLowerCase());
          }}
          onBlur={() => setDraft(value)}
        />
      </div>
    </div>
  );
}

function SliderField({
  label,
  value,
  min,
  max,
  step,
  format,
  onChange,
  disabled,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  format: (v: number) => string;
  onChange: (v: number) => void;
  disabled?: boolean;
}) {
  return (
    <div className={cn("grid gap-2", disabled && "opacity-50")}>
      <div className="flex justify-between text-sm">
        <Label>{label}</Label>
        <span className="text-xs text-muted-foreground tabular-nums">{format(value)}</span>
      </div>
      <Slider value={[value]} min={min} max={max} step={step} disabled={disabled} onValueChange={([v]) => onChange(v)} aria-label={label} />
    </div>
  );
}

function ChoiceField<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: [T, string][]; onChange: (v: T) => void }) {
  return (
    <div className="grid gap-1.5">
      <Label>{label}</Label>
      <Select value={value} onValueChange={(v) => onChange(v as T)}>
        <SelectTrigger size="sm" className="w-full" aria-label={label}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map(([v, l]) => (
            <SelectItem key={v} value={v}>
              {l}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

/** Downscale a picked image so it fits comfortably in local storage. */
async function loadImage(file: File, maxSide = 2560): Promise<string> {
  const bmp = await createImageBitmap(file);
  const scale = Math.min(1, maxSide / Math.max(bmp.width, bmp.height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(bmp.width * scale);
  canvas.height = Math.round(bmp.height * scale);
  canvas.getContext("2d")!.drawImage(bmp, 0, 0, canvas.width, canvas.height);
  bmp.close();
  return canvas.toDataURL("image/jpeg", 0.85);
}

const pct = (v: number) => `${Math.round(v * 100)}%`;

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="space-y-3">
      <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{title}</h3>
      {children}
    </section>
  );
}

function CustomEditor() {
  const { custom: c, image, updateCustom, setImage } = useTheme();
  const fileRef = useRef<HTMLInputElement>(null);
  const [loading, setLoading] = useState(false);
  const set = <K extends keyof CustomTheme>(k: K) => (v: CustomTheme[K]) => updateCustom({ [k]: v } as Partial<CustomTheme>);

  const pick = async (file: File | undefined) => {
    if (!file) return;
    setLoading(true);
    try {
      const url = await loadImage(file);
      if (!setImage(url)) toast.warning("The image is too large to remember after a restart. Try a smaller one.");
    } catch {
      toast.error("That file couldn't be opened as an image.");
    } finally {
      setLoading(false);
      if (fileRef.current) fileRef.current.value = "";
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>Custom theme</CardTitle>
      </CardHeader>
      <CardContent className="space-y-6">
        <Section title="Colours">
          <div className="grid grid-cols-2 gap-3 @xl:grid-cols-3">
            <ColorField label="Background" value={c.background} onChange={set("background")} />
            <ColorField label="Surfaces" value={c.surface} onChange={set("surface")} />
            <ColorField label="Primary" value={c.primary} onChange={set("primary")} />
            <ColorField label="Accent" value={c.accent} onChange={set("accent")} />
            <ColorField label="Text" value={c.foreground} onChange={set("foreground")} />
          </div>
        </Section>

        <Section title="Transparency & glow">
          <div className="grid gap-4 @xl:grid-cols-2">
            <SliderField label="Surface opacity" value={c.opacity} min={0.15} max={1} step={0.05} format={pct} onChange={set("opacity")} />
            <SliderField label="Frosted blur" value={c.blur} min={0} max={40} step={1} format={(v) => `${v}px`} onChange={set("blur")} disabled={c.opacity >= 1 && !image} />
            <SliderField label="Glow" value={c.glow} min={0} max={1} step={0.05} format={(v) => (v ? pct(v) : "Off")} onChange={set("glow")} />
            <div className="flex items-center gap-2 self-end pb-0.5">
              <Switch id="theme-glow-accent" checked={c.glowAccent} disabled={!c.glow} onCheckedChange={set("glowAccent")} />
              <Label htmlFor="theme-glow-accent">Glow in the accent colour</Label>
            </div>
          </div>
        </Section>

        <Section title="Background">
          <div className="grid gap-4 @xl:grid-cols-3">
            <ChoiceField
              label="Gradient"
              value={c.gradient}
              options={[
                ["none", "None"],
                ["linear", "Linear"],
                ["radial", "Radial glow"],
              ]}
              onChange={set("gradient")}
            />
            {c.gradient !== "none" && <ColorField label="Gradient colour" value={c.gradientColor} onChange={set("gradientColor")} />}
            {c.gradient === "linear" && <SliderField label="Angle" value={c.gradientAngle} min={0} max={360} step={5} format={(v) => `${v}°`} onChange={set("gradientAngle")} />}
          </div>
          <div className="flex flex-wrap items-center gap-3 rounded-md border border-dashed p-3">
            {image ? (
              <img src={image} alt="Background preview" className="h-16 w-28 rounded-md border object-cover" />
            ) : (
              <div className="grid h-16 w-28 place-items-center rounded-md border text-xs text-muted-foreground">No image</div>
            )}
            <div className="flex flex-wrap gap-2">
              <input ref={fileRef} type="file" accept="image/*" hidden onChange={(e) => void pick(e.target.files?.[0])} />
              <Button size="sm" disabled={loading} onClick={() => fileRef.current?.click()}>
                <ImagePlus className="size-4" /> {image ? "Change image" : "Choose image"}
              </Button>
              {image && (
                <Button size="sm" variant="ghost" onClick={() => setImage(null)}>
                  <Trash2 className="size-4" /> Remove
                </Button>
              )}
            </div>
          </div>
          {image && (
            <div className="grid gap-4 @xl:grid-cols-3">
              <ChoiceField
                label="Image fit"
                value={c.imageFit}
                options={[
                  ["cover", "Fill window"],
                  ["contain", "Fit inside"],
                  ["tile", "Tile"],
                ]}
                onChange={set("imageFit")}
              />
              <SliderField label="Tint toward background" value={c.imageDim} min={0} max={0.9} step={0.05} format={pct} onChange={set("imageDim")} />
              <SliderField label="Image blur" value={c.imageBlur} min={0} max={30} step={1} format={(v) => `${v}px`} onChange={set("imageBlur")} />
            </div>
          )}
        </Section>

        <Section title="Shape & type">
          <div className="grid gap-4 @xl:grid-cols-2">
            <SliderField label="Corner roundness" value={c.radius} min={0} max={1.4} step={0.05} format={(v) => `${Math.round(v * 16)}px`} onChange={set("radius")} />
            <ChoiceField label="Font" value={c.font} options={(Object.keys(FONTS) as ThemeFont[]).map((f) => [f, FONTS[f].label])} onChange={set("font")} />
          </div>
        </Section>

        <Button size="sm" variant="ghost" onClick={() => updateCustom(DEFAULT_CUSTOM)}>
          <RotateCcw className="size-4" /> Reset custom settings
        </Button>
      </CardContent>
    </Card>
  );
}
