import { create } from "zustand";
import { api } from "@/lib/api";
import type { AppSettings, Playlist, Track } from "@/lib/types";

export type View =
  | { name: "library" }
  | { name: "artists" }
  | { name: "artist"; artist: string }
  | { name: "albums" }
  | { name: "album"; album: string; artist: string }
  | { name: "playlist"; id: string }
  | { name: "queue" }
  | { name: "search" }
  | { name: "settings" };

interface UiState {
  view: View;
  /** Bumped whenever library data changes so views refetch. */
  libraryVersion: number;
  playlists: Playlist[];
  settings: AppSettings | null;
  editing: Track | null;
  navigate(v: View): void;
  bumpLibrary(): void;
  refreshPlaylists(): Promise<void>;
  refreshSettings(): Promise<void>;
  edit(t: Track | null): void;
}

export const useUi = create<UiState>((set) => ({
  view: { name: "library" },
  libraryVersion: 0,
  playlists: [],
  settings: null,
  editing: null,
  navigate: (view) => set({ view }),
  bumpLibrary: () => set((s) => ({ libraryVersion: s.libraryVersion + 1 })),
  async refreshPlaylists() {
    set({ playlists: await api.listPlaylists() });
  },
  async refreshSettings() {
    set({ settings: await api.getSettings() });
  },
  edit: (editing) => set({ editing }),
}));
