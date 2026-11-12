; 1.11: after the installer registers .cadp (its document icon and Open
; with CAD Protocol), Explorer is told file types changed, so .cadp files
; show the new icon straight away instead of after a restart.
!macro customInstall
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x1000, i 0, i 0)'
!macroend
