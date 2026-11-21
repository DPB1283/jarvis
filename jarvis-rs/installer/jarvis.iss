; Inno Setup script for the Jarvis installer. Build it with scripts\make-installer.ps1, which stages the files
; and passes /DStageDir=... /DAppVersion=... /DOutputDir=...
;
; It installs for the current user only (no administrator rights), puts everything Jarvis needs next to
; Jarvis.exe (wake-word library and model, Node helpers, browser extension) and installs the Microsoft Edge
; WebView2 runtime if the PC does not already have it.

#ifndef StageDir
  #error Run scripts\make-installer.ps1 - it passes /DStageDir to this script.
#endif
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef OutputDir
  #define OutputDir "..\target\installer"
#endif

[Setup]
AppId={{B6F2D1C0-5A0D-4F8E-9B58-7C1E3A2F4D10}
AppName=Jarvis
AppVersion={#AppVersion}
AppPublisher=Jarvis
DefaultDirName={localappdata}\Programs\Jarvis
DefaultGroupName=Jarvis
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=Jarvis-Setup-{#AppVersion}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\Jarvis.exe
LicenseFile={#StageDir}\LICENSE.txt
; Close a running Jarvis for the update, then offer to start it again.
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "startup"; Description: "Start Jarvis when I sign in to Windows"; GroupDescription: "Startup:"
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked
Name: "browserext"; Description: "Set up the browser extension when Jarvis starts (gives him his own tab group in Edge or Chrome)"; GroupDescription: "Browser:"

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{autoprograms}\Jarvis"; Filename: "{app}\Jarvis.exe"
Name: "{autodesktop}\Jarvis"; Filename: "{app}\Jarvis.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Jarvis"; ValueData: """{app}\Jarvis.exe"""; Flags: uninsdeletevalue; Tasks: startup

[Run]
Filename: "{tmp}\MicrosoftEdgeWebview2Setup.exe"; Parameters: "/silent /install"; StatusMsg: "Installing Microsoft Edge WebView2 (needed to show Jarvis)..."; Check: NeedsWebView2; Flags: waituntilterminated
Filename: "{app}\Jarvis.exe"; Parameters: "{code:LaunchParams}"; Description: "Start Jarvis"; Flags: nowait postinstall; Check: ShouldLaunch

[Messages]
FinishedLabel=Jarvis is installed.%n%nThe first time he starts, right-click the orb, open Settings, and add your Anthropic key (and an ElevenLabs key for his voice) under API keys. If you chose to set up the browser extension, Settings opens on the Browser tab and walks you through it (you can do it later from Settings, Browser).

[Code]
var
  DownloadPage: TDownloadWizardPage;

// Start Jarvis at the end of a normal install, and after a silent one only when asked (the app's own
// updater passes /LAUNCH so Jarvis comes back after it has replaced itself).
function HasParam(const Name: String): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), Name) = 0 then Result := True;
end;

function ShouldLaunch: Boolean;
begin
  Result := (not WizardSilent) or HasParam('/LAUNCH');
end;

// Open Settings on the browser-extension walkthrough after a normal install when the user ticked it. Never after a
// silent update (the app's updater), which would pop it up again every time.
function LaunchParams(Param: String): String;
begin
  Result := '';
  if (not WizardSilent) and WizardIsTaskSelected('browserext') then Result := '--browser-setup';
end;

// The WebView2 runtime ships with Windows 11 and recent Windows 10, and is shared with Edge.
function WebView2Version(Root: Integer; const Key: String): String;
begin
  Result := '';
  RegQueryStringValue(Root, Key, 'pv', Result);
end;

function NeedsWebView2: Boolean;
var
  Key, Version: String;
begin
  Key := 'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  Version := WebView2Version(HKLM32, Key);
  if (Version = '') or (Version = '0.0.0.0') then Version := WebView2Version(HKLM64, Key);
  if (Version = '') or (Version = '0.0.0.0') then Version := WebView2Version(HKCU, Key);
  Result := (Version = '') or (Version = '0.0.0.0');
end;

procedure InitializeWizard;
begin
  DownloadPage := CreateDownloadPage(SetupMessage(msgWizardPreparing), SetupMessage(msgPreparingDesc), nil);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID = wpReady) and NeedsWebView2 then
  begin
    DownloadPage.Clear;
    // Microsoft's official evergreen bootstrapper.
    DownloadPage.Add('https://go.microsoft.com/fwlink/p/?LinkId=2124703', 'MicrosoftEdgeWebview2Setup.exe', '');
    DownloadPage.Show;
    try
      try
        DownloadPage.Download;
      except
        if DownloadPage.AbortedByUser then
          Log('WebView2 download aborted')
        else
          SuppressibleMsgBox(AddPeriod(GetExceptionMessage) + ' Jarvis needs Microsoft Edge WebView2. You can install it from microsoft.com and run this setup again.', mbCriticalError, MB_OK, IDOK);
        Result := False;
      end;
    finally
      DownloadPage.Hide;
    end;
  end;
end;
