import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import type { Track } from "./types";
import { useUi } from "@/state/ui";

let permission: boolean | null = null;

/** Desktop notification for a track change, only when the window is not focused. */
export async function notifyTrack(track: Track) {
  if (!useUi.getState().settings?.notificationsEnabled) return;
  if (document.hasFocus()) return;
  try {
    if (permission == null) {
      permission = await isPermissionGranted();
      if (!permission) permission = (await requestPermission()) === "granted";
    }
    if (!permission) return;
    sendNotification({ title: track.title, body: [track.artist, track.album].filter(Boolean).join(" — ") });
  } catch {
    /* notifications are best-effort */
  }
}
