<#
.SYNOPSIS
  Builds Jarvis-Setup-<version>.exe: one installer that brings everything Jarvis needs.

.DESCRIPTION
  1. builds the web UI, the Node helper bundle and the release app,
  2. makes sure the wake-word model and runtime are present (downloading the model if missing),
  3. stages Jarvis.exe + wake-word library + model + Node helpers + browser extension,
  4. compiles installer\jarvis.iss with Inno Setup (winget install JRSoftware.InnoSetup).

  Run from anywhere:  powershell -File jarvis-rs\scripts\make-installer.ps1
  Output:             jarvis-rs\target\installer\Jarvis-Setup-<version>.exe
  Add -SkipBuild to reuse the existing release build.
#>
[CmdletBinding()]
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$project = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$ui = (Resolve-Path (Join-Path $project '../jarvis')).Path
$version = (Get-Content (Join-Path $project 'crates/jarvis-app/tauri.conf.json') -Raw | ConvertFrom-Json).version

function Step($text) { Write-Host "`n== $text" -ForegroundColor Cyan }

# --- Inno Setup ---------------------------------------------------------------------------------------
$iscc = @(
  "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
  "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
  "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) {
  Step 'Installing Inno Setup (needed once, to build the installer)'
  winget install --id JRSoftware.InnoSetup --exact --source winget --silent --accept-package-agreements --accept-source-agreements --scope user
  $iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
  if (-not $iscc) { throw 'Inno Setup is not installed. Install it from https://jrsoftware.org/isinfo.php and run this again.' }
}

# --- Build ---------------------------------------------------------------------------------------------
if (-not $SkipBuild) {
  Step 'Installing UI dependencies'
  Push-Location $ui; npm install --no-audit --no-fund; Pop-Location
  Step 'Building the web UI and the Node helpers'
  Push-Location $project
  node scripts/frontend.mjs; if ($LASTEXITCODE) { throw 'UI build failed' }
  node scripts/build-utilities.mjs; if ($LASTEXITCODE) { throw 'Utilities build failed' }
  Step 'Building the release app (a few minutes)'
  cargo build --release -p jarvis-app; if ($LASTEXITCODE) { throw 'cargo build failed' }
  Pop-Location
}

# --- Wake-word model ------------------------------------------------------------------------------------
$modelDir = Join-Path $ui 'models/vosk-model-small-en-us-0.15'
if (-not (Test-Path (Join-Path $modelDir 'am/final.mdl'))) {
  Step 'Downloading the wake-word model (about 40 MB)'
  $zip = Join-Path $env:TEMP 'vosk-model-small-en-us-0.15.zip'
  Invoke-WebRequest 'https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip' -OutFile $zip
  $null = New-Item -ItemType Directory -Force (Join-Path $ui 'models')
  Expand-Archive $zip -DestinationPath (Join-Path $ui 'models') -Force
  Remove-Item $zip
}

# --- Stage ---------------------------------------------------------------------------------------------
Step 'Staging the files'
$stage = Join-Path $project 'target/installer-stage'
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
$null = New-Item -ItemType Directory -Force $stage
Copy-Item (Join-Path $project 'target/release/jarvis-app.exe') (Join-Path $stage 'Jarvis.exe')
Copy-Item (Join-Path $project '../browser-extension') (Join-Path $stage 'browser-extension') -Recurse
foreach ($dir in 'bin', 'models', 'utilities', 'licenses') { $null = New-Item -ItemType Directory (Join-Path $stage $dir) }
$vosk = Join-Path $ui 'node_modules/vosk-koffi'
Get-ChildItem (Join-Path $vosk 'bin-win32-x64') -Filter '*.dll' | Copy-Item -Destination (Join-Path $stage 'bin')
Copy-Item $modelDir (Join-Path $stage 'models') -Recurse
Copy-Item (Join-Path $vosk 'LICENSE') (Join-Path $stage 'licenses/vosk-koffi.txt')
$node = (Get-Command node.exe).Source
Copy-Item $node (Join-Path $stage 'utilities/node.exe')
Copy-Item (Join-Path (Split-Path $node) 'LICENSE') (Join-Path $stage 'licenses/node.txt')
Copy-Item (Join-Path $project 'target/utilities.cjs') (Join-Path $stage 'utilities')
$license = Join-Path $project '../LICENSE'
if (Test-Path $license) { Copy-Item $license (Join-Path $stage 'LICENSE.txt') } else { Set-Content (Join-Path $stage 'LICENSE.txt') 'MIT License' }

# --- Compile -------------------------------------------------------------------------------------------
Step 'Compiling the installer'
$out = Join-Path $project 'target/installer'
$null = New-Item -ItemType Directory -Force $out
& $iscc "/DStageDir=$stage" "/DAppVersion=$version" "/DOutputDir=$out" (Join-Path $project 'installer/jarvis.iss')
if ($LASTEXITCODE) { throw 'Inno Setup failed' }
$exe = Join-Path $out "Jarvis-Setup-$version.exe"
Write-Host ("`nDone: {0} ({1:N0} MB)" -f $exe, ((Get-Item $exe).Length / 1MB)) -ForegroundColor Green
