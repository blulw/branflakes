import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { Sidebar } from "@/components/Sidebar";
import { PlayerBar } from "@/components/PlayerBar";
import { BranDropZone } from "@/components/BranDropZone";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { usePlayer } from "@/playback/store";
import { useUi } from "@/state/ui";
import { toApiError } from "@/lib/api";
import { cn } from "@/lib/utils";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import type { ArtistSyncEvent, SyncEvent } from "@/lib/types";
import { FollowedArtistView, FollowedArtistsView } from "@/views/FollowedArtists";
import { LibraryView } from "@/views/LibraryView";
import { AlbumsView, ArtistsView } from "@/views/BrowseViews";
import { PlaylistView } from "@/views/PlaylistView";
import { QueueView } from "@/views/QueueView";
import { SearchView } from "@/views/SearchView";
import { SettingsView } from "@/views/SettingsView";
import { TrackEditorDialog } from "@/views/TrackEditorDialog";
import { LyricsPanel } from "@/components/LyricsPanel";
import { FullscreenView } from "@/components/FullscreenView";
import { computeLayout, useWindowWidth } from "@/lib/layout";

function CurrentView() {
  const view = useUi((s) => s.view);
  switch (view.name) {
    case "library":
      return <LibraryView />;
    case "artists":
      return <ArtistsView />;
    case "artist":
      return <LibraryView key={view.artist} artist={view.artist} title={view.artist} />;
    case "albums":
      return <AlbumsView />;
    case "album":
      return <LibraryView key={view.album} album={view.album} title={view.album} />;
    case "playlist":
      return <PlaylistView key={view.id} id={view.id} />;
    case "queue":
      return <QueueView />;
    case "search":
      return <SearchView />;
    case "followed":
      return <FollowedArtistsView />;
    case "followedArtist":
      return <FollowedArtistView key={view.id} id={view.id} />;
    case "settings":
      return <SettingsView />;
  }
}

export default function App() {
  const lyricsOpen = useUi((s) => s.lyricsOpen);
  const fullscreen = useUi((s) => s.fullscreen);
  const layout = computeLayout(useWindowWidth(), lyricsOpen);
  const [ready, setReady] = useState(false);
  const [fatal, setFatal] = useState<string | null>(null);

  // Spotify playlist sync runs in the backend; keep the UI current.
  useEffect(() => {
    const un = listen<SyncEvent>("playlist-sync", (e) => {
      const ui = useUi.getState();
      ui.setSyncing(e.payload.playlistId, e.payload.status === "syncing");
      if (e.payload.status !== "syncing") {
        void ui.refreshPlaylists().catch(() => undefined);
        if (e.payload.changed) ui.bumpLibrary();
      }
      if (e.payload.status === "removed" || e.payload.status === "deleted") {
        // Reported once, when it happens (the backend only emits it then).
        if (e.payload.status === "removed" && e.payload.message) toast.warning("Spotify playlist removed", { description: e.payload.message, duration: 15000 });
        if (ui.view.name === "playlist" && ui.view.id === e.payload.playlistId) ui.navigate({ name: "library" });
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  // Followed-artist syncs (scheduled or manual) also run in the backend.
  useEffect(() => {
    const un = listen<ArtistSyncEvent>("artist-sync", (e) => {
      const ui = useUi.getState();
      ui.setArtistSyncing(e.payload.artistId, e.payload.status === "syncing");
      if (e.payload.status !== "syncing") void ui.refreshFollowed().catch(() => undefined);
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    Promise.all([usePlayer.getState().init(), useUi.getState().refreshSettings()])
      .then(() => setReady(true))
      .catch((e) => setFatal(toApiError(e).message));
  }, []);

  if (fatal) {
    return (
      <div className="grid h-full place-items-center p-8 text-center">
        <div className="max-w-md space-y-2">
          <h1 className="text-lg font-semibold">branflakes could not start</h1>
          <p className="text-sm text-muted-foreground" data-selectable>{fatal}</p>
        </div>
      </div>
    );
  }
  if (!ready) {
    return (
      <div className="grid h-full place-items-center">
        <Loader2 className="size-6 animate-spin text-muted-foreground" />
      </div>
    );
  }
  return (
    <TooltipProvider delayDuration={300}>
      {/* Hidden (not unmounted) under the full-screen view so the theme background shows through it. */}
      <div className={cn("flex h-full flex-col", fullscreen && "invisible")} aria-hidden={fullscreen || undefined}>
        <div className="flex min-h-0 flex-1">
          <Sidebar width={layout.sidebarWidth} labels={layout.sidebarLabels} titles={layout.playlistTitles} />
          {/* Kept mounted (just hidden) so the view keeps its state when lyrics take over. */}
          <main className={layout.showMain ? "@container flex min-h-0 min-w-0 flex-1 flex-col" : "hidden"}>
            <CurrentView />
          </main>
          {lyricsOpen && <LyricsPanel docked={layout.lyricsDocked} />}
        </div>
        <PlayerBar />
      </div>
      {fullscreen && <FullscreenView />}
      <TrackEditorDialog />
      <BranDropZone />
      <Toaster position="bottom-right" offset={{ bottom: 110 }} />
    </TooltipProvider>
  );
}
