; The desktop shortcut is made once, on the first install (no checkbox). An update does not bring it back
; if the user deleted it: "DesktopShortcutMade" in HKCU says it was made before. Removed with the program.

; a program installed before this flag existed: its shortcut question is already settled
!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $0 HKCU "Software\SC Desk" "DesktopShortcutMade"
  StrCmp $0 "" 0 scdesk_pre_done
  IfFileExists "$INSTDIR\sc-desk.exe" 0 scdesk_pre_done
  WriteRegStr HKCU "Software\SC Desk" "DesktopShortcutMade" "1"
  scdesk_pre_done:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ReadRegStr $0 HKCU "Software\SC Desk" "DesktopShortcutMade"
  StrCmp $0 "" 0 scdesk_post_done
  CreateShortCut "$DESKTOP\SC Desk.lnk" "$INSTDIR\sc-desk.exe"
  WriteRegStr HKCU "Software\SC Desk" "DesktopShortcutMade" "1"
  scdesk_post_done:
  ; scdesk:// links (rooms, shared playlists) open this program
  WriteRegStr HKCU "Software\Classes\scdesk" "" "URL:SC Desk"
  WriteRegStr HKCU "Software\Classes\scdesk" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\scdesk\shell\open\command" "" '"$INSTDIR\sc-desk.exe" "%1"'
!macroend

; an update runs the old uninstaller too: only a real uninstall takes the shortcut and the flag away
!macro NSIS_HOOK_POSTUNINSTALL
  StrCmp $UpdateMode "1" scdesk_un_done
  Delete "$DESKTOP\SC Desk.lnk"
  DeleteRegValue HKCU "Software\SC Desk" "DesktopShortcutMade"
  DeleteRegKey /ifempty HKCU "Software\SC Desk"
  DeleteRegKey HKCU "Software\Classes\scdesk"
  scdesk_un_done:
!macroend
