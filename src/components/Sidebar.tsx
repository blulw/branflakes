import { useEffect, useMemo, useState } from "react";
import { Library, Mic2, Disc, Search, Settings, Plus, ListOrdered, ArrowDownUp, FileDown } from "lucide-react";
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
import { chooseAndImportBran } from "@/lib/bran";
import { PLAYLIST_SORT_LABEL, sortPlaylists, type PlaylistSort } from "@/lib/playlistSort";
import { cn } from "@/lib/utils";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { FitText } from "./FitText";
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

/**
 * The sidebar's width follows the window smoothly. Icons and covers stay put
 * (they line up on the same centre line at every width); text just goes away
 * when there's no room: button labels first, then playlist titles.
 */
export function Sidebar({ width = 240, labels = true, titles = true }: { width?: number; labels?: boolean; titles?: boolean }) {
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

  // px-3 inside p-3 puts 16px icons on the centre line of the narrowest (64px) sidebar.
  const item = (active: boolean) =>
    cn(
      "flex w-full items-center gap-3 rounded-md px-3 py-2 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
      active && "bg-accent text-foreground",
    );

  const closeForm = () => (setCreating(false), setName(""), setWithSpotify(false));
  const form = (
    <form
      className={cn(labels && "px-1")}
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
        <Button type="button" size="sm" variant="ghost" className="h-7" onClick={closeForm}>
          Cancel
        </Button>
        <Button type="submit" size="sm" className="h-7" disabled={!name.trim() || busy}>
          Create
        </Button>
      </div>
    </form>
  );

  return (
    <nav className="flex shrink-0 flex-col gap-1 overflow-hidden border-r bg-sidebar p-3" style={{ width }} aria-label="Main">
      <div className="flex items-center gap-2 px-2 pb-3 pt-1 text-sm font-semibold tracking-tight">
        <img src={logo} alt={labels ? "" : "branflakes"} className="size-6 shrink-0 rounded-md object-cover" />
        {labels && <FitText>branflakes</FitText>}
      </div>
      {NAV.map(({ view: v, label, icon: Icon }) => (
        <button
          key={label}
          className={item(view.name === v.name || (v.name === "artists" && view.name === "artist") || (v.name === "albums" && view.name === "album"))}
          onClick={() => navigate(v)}
          aria-label={label}
          title={labels ? undefined : label}
        >
          <Icon className="size-4 shrink-0" />
          {labels && <FitText>{label}</FitText>}
        </button>
      ))}

      {/* Three buttons don't fit side by side in the narrowest sidebar, so they stack there. */}
      <div className={cn("mt-4 flex items-center text-xs font-medium uppercase tracking-wide text-muted-foreground", !titles && "flex-col gap-0.5")}>
        {titles && (
          <FitText>
            <span className="pl-3">Playlists</span>
          </FitText>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" className="size-5 shrink-0" aria-label={`Sort playlists (${PLAYLIST_SORT_LABEL[sort]})`} title={`Sorted by: ${PLAYLIST_SORT_LABEL[sort]}`}>
              <ArrowDownUp className="size-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
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
        <Button variant="ghost" size="icon" className="size-5 shrink-0" aria-label="Open shared playlist" title="Open a shared playlist (.bran)" onClick={() => void chooseAndImportBran()}>
          <FileDown className="size-4" />
        </Button>
        <Button variant="ghost" size="icon" className="size-5 shrink-0" aria-label="New playlist" title="New playlist" onClick={() => setCreating((c) => !c)}>
          <Plus className="size-4" />
        </Button>
      </div>
      {creating && labels && form}
      {!labels && (
        <Dialog open={creating} onOpenChange={(o) => !o && closeForm()}>
          <DialogContent className="max-w-sm">
            <DialogHeader>
              <DialogTitle>New playlist</DialogTitle>
            </DialogHeader>
            {form}
          </DialogContent>
        </Dialog>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {sorted.map((p) => (
          <button
            key={p.id}
            // px-1: 32px covers share the 16px icons' centre line.
            className={cn(item(view.name === "playlist" && view.id === p.id), "px-1 py-1.5")}
            onClick={() => navigate({ name: "playlist", id: p.id })}
            aria-label={p.name}
            title={titles ? undefined : p.name}
          >
            <span className="relative shrink-0">
              <PlaylistCover path={p.artworkPath} className="size-8" />
              {!titles && <PlaylistKindIcon playlist={p} className="absolute -bottom-1 -right-1 rounded-full bg-background p-px" />}
            </span>
            {titles && (
              <span className="min-w-0 flex-1 text-left">
                <span className="flex items-center gap-1.5">
                  <span className="truncate">{p.name}</span>
                  <PlaylistKindIcon playlist={p} />
                </span>
                <span className="block truncate text-[11px] text-muted-foreground">{p.trackCount} tracks</span>
              </span>
            )}
          </button>
        ))}
        {playlists.length === 0 && !creating && labels && <p className="px-3 py-2 text-xs text-muted-foreground">No playlists. Playlists can mix local, Spotify and SoundCloud tracks.</p>}
      </div>

      <button className={item(view.name === "settings")} onClick={() => navigate({ name: "settings" })} aria-label="Settings & accounts" title={labels ? undefined : "Settings & accounts"}>
        <Settings className="size-4 shrink-0" />
        {labels && <FitText>Settings & accounts</FitText>}
      </button>
    </nav>
  );
}
