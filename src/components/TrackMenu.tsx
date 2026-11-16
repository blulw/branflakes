import type { ReactNode } from "react";
import { toast } from "sonner";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { api, toApiError } from "@/lib/api";
import { addTracksToPlaylist } from "@/lib/playlistAdd";
import { PlaylistKindIcon } from "@/components/PlaylistKind";
import { usePlayer } from "@/playback/store";
import { useUi } from "@/state/ui";
import type { Track } from "@/lib/types";

export function TrackMenu({ track, children, extra }: { track: Track; children: ReactNode; extra?: ReactNode }) {
  const enqueue = usePlayer((s) => s.enqueue);
  const { playlists, refreshPlaylists, bumpLibrary, edit, navigate } = useUi();

  const run = (fn: () => Promise<unknown>, ok?: string) => async () => {
    try {
      await fn();
      if (ok) toast.success(ok);
    } catch (e) {
      toast.error(toApiError(e).message);
    }
  };

  return (
    <DropdownMenu onOpenChange={(o) => o && void refreshPlaylists()}>
      <DropdownMenuTrigger asChild>{children}</DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuItem onSelect={run(() => enqueue([track], true))}>Play next</DropdownMenuItem>
        <DropdownMenuItem onSelect={run(() => enqueue([track], false))}>Add to queue</DropdownMenuItem>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>Add to playlist</DropdownMenuSubTrigger>
          <DropdownMenuSubContent className="max-h-80 overflow-y-auto">
            {playlists.length === 0 && <DropdownMenuItem disabled>No playlists yet</DropdownMenuItem>}
            {playlists.map((p) => (
              <DropdownMenuItem
                key={p.id}
                onSelect={() =>
                  void addTracksToPlaylist(p, [track], { onOpen: (copy) => navigate({ name: "playlist", id: copy.id }) }).then(() => refreshPlaylists())
                }
              >
                <span className="truncate">{p.name}</span>
                <PlaylistKindIcon playlist={p} className="ml-auto" />
              </DropdownMenuItem>
            ))}
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => edit(track)}>{track.source === "local" ? "Edit metadata…" : "Edit display metadata…"}</DropdownMenuItem>
        {track.inLibrary ? (
          <DropdownMenuItem onSelect={run(() => api.setMembership([track.id], false).then(bumpLibrary), "Removed from library")}>
            Remove from library
          </DropdownMenuItem>
        ) : (
          <DropdownMenuItem onSelect={run(() => api.setMembership([track.id], true).then(bumpLibrary), "Added to library")}>
            Add to library
          </DropdownMenuItem>
        )}
        {track.permalinkUrl && (
          <DropdownMenuItem onSelect={() => void openUrl(track.permalinkUrl!)}>
            Open on {track.source === "spotify" ? "Spotify" : "SoundCloud"}
          </DropdownMenuItem>
        )}
        {extra && (
          <>
            <DropdownMenuSeparator />
            {extra}
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
