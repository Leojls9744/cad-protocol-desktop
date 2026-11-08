CAD Protocol for Windows opens the live CAD Protocol app (https://cad-dza.caffeine.xyz) in its own window. It's the same app, with the same sign-in, vault and drawings, and it updates by itself.

What the desktop app adds:

- **Faster pointer while you draft.** While CAD Protocol is in front, the pointer speed is set to 15 on Windows' 1–20 scale, or kept if yours is already faster. Your own speed comes back the moment you switch to another program, minimise, lock the screen or quit. It's never saved to your Windows settings. You can change it or turn it off in the app's Settings.
- **Every key goes to the drawing.** No browser shortcuts get in the way.
- **More room.** No tabs or address bar, and its own window and taskbar icon.

**Installing:** download `CAD-Protocol-Setup.exe` and open it. It installs for your Windows account only, with no admin prompt, and adds Start menu and desktop shortcuts.

**"Windows protected your PC":** the installer isn't code-signed yet, so Windows SmartScreen may warn you. Click **More info**, then **Run anyway**. Before you do, you can check the file is the one built here: run `Get-FileHash .\CAD-Protocol-Setup.exe` in PowerShell and compare the hash with the SHA-256 below.

**Uninstalling:** Settings › Apps › Installed apps › CAD Protocol › Uninstall. Your drawings stay in your vault; nothing is kept on the computer except the window size and pointer setting.
