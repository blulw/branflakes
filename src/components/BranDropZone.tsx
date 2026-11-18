import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { FileDown } from "lucide-react";
import { toast } from "sonner";
import { api } from "@/lib/api";
import { importBranFiles, isBranPath } from "@/lib/bran";

/**
 * Opens .bran files dropped onto the window, double-clicked in Explorer
 * (first launch: pending args; app already running: "bran-open" event).
 */
export function BranDropZone() {
  const [over, setOver] = useState<null | boolean>(null); // null: no drag; true: has .bran; false: other files

  useEffect(() => {
    const takePending = () =>
      void api
        .takePendingBran()
        .then((paths) => (paths.length ? importBranFiles(paths) : undefined))
        .catch(() => undefined);
    takePending();
    const unOpen = listen("bran-open", takePending);
    const unDrop = getCurrentWebview().onDragDropEvent((e) => {
      const p = e.payload;
      if (p.type === "enter") setOver(p.paths.some(isBranPath));
      else if (p.type === "leave") setOver(null);
      else if (p.type === "drop") {
        setOver(null);
        const files = p.paths.filter(isBranPath);
        if (files.length) void importBranFiles(files);
        else if (p.paths.length) toast.info("Drop a .bran playlist file to add it");
      }
    });
    return () => {
      void unOpen.then((f) => f());
      void unDrop.then((f) => f());
    };
  }, []);

  if (over == null) return null;
  return (
    <div className="pointer-events-none fixed inset-0 z-50 grid place-items-center bg-background/70 backdrop-blur-sm">
      <div className="flex flex-col items-center gap-3 rounded-xl border-2 border-dashed border-primary/60 bg-card px-10 py-8 text-center shadow-lg">
        <FileDown className="size-8 text-primary" />
        <p className="text-sm font-medium">{over ? "Drop to add this shared playlist" : "Only .bran playlist files can be dropped here"}</p>
      </div>
    </div>
  );
}
