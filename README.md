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
- **Sign in with your browser.** Google doesn't allow its sign-in inside an app's own window, so the desktop app signs in through your browser instead, the way native apps are meant to ([RFC 8252](https://www.rfc-editor.org/rfc/rfc8252)):
  - **Your key stays here:** the app's page makes a session key, which never leaves the desktop app.
  - **One listener, this computer only:** [`src/browser-sign-in.js`](src/browser-sign-in.js) listens once on `127.0.0.1` with a random state, for up to 10 minutes.
  - **You sign in as usual:** it opens CAD Protocol's `/desktop-sign-in` page in your browser. You sign in there (Google works) and press Continue.
  - **A session of its own:** the page starts a separate Internet Identity session for the same account and delegates it to the desktop app's key. It hands that back to `127.0.0.1`, where the state is checked, and the desktop app is signed in. Signing out in one leaves the other signed in.
  - **A passkey still works directly in the desktop app.**
- **Signing out on its own (1.2).** Before the window closes, the app gets a minute to save open drawings to your vault and sign you out, if your setting says so ([`src/close-guard.js`](src/close-guard.js)). The app also signs out after a time without activity, and a desktop sign-in lasts 8 hours unless you change it. Both are in Settings › Account and security.
- **New versions (1.2).** The app checks GitHub for a newer installer at most every 6 hours ([`src/updates.js`](src/updates.js)), and the app says so once per new version. Nothing else is sent.
- **Every key goes to the drawing.** There's no menu bar, so no browser shortcuts get in the way.
- **More room.** No tabs or address bar, and its own window and taskbar icon.
- **Unsaved drawings are protected.** It asks before closing with unsaved changes, as the browser does.

## What it can and can't do

The window is locked down the way Electron's [security guidance](https://www.electronjs.org/docs/latest/tutorial/security) recommends.

- **Sandboxed pages.** Pages have no Node.js and run with context isolation.
- **The main window stays on the app.** It only shows pages from `https://cad-dza.caffeine.xyz`. Any other link opens in your own browser.
- **Sign-in.** Internet Identity (`id.ai`) opens in its own small window, so sign-in works as it does in the browser.
- **Report windows.** The app's own printable reports open in their own window.
- **A few things to the computer:** the app's page can reach the pointer-speed setting, start Sign in with your browser, read whether a newer installer is out, and answer before the window closes. The main process checks every request comes from the app's own address ([`src/preload.js`](src/preload.js), [`src/main.js`](src/main.js)).
- **Permissions:** only clipboard, full screen, pointer lock and notifications are allowed, and only to the app. The camera, microphone and location are refused.
- **Settings kept on the computer:** only the window's size and place, the pointer setting, and a short log, all in `%APPDATA%\CAD Protocol`. Drawings and files stay in your encrypted vault.
- **Hardened Electron options:** "run as Node" and debugging flags are switched off in the executable.

## Files

| File | What it does |
| --- | --- |
| [`src/main.js`](src/main.js) | The window, links, permissions, focus and the Windows build's check |
| [`src/config.js`](src/config.js) | The app's address and which pages may open where |
| [`src/close-guard.js`](src/close-guard.js) | Asking the app before the window closes |
| [`src/updates.js`](src/updates.js) | Whether a newer installer is out |
| [`src/browser-sign-in.js`](src/browser-sign-in.js) | Sign in with your browser: the one-use listener on `127.0.0.1` |
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
  // 1.1 on: { chain, principal } once the browser hands the sign-in back
  const { chain, principal } = await window.cadpDesktop.signIn.withBrowser(publicKeyHex);
}
```

## License

[MIT](LICENSE).
