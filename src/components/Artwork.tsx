import { useState } from "react";
import { Music } from "lucide-react";
import { artworkSrc } from "@/lib/format";
import { cn } from "@/lib/utils";

export function Artwork({ src, className, alt = "" }: { src: string | null | undefined; className?: string; alt?: string }) {
  const [failed, setFailed] = useState<string | null>(null);
  const url = artworkSrc(src);
  if (!url || failed === url) {
    return (
      <div className={cn("grid place-items-center rounded-md bg-muted text-muted-foreground", className)} aria-hidden>
        <Music className="size-1/2 max-h-8 max-w-8" />
      </div>
    );
  }
  return (
    <img
      src={url}
      alt={alt}
      loading="lazy"
      draggable={false}
      onError={() => setFailed(url)}
      className={cn("rounded-md bg-muted object-cover", className)}
    />
  );
}
