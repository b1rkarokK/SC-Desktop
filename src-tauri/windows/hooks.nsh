; Desktop shortcut is created automatically (no checkbox) and removed on uninstall.
!macro NSIS_HOOK_POSTINSTALL
  CreateShortCut "$DESKTOP\SC Desk.lnk" "$INSTDIR\sc-desk.exe"
  ; scdesk:// links (rooms, shared playlists) open this program
  WriteRegStr HKCU "Software\Classes\scdesk" "" "URL:SC Desk"
  WriteRegStr HKCU "Software\Classes\scdesk" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\scdesk\shell\open\command" "" '"$INSTDIR\sc-desk.exe" "%1"'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$DESKTOP\SC Desk.lnk"
  DeleteRegKey HKCU "Software\Classes\scdesk"
!macroend
