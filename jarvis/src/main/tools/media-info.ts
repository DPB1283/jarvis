import { z } from "zod";
import { runPs } from "./ps";
import { defineTool } from "./types";

const readMedia = String.raw`
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$managerType = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager, Windows.Media.Control, ContentType=WindowsRuntime]
$propertiesType = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties, Windows.Media.Control, ContentType=WindowsRuntime]
$asTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
  $_.Name -eq 'AsTask' -and $_.IsGenericMethodDefinition -and
  $_.GetGenericArguments().Count -eq 1 -and $_.GetParameters().Count -eq 1 -and
  $_.GetParameters()[0].ParameterType.Name -like 'IAsyncOperation?1'
} | Select-Object -First 1
function Await($operation, [Type]$type) {
  $task = $asTask.MakeGenericMethod($type).Invoke($null, @($operation))
  return $task.GetAwaiter().GetResult()
}
$manager = Await ($managerType::RequestAsync()) $managerType
$session = $manager.GetCurrentSession()
if ($null -eq $session -or $session.GetPlaybackInfo().PlaybackStatus.ToString() -ne 'Playing') {
  $playing = $manager.GetSessions() | Where-Object { $_.GetPlaybackInfo().PlaybackStatus.ToString() -eq 'Playing' } | Select-Object -First 1
  if ($null -ne $playing) { $session = $playing }
}
if ($null -eq $session) { 'null'; exit 0 }
$media = Await ($session.TryGetMediaPropertiesAsync()) $propertiesType
@{
  title = $media.Title
  artist = $media.Artist
  album = $media.AlbumTitle
  app = $session.SourceAppUserModelId
  status = $session.GetPlaybackInfo().PlaybackStatus.ToString()
} | ConvertTo-Json -Compress
`;

export const nowPlaying = defineTool({
  name: "now_playing",
  description: "Read the current Windows media session, including track, artist, album, source app and playback status.",
  schema: z.object({}),
  risk: "safe",
  summary: () => "check what is playing",
  run: async () => {
    if (process.platform !== "win32") return "Now playing is available on Windows.";
    const result = await runPs(readMedia, 9_000);
    if (result.code !== 0) return "Could not read Windows media playback information.";
    try {
      const media = JSON.parse(result.stdout);
      if (!media) return "Windows has no active media session.";
      return `Title: ${media.title || "Unknown"}\nArtist: ${media.artist || "Unknown"}\nAlbum: ${media.album || "Unknown"}\nApp: ${media.app || "Unknown"}\nStatus: ${media.status || "Unknown"}`;
    } catch {
      return "Windows returned an unexpected media session response.";
    }
  },
});
