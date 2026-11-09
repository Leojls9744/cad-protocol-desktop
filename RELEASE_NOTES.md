CAD Protocol for Windows opens the live CAD Protocol app (https://cad-dza.caffeine.xyz) in its own window. It's the same app, with the same sign-in, vault and drawings, and it updates by itself.

**New in 1.4: a purple title bar.** On Windows 11 the bar across the top of every CAD Protocol window is now CAD Protocol purple with white text, instead of your Windows accent color. Windows 10 keeps its usual bar. 1.3 updates itself to 1.4; nothing to install.

**New in 1.3: it updates itself.** From now on the desktop app keeps itself up to date: when it opens, it checks for a new version and, if there is one, installs it on an update screen and restarts; while it's open it checks every 6 hours, downloads quietly and installs when you restart or close (the badge in the top bar says "Update ready – Restart"). No more running the installer. This is the last version you install by hand: install 1.3 over 1.1; your settings stay.

Also in 1.3: closing the window saves your open drawings to your vault and signs you out, unless you turn that off in Settings › Account and security; with the app's newest web build, every sign-in also signs out after a time without activity (10 minutes unless you change it), with a minute's warning, and a desktop sign-in lasts 8 hours unless you change it.

**New in 1.1: Sign in with your browser.** Google doesn't allow its sign-in inside an app's own window ("This browser or app may not be secure"), so the sign-in card now has **Sign in with your browser**. It opens CAD Protocol in your own browser, where you sign in as usual and press **Continue**. Internet Identity opens once more; pick the same identity. Your browser tab then says you're signed in, and the desktop app comes to the front, signed in to the same account with a session of its own. A passkey (Windows Hello) still works in the desktop app directly: **Sign in here with a passkey**. Installing 1.1 over 1.0 keeps your settings.

What the desktop app adds:

- **Faster pointer while you draft.** While CAD Protocol is in front, the pointer speed is set to 15 on Windows' 1–20 scale, or kept if yours is already faster. Your own speed comes back the moment you switch to another program, minimise, lock the screen or quit. It's never saved to your Windows settings. You can change it or turn it off in the app's Settings.
- **Every key goes to the drawing.** No browser shortcuts get in the way.
- **More room.** No tabs or address bar, and its own window and taskbar icon.

**Installing:** download `CAD-Protocol-Setup.exe` and open it. It installs for your Windows account only, with no admin prompt, and adds Start menu and desktop shortcuts.

**"Windows protected your PC":** the installer isn't code-signed yet, so Windows SmartScreen may warn you. Click **More info**, then **Run anyway**. Before you do, you can check the file is the one built here: run `Get-FileHash .\CAD-Protocol-Setup.exe` in PowerShell and compare the hash with the SHA-256 below.

**Uninstalling:** Settings › Apps › Installed apps › CAD Protocol › Uninstall. Your drawings stay in your vault; nothing is kept on the computer except the window size and pointer setting.
