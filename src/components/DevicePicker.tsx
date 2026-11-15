import { useState } from "react";
import { MonitorSpeaker, RefreshCw, Check, Laptop } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { usePlayer } from "@/playback/store";

export function DevicePicker() {
  const { spotifyDevices, spotifyDeviceId, refreshSpotifyDevices, selectSpotifyDevice, sdkDeviceId, startInAppSpotify } = usePlayer();
  const [probing, setProbing] = useState(false);
  const current = spotifyDevices.find((d) => d.id === spotifyDeviceId);

  const enableSdk = async () => {
    setProbing(true);
    await startInAppSpotify(false);
    setProbing(false);
  };

  return (
    <DropdownMenu onOpenChange={(o) => o && void refreshSpotifyDevices()}>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="sm" className="max-w-44 gap-1.5 text-xs text-source-spotify" aria-label="Choose Spotify Connect device">
          <MonitorSpeaker className="size-4" />
          <span className="truncate">{current?.name ?? "Choose device"}</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-72">
        <DropdownMenuLabel className="flex items-center justify-between">
          Spotify Connect devices
          <Button variant="ghost" size="icon" className="size-6" onClick={() => void refreshSpotifyDevices()} aria-label="Refresh devices">
            <RefreshCw className="size-3" />
          </Button>
        </DropdownMenuLabel>
        {spotifyDevices.length === 0 && (
          <div className="px-2 py-2 text-xs text-muted-foreground">
            No devices found. Open Spotify on a phone, speaker or web browser signed in to the same account, then refresh.
          </div>
        )}
        {spotifyDevices.map((d) => (
          <DropdownMenuItem key={d.id ?? d.name} disabled={!d.id || d.isRestricted} onSelect={() => void selectSpotifyDevice(d.id)}>
            <span className="flex-1 truncate">
              {d.name} <span className="text-muted-foreground">· {d.type}</span>
              {d.isRestricted && <span className="block text-[10px] text-muted-foreground">Does not accept remote control</span>}
            </span>
            {d.id === spotifyDeviceId && <Check className="size-4" />}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem disabled={probing || !!sdkDeviceId} onSelect={(e) => { e.preventDefault(); void enableSdk(); }}>
          <Laptop className="size-4" />
          <span className="flex-1">
            {sdkDeviceId ? "In-app device active" : probing ? "Checking DRM support…" : "Try in-app device (experimental)"}
            <span className="block text-[10px] text-muted-foreground">Needs Widevine in WebView2 + Premium</span>
          </span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
