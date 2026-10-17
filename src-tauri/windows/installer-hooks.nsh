; StreamNook's additions to Tauri's NSIS installer (tauri.windows.conf.json,
; bundle.windows.nsis.installerHooks).
;
; The per-user install folder is $LOCALAPPDATA\StreamNook, which is ALSO where
; the app keeps its settings, cache, logs and plugins. Tauri's uninstaller only
; deletes the files it installed and removes the folder if it is empty, so data
; survives a normal uninstall and every update. Its "delete app data" checkbox
; only covers the $APPDATA\${BUNDLEID} and $LOCALAPPDATA\${BUNDLEID} folders, so
; the folders StreamNook names itself are removed here, under the same checkbox,
; and never while updating.

!macro NSIS_HOOK_POSTUNINSTALL
  ; The identity StreamNook registers for its Windows notifications
  ; (services/system_notify.rs). Meaningless once the app is gone.
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\${BUNDLEID}"

  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    ; Settings, cache, logs, plugins.
    RmDir /r "$LOCALAPPDATA\StreamNook"
    ; Sign-in vault, accounts, web profiles.
    RmDir /r "$APPDATA\StreamNook"
    ; The key that seals the sign-in vault (services/token_vault/os_store.rs).
    nsExec::Exec 'cmdkey.exe /delete:"StreamNook.StreamNook Safe Storage"'
    Pop $0
  ${EndIf}
!macroend
