import { useEffect, useMemo, useState } from "react";
import { Library, Mic2, Disc, Search, Settings, Plus, ListOrdered, ArrowDownUp } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { usePlayer } from "@/playback/store";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { PlaylistCover } from "./PlaylistCover";
import { PlaylistKindIcon } from "./PlaylistKind";
import { api, toApiError } from "@/lib/api";
import { PLAYLIST_SORT_LABEL, sortPlaylists, type PlaylistSort } from "@/lib/playlistSort";
import { cn } from "@/lib/utils";
import logo from "@/assets/logo.png";
import { useUi, type View } from "@/state/ui";

const NAV: { view: View; label: string; icon: typeof Library }[] = [
  { view: { name: "search" }, label: "Search", icon: Search },
  { view: { name: "library" }, label: "Songs", icon: Library },
  { view: { name: "artists" }, label: "Artists", icon: Mic2 },
  { view: { name: "albums" }, label: "Albums", icon: Disc },
  { view: { name: "queue" }, label: "Queue", icon: ListOrdered },
];

const SORT_KEY = "unified-music.playlist-sort";

function loadSort(): PlaylistSort {
  try {
    const v = localStorage.getItem(SORT_KEY);
    if (v && v in PLAYLIST_SORT_LABEL) return v as PlaylistSort;
  } catch {
    /* ignore */
  }
  return "recent";
}

export function Sidebar() {
  const { view, navigate, playlists, refreshPlaylists } = useUi();
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [withSpotify, setWithSpotify] = useState(false);
  const [busy, setBusy] = useState(false);
  const spotifyConnected = usePlayer((s) => !!s.providers.spotify?.connected);
  const [sort, setSort] = useState<PlaylistSort>(loadSort);
  const sorted = useMemo(() => sortPlaylists(playlists, sort), [playlists, sort]);

  useEffect(() => {
    void refreshPlaylists();
  }, [refreshPlaylists]);

  const changeSort = (s: string) => {
    setSort(s as PlaylistSort);
    try {
      localStorage.setItem(SORT_KEY, s);
    } catch {
      /* ignore */
    }
  };

  const create = async () => {
    if (busy || !name.trim()) return;
    setBusy(true);
    try {
      const sync = withSpotify && spotifyConnected;
      const r = await api.createPlaylist(name, sync);
      if (sync && r.spotifyError) {
        toast.warning(`“${r.playlist.name}” was created in Branflakes only`, {
          description: `Spotify sync couldn't be enabled: ${r.spotifyError}. You can try again from the playlist page.`,
          duration: 12000,
        });
      } else if (sync) {
        toast.success(`Created “${r.playlist.name}” and synced it with Spotify`);
      }
      setName("");
      setWithSpotify(false);
      setCreating(false);
      await refreshPlaylists();
      navigate({ name: "playlist", id: r.playlist.id });
    } catch (e) {
      toast.error(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  };

  const item = (active: boolean) =>
    cn(
      "flex w-full items-center gap-3 rounded-md px-3 py-2 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
      active && "bg-accent text-foreground",
    );

  return (
    <nav className="flex w-60 shrink-0 flex-col gap-1 border-r bg-sidebar p-3" aria-label="Main">
      <div className="flex items-center gap-2 px-3 pb-3 pt-1 text-sm font-semibold tracking-tight">
        <img src={logo} alt="" className="size-6 rounded-md object-cover" />
        branflakes
      </div>
      {NAV.map(({ view: v, label, icon: Icon }) => (
        <button key={label} className={item(view.name === v.name || (v.name === "artists" && view.name === "artist") || (v.name === "albums" && view.name === "album"))} onClick={() => navigate(v)}>
          <Icon className="size-4" />
          {label}
        </button>
      ))}

      <div className="mt-4 flex items-center gap-1 pl-3 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        <span className="mr-auto">Playlists</span>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" className="size-6" aria-label={`Sort playlists (${PLAYLIST_SORT_LABEL[sort]})`} title={`Sorted by: ${PLAYLIST_SORT_LABEL[sort]}`}>
              <ArrowDownUp className="size-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuLabel>Sort playlists by</DropdownMenuLabel>
            <DropdownMenuRadioGroup value={sort} onValueChange={changeSort}>
              {(Object.keys(PLAYLIST_SORT_LABEL) as PlaylistSort[]).map((k) => (
                <DropdownMenuRadioItem key={k} value={k}>
                  {PLAYLIST_SORT_LABEL[k]}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <Button variant="ghost" size="icon" className="size-6" aria-label="New playlist" onClick={() => setCreating((c) => !c)}>
          <Plus className="size-4" />
        </Button>
      </div>
      {creating && (
        <form
          className="px-1"
          onSubmit={(e) => {
            e.preventDefault();
            void create();
          }}
        >
          <Input autoFocus placeholder="Playlist name" value={name} onChange={(e) => setName(e.target.value)} className="h-8" aria-label="Playlist name" />
          <div className="mt-2 flex items-start gap-2 px-1">
            <Switch id="new-pl-spotify" checked={withSpotify && spotifyConnected} disabled={!spotifyConnected} onCheckedChange={setWithSpotify} />
            <Label htmlFor="new-pl-spotify" className="text-xs font-normal leading-snug text-muted-foreground">
              {spotifyConnected ? "Also create it on Spotify (private) and keep it in sync" : "Connect Spotify in Settings to sync new playlists"}
            </Label>
          </div>
          <div className="mt-2 flex justify-end gap-1">
            <Button type="button" size="sm" variant="ghost" className="h-7" onClick={() => (setCreating(false), setName(""), setWithSpotify(false))}>
              Cancel
            </Button>
            <Button type="submit" size="sm" className="h-7" disabled={!name.trim() || busy}>
              Create
            </Button>
          </div>
        </form>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {sorted.map((p) => (
          <button key={p.id} className={cn(item(view.name === "playlist" && view.id === p.id), "py-1.5")} onClick={() => navigate({ name: "playlist", id: p.id })}>
            <PlaylistCover path={p.artworkPath} className="size-8 shrink-0" />
            <span className="min-w-0 flex-1 text-left">
              <span className="flex items-center gap-1.5">
                <span className="truncate">{p.name}</span>
                <PlaylistKindIcon playlist={p} />
              </span>
              <span className="block text-[11px] text-muted-foreground">{p.trackCount} tracks</span>
            </span>
          </button>
        ))}
        {playlists.length === 0 && !creating && <p className="px-3 py-2 text-xs text-muted-foreground">No playlists. Playlists can mix local, Spotify and SoundCloud tracks.</p>}
      </div>

      <button className={item(view.name === "settings")} onClick={() => navigate({ name: "settings" })}>
        <Settings className="size-4" />
        Settings & accounts
      </button>
    </nav>
  );
}
