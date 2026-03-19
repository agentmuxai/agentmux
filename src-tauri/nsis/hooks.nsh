; AgentMux NSIS Installer Hooks
; Microsoft Store return code compliance
;
; Codes handled here (via hooks):
;   0 - Success (POSTINSTALL)
;   4 - Disk full (PREINSTALL)
;   5 - Reboot required (POSTINSTALL)
;   7 - Package rejected (POSTINSTALL)
;
; Codes handled in custom template (installer.nsi):
;   1 - User cancelled (.onUserAbort)
;   3 - Already in progress (.onInit mutex)
;   6 - Network failure (WebView2 error paths)
;
; See agentmux/specs/installer-return-codes.md for full specification.

!define RC_SUCCESS 0
!define RC_CANCELLED 1
; RC 2 (already exists) is reserved for future silent-mode detection
!define RC_ALREADY_IN_PROGRESS 3
!define RC_DISK_FULL 4
!define RC_REBOOT_REQUIRED 5
!define RC_NETWORK_FAILURE 6
!define RC_PACKAGE_REJECTED 7

!macro NSIS_HOOK_PREINSTALL
  ; Check disk space before file copy (RC 4)
  ${GetRoot} "$INSTDIR" $R0
  ${DriveSpace} "$R0\" "/D=F /S=M" $R1
  ; Need at least 100MB free (covers estimated install size + margin)
  ${If} $R1 < 100
    SetErrorLevel ${RC_DISK_FULL}
    Abort
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Check if main binary was actually installed (RC 7)
  ${IfNot} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    SetErrorLevel ${RC_PACKAGE_REJECTED}
    Abort
  ${EndIf}

  ; Check if reboot is needed (RC 5)
  ${If} ${RebootFlag}
    SetErrorLevel ${RC_REBOOT_REQUIRED}
  ${Else}
    SetErrorLevel ${RC_SUCCESS}
  ${EndIf}
!macroend
