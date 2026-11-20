; Per-user installer: no admin rights, installs to %LOCALAPPDATA%\Programs\Converter.
; Build: makensis /DVERSION=0.1.0 /DSRC=<staged folder> /DOUT=<setup.exe> windows\installer.nsi
Unicode true
!include "MUI2.nsh"

Name "Converter"
OutFile "${OUT}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Converter"
InstallDirRegKey HKCU "Software\Converter" "InstallDir"
SetCompressor /SOLID lzma

!define UNKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Converter"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\converter-desktop.exe"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Install"
  ; Stop a running copy so the exe can be replaced.
  nsExec::Exec 'taskkill /F /IM converter-desktop.exe'
  SetOutPath "$INSTDIR"
  File /r "${SRC}\*.*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Converter.lnk" "$INSTDIR\converter-desktop.exe"
  WriteRegStr HKCU "Software\Converter" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNKEY}" "DisplayName" "Converter"
  WriteRegStr HKCU "${UNKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNKEY}" "Publisher" "Samad126"
  WriteRegStr HKCU "${UNKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNKEY}" "DisplayIcon" "$INSTDIR\converter-desktop.exe"
  WriteRegStr HKCU "${UNKEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNKEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNKEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /F /IM converter-desktop.exe'
  Delete "$SMPROGRAMS\Converter.lnk"
  ; Autostart value written by the app's own setting.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Converter"
  DeleteRegKey HKCU "${UNKEY}"
  DeleteRegKey HKCU "Software\Converter"
  ; Only the files we installed; engines/settings live in the app data folder and are kept.
  RMDir /r "$INSTDIR"
SectionEnd
