import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { Check, FolderPlus, Loader2, RefreshCw, Trash2, X, Info } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { SpotifySyncSettings } from "./SpotifySyncSettings";
import { ThemeSettings } from "./ThemeSettings";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SourceBadge } from "@/components/SourceBadge";
import { useAsync } from "@/hooks/useAsync";
import { api, toApiError } from "@/lib/api";
import { usePlayer } from "@/playback/store";
import { useUi } from "@/state/ui";
import type { ImportReport, PlaybackCapabilities, ProviderStatus, ScanReport, SettingsPatch } from "@/lib/types";

async function guard<T>(fn: () => Promise<T>, ok?: string): Promise<T | undefined> {
  try {
    const r = await fn();
    if (ok) toast.success(ok);
    return r;
  } catch (e) {
    toast.error(toApiError(e).message, { duration: 8000 });
    return undefined;
  }
}

export function SettingsView() {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto max-w-4xl px-4 pb-10 pt-5 @xl:px-6">
        <h1 className="mb-4 text-2xl font-semibold tracking-tight">Settings & accounts</h1>
        <Tabs defaultValue="accounts">
          <TabsList>
            <TabsTrigger value="accounts">Accounts</TabsTrigger>
            <TabsTrigger value="folders">Local folders</TabsTrigger>
            <TabsTrigger value="capabilities">Capabilities</TabsTrigger>
            <TabsTrigger value="themes">Themes</TabsTrigger>
            <TabsTrigger value="general">General</TabsTrigger>
          </TabsList>
          <TabsContent value="accounts" className="space-y-4 pt-4">
            <Accounts />
          </TabsContent>
          <TabsContent value="folders" className="pt-4">
            <Folders />
          </TabsContent>
          <TabsContent value="capabilities" className="pt-4">
            <CapabilityMatrix />
          </TabsContent>
          <TabsContent value="themes" className="pt-4">
            <ThemeSettings />
          </TabsContent>
          <TabsContent value="general" className="pt-4">
            <General />
          </TabsContent>
        </Tabs>
      </div>
    </div>
  );
}

function StatusLine({ p }: { p?: ProviderStatus }) {
  if (!p) return null;
  return (
    <div className="flex items-center gap-2 text-sm">
      {p.connected ? <Check className="size-4 text-primary" /> : <X className="size-4 text-muted-foreground" />}
      {p.connected ? `Connected${p.accountName ? ` as ${p.accountName}` : ""}` : p.configured ? "Not connected" : "Not configured"}
    </div>
  );
}

function Limitations({ p }: { p?: ProviderStatus }) {
  if (!p?.limitations.length) return null;
  return (
    <details className="rounded-md border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
      <summary className="cursor-pointer select-none font-medium text-foreground">Restrictions & limitations</summary>
      <ul className="mt-2 list-disc space-y-1 pl-4">
        {p.limitations.map((l) => (
          <li key={l}>{l}</li>
        ))}
      </ul>
    </details>
  );
}

function Field({ id, label, ...props }: { id: string; label: string } & React.ComponentProps<typeof Input>) {
  return (
    <div className="grid gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Input id={id} {...props} />
    </div>
  );
}

function ImportSummary({ r }: { r: ImportReport }) {
  return (
    <p className="text-xs text-muted-foreground">
      Imported {r.tracks} tracks and {r.playlists} playlists.
      {r.syncedPlaylists > 0 && ` ${r.syncedPlaylists} playlist${r.syncedPlaylists === 1 ? " is" : "s are"} now Spotify Synced.`}
      {r.skippedPlaylists.length > 0 && ` Skipped ${r.skippedPlaylists.length} playlists you don't own or collaborate on (not readable in Spotify Development Mode).`}
      {r.notSynced.length > 0 && (
        <span className="mt-1 block">
          Imported without Spotify sync:
          {r.notSynced.map((n) => (
            <span key={n.name} className="block pl-2">
              • {n.name}: {n.reason}
            </span>
          ))}
        </span>
      )}
      {r.errors.length > 0 && ` ${r.errors.length} errors: ${r.errors.slice(0, 3).join("; ")}`}
    </p>
  );
}

function Accounts() {
  const { settings, refreshSettings, bumpLibrary, refreshPlaylists } = useUi();
  const { providers, refreshProviders } = usePlayer();
  const [form, setForm] = useState<SettingsPatch>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [progress, setProgress] = useState<string | null>(null);
  const [imports, setImports] = useState<Record<string, ImportReport>>({});
  const scrobbles = useAsync(() => api.lastfmQueueStatus(), [busy]);

  useEffect(() => {
    void refreshSettings();
    const un = listen<{ provider: string; message: string }>("import-progress", (e) => setProgress(e.payload.message));
    return () => {
      void un.then((f) => f());
    };
  }, [refreshSettings]);

  if (!settings) return <Loader2 className="size-5 animate-spin" />;

  const save = async (patch: SettingsPatch) => {
    await guard(async () => {
      await api.updateSettings(patch);
      setForm({});
      await refreshSettings();
      await refreshProviders();
    }, "Saved");
  };

  const action = async (key: string, fn: () => Promise<unknown>, ok?: string) => {
    setBusy(key);
    await guard(fn, ok);
    setBusy(null);
    setProgress(null);
    await refreshProviders();
  };

  const doImport = (provider: "spotify" | "soundcloud") =>
    action(`${provider}-import`, async () => {
      const r = provider === "spotify" ? await api.spotifyImport() : await api.soundcloudImport();
      setImports((m) => ({ ...m, [provider]: r }));
      bumpLibrary();
      await refreshPlaylists();
    });

  const v = (k: keyof SettingsPatch, fallback: string) => (form[k] as string | undefined) ?? fallback;

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <SourceBadge source="spotify" /> Spotify
          </CardTitle>
          <CardDescription>
            Create an app at developer.spotify.com, add the redirect URI below, and paste the Client ID. No client secret is needed (PKCE).
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="grid grid-cols-1 gap-3 @xl:grid-cols-2">
            <Field id="sp-id" label="Client ID" value={v("spotifyClientId", settings.spotifyClientId)} onChange={(e) => setForm({ ...form, spotifyClientId: e.target.value })} />
            <Field id="sp-redirect" label="Redirect URI (add this to your Spotify app)" value={settings.spotifyRedirectUri} readOnly data-selectable />
          </div>
          <StatusLine p={providers.spotify} />
          {providers.spotify?.needsReconnect && (
            <p className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
              Your Spotify sign-in is missing permissions the app now needs (in-app player, playlist sync). Click <b>Reconnect Spotify</b> and approve the request in your browser.
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button size="sm" variant="secondary" disabled={form.spotifyClientId === undefined} onClick={() => void save({ spotifyClientId: form.spotifyClientId })}>
              Save
            </Button>
            {providers.spotify?.connected ? (
              <>
                <Button
                  size="sm"
                  variant={providers.spotify.needsReconnect ? "default" : "secondary"}
                  disabled={!!busy}
                  onClick={() => void action("sp-connect", () => api.spotifyConnect(), "Spotify reconnected")}
                >
                  {busy === "sp-connect" && <Loader2 className="size-4 animate-spin" />} Reconnect Spotify
                </Button>
                <Button size="sm" disabled={!!busy} onClick={() => void doImport("spotify")}>
                  {busy === "spotify-import" && <Loader2 className="size-4 animate-spin" />} Import saved tracks & playlists
                </Button>
                <Button size="sm" variant="ghost" onClick={() => void action("sp-dc", () => api.spotifyDisconnect(), "Spotify disconnected")}>
                  Disconnect
                </Button>
              </>
            ) : (
              <Button size="sm" disabled={!settings.spotifyClientId || !!busy} onClick={() => void action("sp-connect", () => api.spotifyConnect(), "Spotify connected")}>
                {busy === "sp-connect" && <Loader2 className="size-4 animate-spin" />} Connect Spotify (opens browser)
              </Button>
            )}
          </div>
          {busy?.startsWith("spotify") && progress && <p className="text-xs text-muted-foreground">{progress}</p>}
          {imports.spotify && <ImportSummary r={imports.spotify} />}
          {providers.spotify?.connected && <SpotifySyncSettings intervalMinutes={settings.spotifySyncIntervalMinutes} needsReconnect={!!providers.spotify.needsReconnect} />}
          <Limitations p={providers.spotify} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <SourceBadge source="soundcloud" /> SoundCloud
          </CardTitle>
          <CardDescription>
            Register an app (requires SoundCloud Artist Pro) at soundcloud.com/you/apps or with SoundCloud's CLI, add the redirect URI below, and paste the client ID and secret. The secret is stored in Windows Credential Manager.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="grid grid-cols-1 gap-3 @xl:grid-cols-2">
            <Field id="sc-id" label="Client ID" value={v("soundcloudClientId", settings.soundcloudClientId)} onChange={(e) => setForm({ ...form, soundcloudClientId: e.target.value })} />
            <Field
              id="sc-secret"
              label={`Client secret ${settings.soundcloudSecretSet ? "(stored — leave blank to keep)" : ""}`}
              type="password"
              autoComplete="off"
              value={form.soundcloudClientSecret ?? ""}
              onChange={(e) => setForm({ ...form, soundcloudClientSecret: e.target.value })}
            />
            <Field id="sc-redirect" label="Redirect URI (add this to your SoundCloud app)" value={settings.soundcloudRedirectUri} readOnly data-selectable className="col-span-2" />
          </div>
          <StatusLine p={providers.soundcloud} />
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="secondary"
              disabled={form.soundcloudClientId === undefined && !form.soundcloudClientSecret}
              onClick={() => void save({ soundcloudClientId: form.soundcloudClientId, soundcloudClientSecret: form.soundcloudClientSecret || undefined })}
            >
              Save
            </Button>
            {providers.soundcloud?.connected ? (
              <>
                <Button size="sm" disabled={!!busy} onClick={() => void doImport("soundcloud")}>
                  {busy === "soundcloud-import" && <Loader2 className="size-4 animate-spin" />} Import likes & playlists
                </Button>
                <Button size="sm" variant="ghost" onClick={() => void action("sc-dc", () => api.soundcloudDisconnect(), "SoundCloud disconnected")}>
                  Disconnect
                </Button>
              </>
            ) : (
              <Button size="sm" disabled={!providers.soundcloud?.configured || !!busy} onClick={() => void action("sc-connect", () => api.soundcloudConnect(), "SoundCloud connected")}>
                {busy === "sc-connect" && <Loader2 className="size-4 animate-spin" />} Connect SoundCloud (opens browser)
              </Button>
            )}
          </div>
          {busy?.startsWith("soundcloud") && progress && <p className="text-xs text-muted-foreground">{progress}</p>}
          {imports.soundcloud && <ImportSummary r={imports.soundcloud} />}
          <SoundCloudWebMode enabled={settings.soundcloudWebMode} onToggle={(on) => void save({ soundcloudWebMode: on })} />
          <Limitations p={providers.soundcloud} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Last.fm scrobbling</CardTitle>
          <CardDescription>Create an API account at last.fm/api/account/create. Scrobbles are sent for local files, Spotify and SoundCloud tracks.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="grid grid-cols-1 gap-3 @xl:grid-cols-2">
            <Field id="lf-key" label="API key" value={v("lastfmApiKey", settings.lastfmApiKey)} onChange={(e) => setForm({ ...form, lastfmApiKey: e.target.value })} />
            <Field
              id="lf-secret"
              label={`Shared secret ${settings.lastfmSecretSet ? "(stored — leave blank to keep)" : ""}`}
              type="password"
              autoComplete="off"
              value={form.lastfmSharedSecret ?? ""}
              onChange={(e) => setForm({ ...form, lastfmSharedSecret: e.target.value })}
            />
          </div>
          <div className="flex items-center gap-2">
            <Switch id="lf-enabled" checked={settings.lastfmEnabled} onCheckedChange={(c) => void save({ lastfmEnabled: c })} />
            <Label htmlFor="lf-enabled">Scrobble to Last.fm</Label>
          </div>
          <div className="flex items-center gap-2">
            <Switch
              id="lf-spotify"
              checked={settings.lastfmScrobbleSpotify}
              disabled={!settings.lastfmEnabled}
              onCheckedChange={(c) => void save({ lastfmScrobbleSpotify: c })}
            />
            <Label htmlFor="lf-spotify">Scrobble Spotify tracks</Label>
          </div>
          <p className="text-xs text-muted-foreground">
            If your Spotify account is already linked to Last.fm in Spotify&apos;s own settings, turn this off to avoid double scrobbles.
          </p>
          <StatusLine p={providers.lastfm} />
          {scrobbles.data && (
            <p className="text-xs text-muted-foreground">
              Scrobble cache: {scrobbles.data.pending} pending, {scrobbles.data.failed} rejected.
              {scrobbles.data.lastError && ` Last error: ${scrobbles.data.lastError}`}
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="secondary"
              disabled={form.lastfmApiKey === undefined && !form.lastfmSharedSecret}
              onClick={() => void save({ lastfmApiKey: form.lastfmApiKey, lastfmSharedSecret: form.lastfmSharedSecret || undefined })}
            >
              Save
            </Button>
            {providers.lastfm?.connected ? (
              <>
                <Button size="sm" disabled={!!busy || !scrobbles.data?.pending} onClick={() => void action("lf-flush", () => api.lastfmFlush(), "Scrobbles sent")}>
                  Send pending scrobbles
                </Button>
                {!!scrobbles.data?.failed && (
                  <Button size="sm" variant="ghost" onClick={() => void action("lf-clear", () => api.lastfmClearFailed())}>
                    Discard rejected
                  </Button>
                )}
                <Button size="sm" variant="ghost" onClick={() => void action("lf-dc", () => api.lastfmDisconnect(), "Last.fm disconnected")}>
                  Disconnect
                </Button>
              </>
            ) : (
              <Button size="sm" disabled={!providers.lastfm?.configured || !!busy} onClick={() => void action("lf-connect", () => api.lastfmConnect(), "Last.fm connected")}>
                {busy === "lf-connect" && <Loader2 className="size-4 animate-spin" />}
                {busy === "lf-connect" ? "Waiting for you to approve in the browser…" : "Connect Last.fm (opens browser)"}
              </Button>
            )}
          </div>
          <Limitations p={providers.lastfm} />
        </CardContent>
      </Card>
    </>
  );
}

function SoundCloudWebMode({ enabled, onToggle }: { enabled: boolean; onToggle: (on: boolean) => void }) {
  const { bumpLibrary, refreshPlaylists } = useUi();
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [last, setLast] = useState<ImportReport | null>(null);

  const doImport = async () => {
    setBusy(true);
    const r = await guard(() => api.soundcloudWebImport(url));
    setBusy(false);
    if (r) {
      setLast(r);
      setUrl("");
      bumpLibrary();
      await refreshPlaylists();
      toast.success(r.playlists ? `Imported ${r.tracks} tracks into a playlist` : `Added ${r.tracks} track to your library`);
    }
  };

  return (
    <div className="space-y-3 rounded-md border border-dashed p-3">
      <div className="flex items-start gap-3">
        <Switch id="sc-web" checked={enabled} onCheckedChange={onToggle} />
        <div className="space-y-1">
          <Label htmlFor="sc-web">Web mode — play SoundCloud without an API app (unofficial)</Label>
          <p className="text-xs text-muted-foreground">
            Uses the same internal API as the soundcloud.com website instead of the official API, so no Artist Pro plan is needed. It can stop working whenever
            SoundCloud changes its website, SoundCloud may rate-limit or block it, and it may be against SoundCloud's Terms of Use. Public tracks only; Go+ tracks
            play the 30-second preview; DRM streams are never used and nothing is downloaded. If you connect an official SoundCloud app above, that is used instead.
          </p>
        </div>
      </div>
      {enabled && (
        <form
          className="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            void doImport();
          }}
        >
          <Input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="Paste a SoundCloud track, playlist or profile link"
            aria-label="SoundCloud link to import"
          />
          <Button type="submit" size="sm" disabled={busy || !url.trim()}>
            {busy && <Loader2 className="size-4 animate-spin" />} Import
          </Button>
        </form>
      )}
      {enabled && <p className="text-xs text-muted-foreground">A profile link imports that user's public likes as a playlist. You can also find SoundCloud tracks in Search.</p>}
      {last && <ImportSummary r={last} />}
    </div>
  );
}

function Folders() {
  const bumpLibrary = useUi((s) => s.bumpLibrary);
  const [rev, setRev] = useState(0);
  const { data: folders } = useAsync(() => api.listFolders(), [rev]);
  const [scanning, setScanning] = useState(false);
  const [progress, setProgress] = useState<string | null>(null);
  const [reports, setReports] = useState<ScanReport[]>([]);

  useEffect(() => {
    const un = listen<{ folder: string; processed: number }>("scan-progress", (e) => setProgress(`${e.payload.folder}: ${e.payload.processed} files`));
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const scan = async (id?: number) => {
    setScanning(true);
    const r = await guard(() => api.scan(id));
    if (r) setReports(r);
    setScanning(false);
    setProgress(null);
    setRev((x) => x + 1);
    bumpLibrary();
  };

  const add = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Choose a music folder" });
    if (typeof dir !== "string") return;
    const f = await guard(() => api.addFolder(dir));
    if (f) await scan(f.id);
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>Local music folders</CardTitle>
        <CardDescription>
          Supported: MP3, FLAC, M4A/AAC, Ogg Vorbis, Opus, WAV (play + edit); AIFF, APE, WavPack, Musepack, Speex (edit only). WMA and DSD files are reported but skipped.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {folders?.map((f) => (
          <div key={f.id} className="flex items-center gap-3 rounded-md border px-3 py-2 text-sm">
            <span className="flex-1 truncate" data-selectable>
              {f.path}
            </span>
            <span className="text-xs text-muted-foreground">{f.lastScannedAt ? `Scanned ${new Date(f.lastScannedAt).toLocaleString()}` : "Not scanned"}</span>
            <Button variant="ghost" size="icon" className="size-7" disabled={scanning} onClick={() => void scan(f.id)} aria-label="Rescan">
              <RefreshCw className="size-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="size-7"
              aria-label="Remove folder"
              onClick={() => {
                if (confirm(`Remove ${f.path} from the library? Files on disk are not touched.`)) void guard(() => api.removeFolder(f.id)).then(() => (setRev((x) => x + 1), bumpLibrary()));
              }}
            >
              <Trash2 className="size-3.5" />
            </Button>
          </div>
        ))}
        <div className="flex gap-2">
          <Button size="sm" onClick={() => void add()} disabled={scanning}>
            <FolderPlus className="size-4" /> Add folder
          </Button>
          <Button size="sm" variant="secondary" disabled={scanning || !folders?.length} onClick={() => void scan()}>
            {scanning ? <Loader2 className="size-4 animate-spin" /> : <RefreshCw className="size-4" />} Rescan all
          </Button>
        </div>
        {scanning && progress && <p className="text-xs text-muted-foreground">{progress}</p>}
        {reports.map((r) => (
          <div key={r.folder} className="rounded-md bg-muted/40 p-3 text-xs text-muted-foreground">
            <div className="font-medium text-foreground">{r.folder}</div>
            {r.filesSeen} audio files · {r.addedOrUpdated} added/updated · {r.unchanged} unchanged · {r.missing} missing · {r.unsupported} unsupported
            {r.errors.length > 0 && (
              <details className="mt-1">
                <summary className="cursor-pointer text-destructive">{r.errors.length} files could not be read</summary>
                <ul className="mt-1 max-h-40 overflow-y-auto">
                  {r.errors.map((e) => (
                    <li key={e} data-selectable>
                      {e}
                    </li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        ))}
      </CardContent>
    </Card>
  );
}

const CAP_LABELS: [keyof PlaybackCapabilities, string][] = [
  ["play", "Play"],
  ["pause", "Pause"],
  ["next", "Next"],
  ["previous", "Previous"],
  ["seek", "Seek"],
  ["volume", "Volume"],
  ["queue", "Queue"],
  ["deviceSelection", "Device selection"],
  ["playbackPosition", "Playback position"],
  ["duration", "Duration"],
];

function CapabilityMatrix() {
  const providers = usePlayer((s) => s.providers);
  const list = (["local", "spotify", "soundcloud"] as const).map((k) => providers[k]).filter(Boolean) as ProviderStatus[];
  return (
    <Card>
      <CardHeader>
        <CardTitle>Playback capabilities (live)</CardTitle>
        <CardDescription>
          What each source can do right now, as declared by its adapter and current connection state. The player only enables controls marked ✓. Next/previous move through the unified app queue.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-xs text-muted-foreground">
              <th className="py-1 font-medium">Capability</th>
              {list.map((p) => (
                <th key={p.provider} className="py-1 font-medium">
                  {p.displayName}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {CAP_LABELS.map(([k, label]) => (
              <tr key={k} className="border-t">
                <td className="py-1.5">{label}</td>
                {list.map((p) => (
                  <td key={p.provider} className="py-1.5">
                    {p.capabilities[k] ? <Check className="size-4 text-primary" aria-label="supported" /> : <X className="size-4 text-muted-foreground" aria-label="not supported" />}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
        <p className="mt-3 flex gap-2 text-xs text-muted-foreground">
          <Info className="size-4 shrink-0" />
          Gapless playback and crossfading are not supported. Only one source owns playback at a time; switching sources stops the previous one.
        </p>
      </CardContent>
    </Card>
  );
}

function NormalizeVolume() {
  const { normalizeVolume, setNormalizeVolume } = usePlayer();
  return (
    <div className="flex items-start gap-3">
      <Switch id="normalize-volume" checked={normalizeVolume} onCheckedChange={(c) => void setNormalizeVolume(c)} />
      <div className="space-y-1">
        <Label htmlFor="normalize-volume">Consistent volume between songs</Label>
        <p className="text-xs text-muted-foreground">
          Measures each song's average loudness (LUFS) the first time it plays and adjusts its level so loud and quiet songs sound about the same. The volume
          slider still controls overall volume. Spotify songs use the same song from your local files when you have it (on any device); otherwise the
          in-app player measures the song while it plays, adjusting a few seconds in and remembering the result once most of the song has played.
        </p>
      </div>
    </div>
  );
}

function LyricsSources() {
  const { settings, refreshSettings } = useUi();
  if (!settings) return null;
  return (
    <div className="flex items-start gap-3">
      <Switch
        id="lyrics-extra-sources"
        checked={settings.lyricsExtraSources}
        onCheckedChange={(c) => void guard(() => api.updateSettings({ lyricsExtraSources: c })).then(() => refreshSettings())}
      />
      <div className="space-y-1">
        <Label htmlFor="lyrics-extra-sources">More synced lyrics (Musixmatch, NetEase)</Label>
        <p className="text-xs text-muted-foreground">
          Lyrics always use Genius's words. Timing comes from LRCLIB; with this on, songs LRCLIB doesn't have are also timed from Musixmatch and NetEase Cloud
          Music (unofficial access). Only the song's title, artist, album and length are sent.
        </p>
      </div>
    </div>
  );
}

function General() {
  const { settings, refreshSettings } = useUi();
  const { data: info } = useAsync(() => api.appInfo(), []);
  const [port, setPort] = useState<string>("");
  if (!settings) return null;
  return (
    <Card>
      <CardHeader>
        <CardTitle>General</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4 text-sm">
        <NormalizeVolume />
        <LyricsSources />
        <div className="flex items-end gap-2">
          <Field id="port" label="OAuth loopback port" className="w-32" placeholder={String(settings.oauthPort)} value={port} onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))} />
          <Button size="sm" variant="secondary" disabled={!port} onClick={() => void guard(() => api.updateSettings({ oauthPort: Number(port) }), "Saved — update the redirect URIs in your Spotify/SoundCloud apps").then(() => (setPort(""), refreshSettings()))}>
            Save
          </Button>
        </div>
        <div className="text-xs text-muted-foreground" data-selectable>
          Version {info?.version} · schema v{info?.schemaVersion} · Windows media controls {info?.mediaControls ? "active" : "unavailable"}
          <br />
          Data folder: {settings.dataDir}
        </div>
      </CardContent>
    </Card>
  );
}
