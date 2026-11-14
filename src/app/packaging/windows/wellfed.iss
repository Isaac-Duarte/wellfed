; The Windows installer, built by `cargo xtask package-desktop` from a
; staged release build: the runner's Release folder plus the Visual C++
; runtime, which the runner and the Rust bridge both load and a clean Windows
; may not have.
;
; Per user, with no administrator prompt: the app goes to
; %LOCALAPPDATA%\Programs\Wellfed, and a Start menu shortcut beside it.
; The app's stores live elsewhere (%LOCALAPPDATA%\Wellfed, and the client
; secret under %APPDATA%), and the uninstaller never deletes them, since they
; hold the key every feeder knows this computer by.

#ifndef AppVersion
  #error Pass /DAppVersion=<the pubspec version>
#endif
#ifndef FileVersion
  #error Pass /DFileVersion=<four numbers>
#endif
#ifndef SourceDir
  #error Pass /DSourceDir=<the staged build>
#endif
#ifndef OutputDir
  #error Pass /DOutputDir=<the artifact folder>
#endif
#ifndef OutputName
  #error Pass /DOutputName=<the installer's name without .exe>
#endif

[Setup]
; Fixed for good: Windows finds the installed app, to upgrade or remove it,
; by this id.
AppId={{24CF1924-B47F-475A-9CB4-20CE6E0FE06E}
AppName=Wellfed
AppVersion={#AppVersion}
AppVerName=Wellfed {#AppVersion}
AppPublisher=Wellfed contributors
AppPublisherURL=https://well-fed.dev
AppSupportURL=https://well-fed.dev
VersionInfoVersion={#FileVersion}
VersionInfoProductName=Wellfed
VersionInfoDescription=Wellfed installer
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\Wellfed
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
; The runner's single-instance mutex: setup and the uninstaller ask the
; owner to close a running Wellfed first.
AppMutex=Local\io.github.isaacduarte.wellfed
SetupIconFile={#SourcePath}..\..\windows\runner\resources\app_icon.ico
UninstallDisplayIcon={app}\wellfed.exe
UninstallDisplayName=Wellfed
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; Named one by one so the build fails if the staged folder lacks one.
Source: "{#SourceDir}\wellfed.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\wellfed_app_bridge.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\flutter_windows.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\vcruntime140.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\data\app.so"; DestDir: "{app}\data"; Flags: ignoreversion
Source: "{#SourceDir}\*"; DestDir: "{app}"; Excludes: "*.lib,\wellfed.exe,\wellfed_app_bridge.dll,\flutter_windows.dll,\vcruntime140.dll,\data\app.so"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Wellfed"; Filename: "{app}\wellfed.exe"
Name: "{autodesktop}\Wellfed"; Filename: "{app}\wellfed.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\wellfed.exe"; Description: "{cm:LaunchProgram,Wellfed}"; Flags: nowait postinstall skipifsilent

; No [UninstallDelete]: removing the app leaves its stores, so installing
; again finds the same feeders.
