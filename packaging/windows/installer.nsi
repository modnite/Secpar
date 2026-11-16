; Secpar for Windows: one application that hosts and connects.
;
; Per-user install into %LOCALAPPDATA%\Programs\Secpar, so an update needs no
; elevation. The background part (secpar-service.exe with the bundled engine)
; starts on first run and again at every sign-in through the HKCU Run entry.
; This computer's identity and the engine's state live in %LOCALAPPDATA%\Secpar4,
; which the installer never writes to and the uninstaller never removes.
;
; Build with packaging\windows\build-installer.ps1, which passes APP_VERSION,
; PAYLOAD, OUTFILE and ICON.

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!ifndef APP_VERSION
  !error "APP_VERSION is required"
!endif
!ifndef PAYLOAD
  !error "PAYLOAD is required (folder with the installed files)"
!endif
!ifndef OUTFILE
  !error "OUTFILE is required"
!endif
!ifndef ICON
  !error "ICON is required"
!endif

!define APP_NAME "Secpar"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Secpar"

Name "${APP_NAME} ${APP_VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\Secpar"
; An update goes back into the folder the previous install chose.
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 64
BrandingText "${APP_NAME} ${APP_VERSION}"
Icon "${ICON}"
UninstallIcon "${ICON}"
VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "FileVersion" "${APP_VERSION}"
VIAddVersionKey "CompanyName" "Secpar"
VIAddVersionKey "FileDescription" "Secpar installer"
VIAddVersionKey "LegalCopyright" "GPL-3.0"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\secpar.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Open Secpar"
!define MUI_FINISHPAGE_TEXT "Secpar is installed. This computer is ready to host and to connect."
!define MUI_UNCONFIRMPAGE_TEXT_TOP "Removing Secpar deletes the program files only. Your account, this computer's identity in %LOCALAPPDATA%\Secpar4 and the engine settings are kept."

!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Var Update
Var Restart

; The background part and the engine are separate processes, so they must be
; stopped before their files are replaced or removed.
!macro StopRunningFromInstallDir
  SetOutPath "$INSTDIR"
  nsExec::ExecToStack 'powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$PLUGINSDIR\stop-secpar.ps1" -Root "$INSTDIR"'
  Pop $0
!macroend

Function .onInit
  StrCpy $Update "0"
  StrCpy $Restart "0"
  ${GetParameters} $R0
  ${GetOptions} $R0 "/RESTART" $R1
  ${IfNot} ${Errors}
    StrCpy $Restart "1"
  ${EndIf}
  ; An install over an existing one keeps the folder, updates the files and
  ; leaves this computer's identity and engine state where they are.
  ReadRegStr $R2 HKCU "${UNINSTALL_KEY}" "DisplayVersion"
  ${If} $R2 != ""
    StrCpy $Update "1"
  ${EndIf}
  InitPluginsDir
  File /oname=$PLUGINSDIR\stop-secpar.ps1 "${__FILEDIR__}\stop-secpar.ps1"
FunctionEnd

Function un.onInit
  InitPluginsDir
  File /oname=$PLUGINSDIR\stop-secpar.ps1 "${__FILEDIR__}\stop-secpar.ps1"
FunctionEnd

Section "Secpar" SecMain
  SetShellVarContext current
  !insertmacro StopRunningFromInstallDir

  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD}\*.*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateDirectory "$SMPROGRAMS"
  CreateShortcut "$SMPROGRAMS\Secpar.lnk" "$INSTDIR\secpar.exe" "" "$INSTDIR\secpar.exe" 0

  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Secpar"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Secpar"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\secpar.exe,0"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" "$INSTDIR\Uninstall.exe /S"
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" "$0"

  ; The background part starts with the person's session, and again now, so
  ; hosting works before the window is opened. --start-host is the window
  ; starting it without a console.
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Secpar" '"$INSTDIR\secpar.exe" --start-host'
  ExecWait '"$INSTDIR\secpar.exe" --start-host'

  ${If} $Update == "1"
  ${AndIf} $Restart == "1"
    Exec '"$INSTDIR\secpar.exe"'
  ${EndIf}
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  !insertmacro StopRunningFromInstallDir

  Delete "$SMPROGRAMS\Secpar.lnk"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Secpar"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  ; Program files only: %LOCALAPPDATA%\Secpar4 and %LOCALAPPDATA%\Secpar hold
  ; this computer's identity, the engine's state and the stream settings.
  RMDir /r "$INSTDIR"
SectionEnd
