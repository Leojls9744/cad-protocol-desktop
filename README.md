# CAD Protocol for Windows

The desktop app for [CAD Protocol](https://cad-dza.caffeine.xyz), the drafting app for land surveyors on the Internet Computer.

It's a small, open window around the live app. It loads `https://cad-dza.caffeine.xyz`, the same app you use in the browser, so your sign-in, vault, drawings and scans work exactly as they do there, and the app updates by itself. This repository holds only the window. You can read all of it here before installing: it's a few hundred lines in [`src/`](src), most of them comments.

## Download

**[Download CAD-Protocol-Setup.exe](https://github.com/leojls9744/cad-protocol-desktop/releases/latest/download/CAD-Protocol-Setup.exe)**. Or see [all releases](https://github.com/leojls9744/cad-protocol-desktop/releases) with their SHA-256 checksums.

1. Open `CAD-Protocol-Setup.exe`. It installs for your Windows account only, with no admin prompt, and adds Start menu and desktop shortcuts.
2. The installer isn't code-signed yet, so Windows SmartScreen may say "Windows protected your PC". Click **More info**, then **Run anyway**.
3. To check the file is the one GitHub built from this code, run `Get-FileHash .\CAD-Protocol-Setup.exe` in PowerShell and compare the hash with the one on the release page.

Each installer is built by GitHub Actions from the code in this repository ([the workflow](.github/workflows/build.yml)), not on anyone's own computer. Every build is also checked on Windows before it's kept.

## What it adds to the browser version

- **A faster pointer while you draft.** Windows has one pointer speed for the whole computer, from 1 to 20, and the default is 10. Drafting feels better a little faster. While a CAD Protocol window is in front, the app sets the speed to 15, or leaves yours if it's already faster.
  - **When yours comes back:** the moment you switch to another program, minimise, lock the screen or quit.
  - **Never saved:** the change is never written to your Windows settings, so signing out always resets it. If the app is ever stopped mid-session, the next launch puts your speed back.
  - **Changing it:** in the app, under Settings › Desktop app, pick a speed from 1 to 20, Automatic, or Off.
- **Every key goes to the drawing.** There's no menu bar, so no browser shortcuts get in the way.
- **More room.** No tabs or address bar, and its own window and taskbar icon.
- **Unsaved drawings are protected.** It asks before closing with unsaved changes, as the browser does.

## What it can and can't do

The window is locked down the way Electron's [security guidance](https://www.electronjs.org/docs/latest/tutorial/security) recommends.

- **Sandboxed pages.** Pages have no Node.js and run with context isolation.
- **The main window stays on the app.** It only shows pages from `https://cad-dza.caffeine.xyz`. Any other link opens in your own browser.
- **Sign-in.** Internet Identity (`id.ai`) opens in its own small window, so sign-in works as it does in the browser.
- **Report windows.** The app's own printable reports open in their own window.
- **One thing to the computer:** the app's page can reach the pointer-speed setting. The main process checks every request comes from the app's own address ([`src/preload.js`](src/preload.js), [`src/main.js`](src/main.js)).
- **Permissions:** only clipboard, full screen, pointer lock and notifications are allowed, and only to the app. The camera, microphone and location are refused.
- **Settings kept on the computer:** only the window's size and place, the pointer setting, and a short log, all in `%APPDATA%\CAD Protocol`. Drawings and files stay in your encrypted vault.
- **Hardened Electron options:** "run as Node" and debugging flags are switched off in the executable.

## Files

| File | What it does |
| --- | --- |
| [`src/main.js`](src/main.js) | The window, links, permissions, focus and the Windows build's check |
| [`src/config.js`](src/config.js) | The app's address and which pages may open where |
| [`src/pointer-speed.js`](src/pointer-speed.js) | Reading and setting the pointer speed (Windows' `SystemParametersInfo`, through [koffi](https://koffi.dev)) |
| [`src/preload.js`](src/preload.js) | The small `window.cadpDesktop` object the app sees |
| [`src/store.js`](src/store.js) | The settings file and log |
| [`src/offline.html`](src/offline.html) | Shown when the app can't be reached |
| [`test/`](test) | Unit tests, run with `npm test` |

## Building it yourself

You need Node.js 22 and Windows.

```
npm ci
npm test
npm start            # run it
npm run dist         # build dist\CAD-Protocol-Setup.exe
```

For the app's page, `window.cadpDesktop` exists only in the desktop app:

```js
if (window.cadpDesktop) {
  const status = await window.cadpDesktop.pointerSpeed.get();
  // { supported, chosen, automatic, off, own, inUse, defaultSpeed, min, max }
  await window.cadpDesktop.pointerSpeed.set(16); // null = automatic, 0 = off
}
```

## License

[MIT](LICENSE).
