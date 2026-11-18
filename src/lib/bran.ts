// Shared playlist files (.bran): import/export flows used by the UI.
import { open, save } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { api, toApiError } from "./api";
import { useUi } from "@/state/ui";
import type { Playlist } from "./types";

export const BRAN_EXT = "bran";
const FILTERS = [{ name: "Branflakes playlist", extensions: [BRAN_EXT] }];

export function isBranPath(path: string): boolean {
  return path.toLowerCase().endsWith(`.${BRAN_EXT}`);
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(0)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}

/** Characters Windows doesn't allow in file names. */
export function safeFileName(name: string): string {
  return name.replace(/[<>:"/\|?*\u0000-\u001f]/g, "_").replace(/[. ]+$/, "").trim() || "playlist";
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** Ask where to save, then write the playlist as a .bran file. */
export async function exportPlaylistAsBran(playlist: Playlist): Promise<void> {
  const path = await save({ defaultPath: `${safeFileName(playlist.name)}.${BRAN_EXT}`, filters: FILTERS });
  if (!path) return;
  const t = toast.loading(`Saving “${playlist.name}”…`, { description: "Local songs are packed into the file; large playlists take a moment." });
  try {
    const r = await api.exportBran(playlist.id, path);
    const parts = [plural(r.tracks, "song"), r.localFiles ? `${plural(r.localFiles, "local file")} included` : null, formatBytes(r.bytes)].filter(Boolean);
    toast.success(`Saved “${playlist.name}” for sharing`, {
      id: t,
      description: parts.join(" · ") + (r.missing.length ? `. Not included (file missing): ${r.missing.join(", ")}` : ""),
      duration: r.missing.length ? 15000 : 6000,
    });
  } catch (e) {
    toast.error(`Couldn't save the playlist: ${toApiError(e).message}`, { id: t });
  }
}

let importing = Promise.resolve();

/** Import .bran files one after another, opening the last imported playlist. */
export function importBranFiles(paths: string[]): Promise<void> {
  const files = paths.filter(isBranPath);
  importing = importing.then(async () => {
    for (const path of files) await importOne(path);
  });
  return importing;
}

async function importOne(path: string): Promise<void> {
  const file = path.split(/[\/]/).pop() ?? path;
  const t = toast.loading(`Opening ${file}…`);
  try {
    const r = await api.importBran(path);
    const ui = useUi.getState();
    await ui.refreshPlaylists();
    ui.bumpLibrary();
    ui.navigate({ name: "playlist", id: r.playlist.id });
    const parts = [plural(r.tracks, "song"), r.localFiles ? `${plural(r.localFiles, "local file")}` : null].filter(Boolean);
    toast.success(`Added “${r.playlist.name}”`, {
      id: t,
      description: parts.join(" · ") + (r.skipped.length ? `. Skipped: ${r.skipped.join("; ")}` : ""),
      duration: r.skipped.length ? 15000 : 6000,
    });
  } catch (e) {
    toast.error(`Couldn't open ${file}: ${toApiError(e).message}`, { id: t });
  }
}

/** Pick .bran files with the file dialog and import them. */
export async function chooseAndImportBran(): Promise<void> {
  const p = await open({ multiple: true, filters: FILTERS });
  if (!p) return;
  await importBranFiles(Array.isArray(p) ? p : [p]);
}
