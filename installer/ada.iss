; Ada installer (Inno Setup 6)
; Build: iscc installer\ada.iss   (expects dist\ada.exe to exist)
; Version comes from the ADA_VERSION environment variable, falls back to 0.1.0.

#define AppVersion GetEnv("ADA_VERSION")
#if AppVersion == ""
  #undef AppVersion
  #define AppVersion "0.1.0"
#endif

#define AppName "Ada"
#define AppExe "ada.exe"
#define AppUrl "https://github.com/Yemresalcan/ada-notch"

[Setup]
; Fixed GUID - never change it, upgrades and uninstall rely on it.
AppId={{E46D4404-B66F-4756-A5D8-4A9FDA09DFB5}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Yunus Emre Salcan
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}
AppUpdatesURL={#AppUrl}/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Ada
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.18362
OutputDir=..\dist
OutputBaseFilename=Ada-Setup-{#AppVersion}
SetupIconFile=..\assets\icon\ada.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
LicenseFile=..\LICENSE
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "turkish"; MessagesFile: "compiler:Languages\Turkish.isl"

[Tasks]
Name: "startup"; Description: "Start {#AppName} when I sign in to Windows"; GroupDescription: "Startup:"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\dist\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; Autostart entry (removed on uninstall)
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Ada"; ValueData: """{app}\{#AppExe}"""; Flags: uninsdeletevalue; Tasks: startup

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
// Kill a running Ada so the exe can be replaced / removed.
procedure KillAda();
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#AppExe}', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
end;

function InitializeSetup(): Boolean;
begin
  KillAda();
  Result := True;
end;

function InitializeUninstall(): Boolean;
begin
  KillAda();
  Result := True;
end;

// Settings and data are kept unless the user explicitly opts in to delete them.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and (not UninstallSilent) then
  begin
    if MsgBox('Also delete Ada settings and data?', mbConfirmation,
      MB_YESNO or MB_DEFBUTTON2) = IDYES then
    begin
      DelTree(ExpandConstant('{userappdata}\Ada'), True, True, True);
      DelTree(ExpandConstant('{localappdata}\Ada'), True, True, True);
    end;
  end;
end;
