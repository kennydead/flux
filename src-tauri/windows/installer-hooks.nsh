; Flux AI was called "Flux" up to 0.5.8. The installer keys its install folder, Start menu entry and
; uninstall entry by product name, so without this an update would put "Flux AI" NEXT TO the old
; "Flux". Before installing, remove that old install quietly. Settings live under the bundle
; identifier (unchanged) and the farm runs in Docker, so both stay. (owner 2026-11-18)

!define LEGACY_UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Flux"
!define RUNKEY "Software\Microsoft\Windows\CurrentVersion\Run"

; The old uninstaller deletes its own "launch at login" entry: remember it, and set Flux AI's afterwards.
Var FluxAILegacyAutostart

!macro FLUXAI_UNQUOTE VAR
  StrCpy $R9 ${VAR} 1
  StrCmp $R9 '"' 0 +3
    StrCpy ${VAR} ${VAR} "" 1
    StrCpy ${VAR} ${VAR} -1
!macroend

!macro NSIS_HOOK_PREINSTALL
  StrCpy $FluxAILegacyAutostart ""
  ReadRegStr $FluxAILegacyAutostart HKCU "${RUNKEY}" "Flux"
  ReadRegStr $R7 SHCTX "${LEGACY_UNINSTKEY}" "UninstallString"
  StrCmp $R7 "" fluxai_no_legacy
    ReadRegStr $R8 SHCTX "${LEGACY_UNINSTKEY}" "InstallLocation"
    !insertmacro FLUXAI_UNQUOTE $R7
    !insertmacro FLUXAI_UNQUOTE $R8
    DetailPrint "Removing the previous version (Flux) from $R8"
    ; _?= runs the uninstaller in place and waits for it; /S = silent (keeps app data).
    ExecWait '"$R7" /S _?=$R8'
    Delete "$R7"
    RMDir "$R8"
    DeleteRegKey SHCTX "${LEGACY_UNINSTKEY}"
  fluxai_no_legacy:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  StrCmp $FluxAILegacyAutostart "" fluxai_no_autostart
    WriteRegStr HKCU "${RUNKEY}" "${PRODUCTNAME}" '"$INSTDIR\${MAINBINARYNAME}.exe"'
  fluxai_no_autostart:
!macroend
