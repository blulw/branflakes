import type { ReactNode } from "react";
import { Plus } from "lucide-react";
import { toast } from "sonner";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { PlaylistKindIcon } from "@/components/PlaylistKind";
import { api, toApiError } from "@/lib/api";
import { addTracksToPlaylist } from "@/lib/playlistAdd";
import { useUi } from "@/state/ui";
import type { Track } from "@/lib/types";

/**
 * "Add to playlist" picker for one or more tracks. With `newPlaylistName`
 * it also offers to create a playlist of that name holding the tracks.
 */
export function AddToPlaylistMenu({
  tracks,
  children,
  newPlaylistName,
  align = "end",
}: {
  tracks: Track[];
  children: ReactNode;
  newPlaylistName?: string;
  align?: "start" | "end";
}) {
  const { playlists, refreshPlaylists, navigate } = useUi();
  const many = tracks.length > 1;

  const createWith = async (name: string) => {
    try {
      const r = await api.createPlaylist(name);
      await addTracksToPlaylist(r.playlist, tracks, { quiet: true });
      await refreshPlaylists();
      toast.success(`Created “${r.playlist.name}”`, {
        description: `${tracks.length} ${tracks.length === 1 ? "track" : "tracks"}`,
        action: { label: "Open", onClick: () => navigate({ name: "playlist", id: r.playlist.id }) },
      });
    } catch (e) {
      toast.error(toApiError(e).message);
    }
  };

  return (
    <DropdownMenu onOpenChange={(o) => o && void refreshPlaylists()}>
      <DropdownMenuTrigger asChild>{children}</DropdownMenuTrigger>
      <DropdownMenuContent align={align} className="max-h-80 w-56 overflow-y-auto">
        <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
          {many ? `Add ${tracks.length} tracks to playlist` : "Add to playlist"}
        </DropdownMenuLabel>
        {newPlaylistName && (
          <>
            <DropdownMenuItem onSelect={() => void createWith(newPlaylistName)}>
              <Plus className="size-4" />
              <span className="truncate">New playlist “{newPlaylistName}”</span>
            </DropdownMenuItem>
            <DropdownMenuSeparator />
          </>
        )}
        {playlists.length === 0 && <DropdownMenuItem disabled>No playlists yet</DropdownMenuItem>}
        {playlists.map((p) => (
          <DropdownMenuItem
            key={p.id}
            disabled={tracks.length === 0}
            onSelect={() =>
              void addTracksToPlaylist(p, tracks, { onOpen: (copy) => navigate({ name: "playlist", id: copy.id }) }).then(() => refreshPlaylists())
            }
          >
            <span className="truncate">{p.name}</span>
            <PlaylistKindIcon playlist={p} className="ml-auto" />
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
