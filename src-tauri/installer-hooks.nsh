; Hooks are included before NSIS defines MANUFACTURER. Keep this name aligned
; with bundle.publisher in tauri.conf.json.
VIAddVersionKey "CompanyName" "Discord to VR"

; The application stores settings outside Tauri's bundle-ID directory.
; Match Tauri's explicit data-removal choice and preserve settings on updates.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$APPDATA\discord-to-vr"
  ${EndIf}
!macroend
