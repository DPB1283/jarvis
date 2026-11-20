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
