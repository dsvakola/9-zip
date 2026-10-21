; 9-zip installer hooks: add the Explorer menu after installing and remove it
; before uninstalling, so nothing is left in the registry.

!macro NSIS_HOOK_POSTINSTALL
  ExecWait '"$INSTDIR\9-zip.exe" --register-shell'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ExecWait '"$INSTDIR\9-zip.exe" --unregister-shell'
!macroend
