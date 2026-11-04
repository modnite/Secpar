; Secpar Host installer hooks (Tauri NSIS). The installer runs elevated (perMachine), or as
; SYSTEM with /S when the service installs an update.
; Side-by-side install: no shared drivers or legacy applications are modified.

!define PCR_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define PCR_FW_RULE "Secpar Host"
!define PCR_SVC "Secpar"

; Stop the service (it ends its server) and the tray before the files are replaced. Runs before
; Tauri's own "app is running" check, so that check finds nothing to kill.
!macro NSIS_HOOK_PREINSTALL
  nsExec::Exec 'sc.exe stop "${PCR_SVC}"'
  ; up to ~10 s for the service to reach STOPPED (skipped when it isn't installed yet)
  StrCpy $1 0
  pcr_wait_stop:
    nsExec::Exec 'cmd.exe /c sc.exe query "${PCR_SVC}" | findstr /c:"RUNNING" /c:"PENDING"'
    Pop $0
    ${If} $0 == 0
    ${AndIf} $1 < 20
      IntOp $1 $1 + 1
      Sleep 500
      Goto pcr_wait_stop
    ${EndIf}
  pcr_stopped:
  nsExec::Exec 'taskkill.exe /F /IM secpar-host.exe'
  ; 2.x: the elevated start task and the per-user autostart are replaced by the service
  nsExec::Exec 'schtasks.exe /Delete /TN "Secpar" /F'
  DeleteRegValue HKCU "${PCR_RUN_KEY}" "Secpar"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Reuse an existing gamepad driver. Side-by-side setup never installs shared drivers.

  ; Phones reach the agent on TCP 48999 (TLS, token-authenticated) and stream over UDP (48998 when free).
  nsExec::Exec 'netsh advfirewall firewall delete rule name="${PCR_FW_RULE}"'
  nsExec::Exec 'netsh advfirewall firewall add rule name="${PCR_FW_RULE}" dir=in action=allow protocol=TCP localport=48999 program="$INSTDIR\secpar-host.exe" profile=any'
  nsExec::Exec 'netsh advfirewall firewall add rule name="${PCR_FW_RULE}" dir=in action=allow protocol=UDP program="$INSTDIR\secpar-host.exe" profile=any'

  ; The service: starts at boot (before sign-in), restarts after a crash.
  DetailPrint "Registering the Secpar service…"
  nsExec::Exec 'sc.exe query "${PCR_SVC}"'
  Pop $0
  ${If} $0 == 0
    nsExec::Exec 'sc.exe config "${PCR_SVC}" binPath= "\"$INSTDIR\secpar-host.exe\" --service" start= auto obj= LocalSystem'
  ${Else}
    nsExec::Exec 'sc.exe create "${PCR_SVC}" binPath= "\"$INSTDIR\secpar-host.exe\" --service" start= auto obj= LocalSystem DisplayName= "Secpar"'
  ${EndIf}
  nsExec::Exec 'sc.exe description "${PCR_SVC}" "Lets your paired phone see and control this PC, at the sign-in screen too."'
  nsExec::Exec 'sc.exe failure "${PCR_SVC}" reset= 86400 actions= restart/5000/restart/5000/restart/30000'

  ; Only Secpar is started. No PC Remote, Sunshine, power or adapter settings are changed.
  nsExec::Exec 'sc.exe start "${PCR_SVC}"'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'sc.exe stop "${PCR_SVC}"'
  Sleep 2000
  nsExec::Exec 'sc.exe delete "${PCR_SVC}"'
  nsExec::Exec 'taskkill.exe /F /IM secpar-host.exe'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="${PCR_FW_RULE}"'
  nsExec::Exec 'schtasks.exe /Delete /TN "Secpar" /F'
  DeleteRegValue HKCU "${PCR_RUN_KEY}" "Secpar"
  DeleteRegKey HKCU "Software\Classes\*\shell\Secpar.Send"
!macroend
