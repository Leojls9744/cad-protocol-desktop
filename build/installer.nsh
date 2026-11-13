; 1.15: cadprotocol:// links are the app's, so another app (UnifyField, say)
; can open it with cadprotocol://open. electron-builder's installer for
; Windows doesn't register build.protocols itself (only its Store, Mac and
; Linux packages do), so it's written here, as Windows wants a link's
; program: the same entry the app writes for itself each time it starts
; (app.setAsDefaultProtocolClient, src/app-link.js).
;
; 1.11: after the installer registers .cadp (its document icon and Open
; with CAD Protocol), Explorer is told file types changed, so .cadp files
; show the new icon straight away instead of after a restart.
!macro customInstall
  WriteRegStr SHELL_CONTEXT "Software\Classes\cadprotocol" "" "URL:CAD Protocol"
  WriteRegStr SHELL_CONTEXT "Software\Classes\cadprotocol" "URL Protocol" ""
  WriteRegStr SHELL_CONTEXT "Software\Classes\cadprotocol\DefaultIcon" "" "$appExe,0"
  WriteRegStr SHELL_CONTEXT "Software\Classes\cadprotocol\shell\open\command" "" '"$appExe" "%1"'
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x1000, i 0, i 0)'
!macroend

; Uninstalled (not while an update puts the new version in): the links go
; with the app, so none is left pointing at a program that isn't there.
!macro customUnInstall
  ${ifNot} ${isUpdated}
    DeleteRegKey SHELL_CONTEXT "Software\Classes\cadprotocol"
  ${endIf}
!macroend
