; Inno Setup script for the Rust version of ParameterSaveStates.
; Build with: iscc /DAppVersion=1.2.3 /DSourceDir=..\..\dist\windows installer.iss

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\dist\windows"
#endif

[Setup]
AppId={{6C1C2F0E-3B4B-4C55-9A1D-5A2D2F6D7E11}
AppName=ParameterSaveStates
AppVersion={#AppVersion}
AppPublisher=i5ucc
DefaultDirName={autopf}\ParameterSaveStates
DefaultGroupName=ParameterSaveStates
OutputDir=.
OutputBaseFilename=ParameterSaveStates-Setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\ParameterSaveStates.exe
CloseApplications=yes

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\ParameterSaveStates"; Filename: "{app}\ParameterSaveStates.exe"
Name: "{group}\Uninstall ParameterSaveStates"; Filename: "{uninstallexe}"
Name: "{autodesktop}\ParameterSaveStates"; Filename: "{app}\ParameterSaveStates.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional icons:"

[Run]
Filename: "{app}\ParameterSaveStates.exe"; Description: "Launch ParameterSaveStates"; Flags: nowait postinstall skipifsilent
