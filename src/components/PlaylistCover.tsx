import { useState } from "react";
import { artworkSrc } from "@/lib/format";
import { cn } from "@/lib/utils";

/** Playlist cover: the chosen image, or a plain grey square when none is set. */
export function PlaylistCover({ path, className, alt = "" }: { path: string | null | undefined; className?: string; alt?: string }) {
  const [failed, setFailed] = useState<string | null>(null);
  const url = artworkSrc(path);
  if (!url || failed === url) {
    return <div className={cn("rounded-md bg-muted", className)} aria-label={alt || "No cover"} role="img" />;
  }
  return <img src={url} alt={alt} draggable={false} onError={() => setFailed(url)} className={cn("rounded-md bg-muted object-cover", className)} />;
}
