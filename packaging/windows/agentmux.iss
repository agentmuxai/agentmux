; AgentMux Windows installer (Inno Setup 6) — native CEF.
;
; Built fresh after the Tauri removal (the old tauri-action NSIS/WiX path is gone).
; Packages a `task package:release` portable into a per-user install that runs in
; INSTALLED mode (per-user data dir), NOT portable mode — achieved by omitting the
; `agentmux-portable.marker` (+ the seed `data\` and `README.txt`), exactly like
; scripts/package-msix.ps1's staging. See agentmux-common::runtime_mode.
;
; Invoke via scripts/package-installer.ps1 (or `task package:installer`), which
; passes these defines:
;   /DAppVersion=<x.y.z>  /DSourceDir=<portable dir>  /DOutputDir=<output dir>

#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif
#ifndef SourceDir
  #error SourceDir is required (the unpacked portable folder)
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif

#define AppName "AgentMux"
#define AppExe "agentmux.exe"
#define AppPublisher "AgentMux"
#define AppURL "https://github.com/agentmuxai/agentmux"
; Repo-relative icon (this .iss lives in packaging/windows/).
#define IconFile "..\..\crates\cef\resources\win\agentmux.ico"

[Setup]
; Stable AppId — keep constant across versions so upgrades replace in place.
AppId={{8654EA96-8EAA-4845-ADE9-352ADA712BC0}}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
; Per-user install → no UAC elevation (the build is unsigned; keeps testing simple).
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=AgentMux-{#AppVersion}-x64-setup
; The installer's own .exe icon + Add/Remove Programs icon.
SetupIconFile={#IconFile}
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName} {#AppVersion}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional icons:"; Flags: unchecked

[Files]
; agentmux.exe (root launcher) + the whole runtime\ tree. Deliberately NOT shipping
; the portable marker / seed data\ / README.txt (→ installed-mode data dir).
Source: "{#SourceDir}\agentmux.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\runtime\*"; DestDir: "{app}\runtime"; Flags: ignoreversion recursesubdirs createallsubdirs
; Ship the icon so shortcuts reference a stable file (independent of exe icon embedding).
Source: "{#IconFile}"; DestDir: "{app}"; DestName: "agentmux.ico"; Flags: ignoreversion

[Icons]
; /NOICONS means no Start Menu folder. Setup only honours that switch itself on the program group page,
; which this script turns off, so the two group entries read it here.
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"; IconFilename: "{app}\agentmux.ico"; Check: not NoIconsRequested
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"; Check: not NoIconsRequested
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; IconFilename: "{app}\agentmux.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

[Code]
// True when setup was started with /NOICONS (winget or a script may pass it).
function NoIconsRequested: Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/NOICONS') = 0 then
      Result := True;
end;
