; FrameMap Studio — Windows installer (NSIS 3)
; Build from the repository root after `cargo build --release`:
;   makensis packaging\windows\installer.nsi

Unicode true
!define APPNAME   "FrameMap Studio"
!define APPID     "FrameMapStudio"
!define VERSION   "1.0.0"
!define PUBLISHER "Muruhukye Jacob"
!define EXE       "framemap.exe"
!define ROOT      "..\.."

Name "${APPNAME}"
OutFile "${ROOT}\dist\FrameMap-Studio-Setup-${VERSION}.exe"
InstallDir "$PROGRAMFILES64\${APPNAME}"
InstallDirRegKey HKLM "Software\${APPID}" "InstallDir"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
BrandingText "${APPNAME} ${VERSION}"

!include "MUI2.nsh"
!define MUI_ICON   "${ROOT}\assets\icon.ico"
!define MUI_UNICON "${ROOT}\assets\icon.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APPNAME}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${ROOT}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "FileDescription" "${APPNAME} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT License"

Section "Install"
  SetShellVarContext all
  SetOutPath "$INSTDIR"
  File "${ROOT}\target\release\${EXE}"
  File "${ROOT}\assets\icon.ico"
  File "${ROOT}\LICENSE"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateDirectory "$SMPROGRAMS\${APPNAME}"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\icon.ico"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\Uninstall ${APPNAME}.lnk" "$INSTDIR\Uninstall.exe"
  CreateShortcut "$DESKTOP\${APPNAME}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\icon.ico"

  ; .fmap files open in FrameMap Studio when double-clicked
  WriteRegStr HKCR ".fmap" "" "FrameMap.Diagram"
  WriteRegStr HKCR "FrameMap.Diagram" "" "FrameMap diagram"
  WriteRegStr HKCR "FrameMap.Diagram\DefaultIcon" "" "$INSTDIR\icon.ico"
  WriteRegStr HKCR "FrameMap.Diagram\shell\open\command" "" '"$INSTDIR\${EXE}" "%1"'

  ; Add/Remove Programs entry
  WriteRegStr HKLM "Software\${APPID}" "InstallDir" "$INSTDIR"
  !define UNINST "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPID}"
  WriteRegStr HKLM "${UNINST}" "DisplayName" "${APPNAME}"
  WriteRegStr HKLM "${UNINST}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKLM "${UNINST}" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKLM "${UNINST}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKLM "${UNINST}" "InstallLocation" "$INSTDIR"
  WriteRegDWORD HKLM "${UNINST}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST}" "NoRepair" 1
  WriteRegDWORD HKLM "${UNINST}" "EstimatedSize" 16000

  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

Section "Uninstall"
  SetShellVarContext all
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk"
  Delete "$SMPROGRAMS\${APPNAME}\Uninstall ${APPNAME}.lnk"
  RMDir "$SMPROGRAMS\${APPNAME}"
  Delete "$DESKTOP\${APPNAME}.lnk"
  DeleteRegKey HKCR "FrameMap.Diagram"
  ReadRegStr $0 HKCR ".fmap" ""
  StrCmp $0 "FrameMap.Diagram" 0 +2
    DeleteRegKey HKCR ".fmap"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPID}"
  DeleteRegKey HKLM "Software\${APPID}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd
