import { HardDrive, Cloud, Disc3 } from "lucide-react";
import { cn } from "@/lib/utils";
import { SOURCE_LABEL } from "@/lib/format";
import type { Source } from "@/lib/types";

const STYLE: Record<Source, string> = {
  local: "text-source-local border-source-local/40 bg-source-local/10",
  spotify: "text-source-spotify border-source-spotify/40 bg-source-spotify/10",
  soundcloud: "text-source-soundcloud border-source-soundcloud/40 bg-source-soundcloud/10",
};

const ICON: Record<Source, typeof HardDrive> = { local: HardDrive, spotify: Disc3, soundcloud: Cloud };

export function SourceBadge({ source, compact, className }: { source: Source; compact?: boolean; className?: string }) {
  const Icon = ICON[source];
  return (
    <span
      title={SOURCE_LABEL[source]}
      aria-label={`Source: ${SOURCE_LABEL[source]}`}
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded-full border px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide",
        STYLE[source],
        className,
      )}
    >
      <Icon className="size-3" aria-hidden />
      {!compact && SOURCE_LABEL[source]}
    </span>
  );
}

export function SourceBadges({ sources }: { sources: Source[] }) {
  return (
    <span className="inline-flex gap-1">
      {sources.map((s) => (
        <SourceBadge key={s} source={s} compact />
      ))}
    </span>
  );
}
