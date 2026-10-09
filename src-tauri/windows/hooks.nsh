; Desktop shortcut is created automatically (no checkbox) and removed on uninstall.
!macro NSIS_HOOK_POSTINSTALL
  CreateShortCut "$DESKTOP\SC Desk.lnk" "$INSTDIR\sc-desk.exe"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$DESKTOP\SC Desk.lnk"
!macroend
