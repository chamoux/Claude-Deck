; Claude Deck installer hooks (Tauri NSIS, see bundle.windows.nsis.installerHooks).
; The background helper keeps deck-host.exe open, so stop it before files are
; replaced or removed. Its Claude sessions end with it (kill-on-close job);
; their conversations are saved by Claude and come back asleep.

!macro DECK_STOP_HELPER
  nsExec::Exec 'taskkill /F /IM deck-host.exe'
  Pop $0
  Sleep 1000
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro DECK_STOP_HELPER
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro DECK_STOP_HELPER
!macroend
