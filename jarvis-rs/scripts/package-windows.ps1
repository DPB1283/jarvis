[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$project = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$legacy = (Resolve-Path (Join-Path $project '../jarvis')).Path
$release = Join-Path $project 'target/release/jarvis-app.exe'
$node = (Get-Command node.exe).Source
$version = (Get-Content (Join-Path $project 'crates/jarvis-app/tauri.conf.json') -Raw | ConvertFrom-Json).version
$packages = Join-Path $project 'target/packages'
$stage = Join-Path $packages ("Jarvis-$version-win-x64-" + [Guid]::NewGuid().ToString('N').Substring(0,8))
$null = New-Item -ItemType Directory -Path $stage
Copy-Item -LiteralPath $release -Destination (Join-Path $stage 'Jarvis.exe')
Copy-Item -LiteralPath (Join-Path $project "../browser-extension") -Destination (Join-Path $stage "browser-extension") -Recurse
foreach ($dir in 'bin','models','utilities','licenses') { $null = New-Item -ItemType Directory -Path (Join-Path $stage $dir) }
$vosk = Join-Path $legacy 'node_modules/vosk-koffi'
Get-ChildItem -LiteralPath (Join-Path $vosk 'bin-win32-x64') -Filter '*.dll' | Copy-Item -Destination (Join-Path $stage 'bin')
Copy-Item -LiteralPath (Join-Path $legacy 'models/vosk-model-small-en-us-0.15') -Destination (Join-Path $stage 'models') -Recurse
Copy-Item -LiteralPath (Join-Path $vosk 'LICENSE') -Destination (Join-Path $stage 'licenses/vosk-koffi.txt')
Copy-Item -LiteralPath $node -Destination (Join-Path $stage 'utilities/node.exe')
Copy-Item -LiteralPath (Join-Path (Split-Path $node) 'LICENSE') -Destination (Join-Path $stage 'licenses/node.txt')
Copy-Item -LiteralPath (Join-Path $project 'target/utilities.cjs') -Destination (Join-Path $stage 'utilities')
$archive = "$stage.zip"
Compress-Archive -LiteralPath $stage -DestinationPath $archive -CompressionLevel Fastest
[PSCustomObject]@{ Directory = $stage; Archive = $archive }

