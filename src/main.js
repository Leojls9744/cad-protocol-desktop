"use strict";
// CAD Protocol for Windows: a small window around the live CAD Protocol app.
//
// It loads https://cad-dza.caffeine.xyz, the same app as in the browser, so
// drawings, the vault and sign-in all work exactly as they do there and the
// app updates itself. What the desktop app adds:
// - its own window and taskbar icon, with no browser bars or tabs;
// - a faster pointer while CAD Protocol is in front (see pointer-speed.js),
//   put back the moment it isn't;
// - links to other sites open in the person's own browser;
// - "Sign in with your browser", since Google won't sign anyone in inside an
//   app's own window (see browser-sign-in.js).
//
// It has no access to the computer beyond that: pages get no Node.js, run
// sandboxed, and only the app's own pages may load in the main window.

const path = require("node:path");
const fs = require("node:fs");
const { pathToFileURL } = require("node:url");
const { app, BrowserWindow, dialog, ipcMain, Menu, powerMonitor, screen, session, shell } = require("electron");
const config = require("./config");
const { createStore, createLog } = require("./store");
const { createPointerSpeed, windowsBackend } = require("./pointer-speed");
const { createBrowserSignIn } = require("./browser-sign-in");

const OFFLINE_FILE = path.join(__dirname, "offline.html");
const OFFLINE_URL = pathToFileURL(OFFLINE_FILE).href;

// ---------------------------------------------------------------------------
// --smoke-test=<file>: used by the build on GitHub to check, on real Windows,
// that the packaged app can read and set the pointer speed. It changes the
// speed for a moment, puts it back, writes what it saw to <file> and quits.
// ---------------------------------------------------------------------------
const smokeArg = process.argv.find((a) => a.startsWith("--smoke-test="));
if (smokeArg) {
  app.whenReady().then(() => {
    const out = smokeArg.slice("--smoke-test=".length);
    const backend = windowsBackend((line) => fs.appendFileSync(`${out}.log`, `${line}\n`));
    const result = { version: app.getVersion(), electron: process.versions.electron, supported: backend.supported };
    try {
      result.before = backend.get();
      if (backend.supported && result.before !== null) {
        result.target = result.before === 15 ? 14 : 15;
        result.setOk = backend.set(result.target);
        result.during = backend.get();
        result.restoreOk = backend.set(result.before);
        result.after = backend.get();
      }
      result.config = {
        app: config.APP_URL,
        signIn: config.openKind("https://id.ai/authorize"),
        report: config.openKind(`blob:${config.APP_ORIGIN}/0b8f`),
        external: config.openKind("https://github.com/"),
      };
    } catch (err) {
      result.error = String(err && err.stack ? err.stack : err);
    }
    fs.writeFileSync(out, JSON.stringify(result, null, 2));
    app.exit(0);
  });
} else if (!app.requestSingleInstanceLock()) {
  // One CAD Protocol at a time, so two copies never fight over the pointer.
  app.quit();
} else {
  start();
}

function start() {
  app.setAppUserModelId("com.cadprotocol.desktop");
  app.userAgentFallback = config.plainUserAgent(app.userAgentFallback);
  Menu.setApplicationMenu(null); // no menu bar: every key goes to the drawing

  const data = app.getPath("userData");
  const log = createLog(path.join(data, "desktop.log"));
  const store = createStore(path.join(data, "desktop-settings.json"), log);
  let pointer = null;
  let mainWindow = null;
  const browserSignIn = createBrowserSignIn({
    appUrl: config.APP_URL,
    openExternal: (url) => shell.openExternal(url),
    focusApp: () => bringToFront(),
    log,
  });

  /** Which kind each window is: "main", "signin" or "report" (see config.js). */
  const kinds = new WeakMap();

  process.on("uncaughtException", (err) => {
    try {
      if (pointer) pointer.blur();
    } catch {
      // Already failing; carry on closing.
    }
    log(`uncaught: ${err && err.stack ? err.stack : err}`);
    dialog.showErrorBox("CAD Protocol", "Something went wrong and CAD Protocol has to close. Your pointer speed is back to your own.");
    app.exit(1);
  });

  app.on("second-instance", () => {
    if (!mainWindow) return;
    if (mainWindow.isMinimized()) mainWindow.restore();
    mainWindow.focus();
  });

  app.on("web-contents-created", (_event, contents) => {
    contents.on("will-attach-webview", (event) => event.preventDefault());

    contents.setWindowOpenHandler(({ url }) => {
      const kind = config.openKind(url, kinds.get(contents));
      if (kind === "signin" || kind === "report") {
        return {
          action: "allow",
          overrideBrowserWindowOptions: {
            autoHideMenuBar: true,
            backgroundColor: "#ffffff",
            webPreferences: secureWebPreferences(),
          },
        };
      }
      if (kind === "external") openOutside(url);
      return { action: "deny" };
    });

    contents.on("did-create-window", (child, details) => {
      kinds.set(child.webContents, config.openKind(details.url, kinds.get(contents)));
    });

    contents.on("will-navigate", (event, url) => {
      const kind = kinds.get(contents) || "report";
      if (config.canNavigate(kind, url, OFFLINE_URL)) return;
      event.preventDefault();
      if (config.isExternalSafe(url)) openOutside(url);
    });
  });

  app.whenReady().then(() => {
    pointer = createPointerSpeed({ backend: windowsBackend(log), store, log });
    pointer.recover();
    guardPermissions();
    answerTheApp();
    mainWindow = createMainWindow();

    app.on("browser-window-focus", () => pointer.focus());
    app.on("browser-window-blur", () => {
      // Moving between the app and its own sign-in window keeps the speed.
      setTimeout(() => {
        if (!BrowserWindow.getFocusedWindow()) pointer.blur();
      }, 60);
    });
    powerMonitor.on("lock-screen", () => pointer.blur());
    powerMonitor.on("suspend", () => pointer.blur());
  });

  app.on("before-quit", () => {
    browserSignIn.cancel();
    if (pointer) pointer.blur();
  });
  app.on("will-quit", () => pointer && pointer.blur());
  process.on("exit", () => pointer && pointer.blur());
  app.on("window-all-closed", () => app.quit());

  function secureWebPreferences(extra = {}) {
    return {
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      nodeIntegrationInSubFrames: false,
      webSecurity: true,
      allowRunningInsecureContent: false,
      spellcheck: false,
      ...extra,
    };
  }

  function openOutside(url) {
    if (!config.isExternalSafe(url)) return;
    shell.openExternal(url).catch((err) => log(`couldn't open ${url}: ${err.message}`));
  }

  function guardPermissions() {
    const ses = session.defaultSession;
    ses.setPermissionRequestHandler((contents, permission, callback, details) => {
      const from = (details && details.requestingUrl) || contents.getURL();
      const ok = config.permissionAllowed(permission, from);
      if (!ok) log(`refused ${permission} for ${from}`);
      callback(ok);
    });
    ses.setPermissionCheckHandler((_contents, permission, requestingOrigin) =>
      config.permissionAllowed(permission, requestingOrigin),
    );
  }

  /** Brings the app's window to the front, as far as Windows allows. */
  function bringToFront() {
    if (!mainWindow) return;
    if (mainWindow.isMinimized()) mainWindow.restore();
    mainWindow.show();
    mainWindow.setAlwaysOnTop(true);
    mainWindow.focus();
    mainWindow.setAlwaysOnTop(false);
    mainWindow.flashFrame(true);
    setTimeout(() => mainWindow && mainWindow.flashFrame(false), 3000);
  }

  /** What the app's page may ask: the pointer speed, to change it, and to sign in through the browser. */
  function answerTheApp() {
    const fromApp = (event) => config.isAppUrl(event.senderFrame ? event.senderFrame.url : "");
    ipcMain.handle("cadp:pointer-speed:get", (event) => {
      if (!fromApp(event)) throw new Error("not allowed");
      return pointer.status();
    });
    ipcMain.handle("cadp:pointer-speed:set", (event, value) => {
      if (!fromApp(event)) throw new Error("not allowed");
      return pointer.choose(value === undefined ? null : value);
    });
    ipcMain.handle("cadp:sign-in:browser", (event, publicKeyHex) => {
      if (!fromApp(event)) throw new Error("not allowed");
      return browserSignIn.start(publicKeyHex);
    });
    ipcMain.handle("cadp:sign-in:cancel", (event) => {
      if (!fromApp(event)) throw new Error("not allowed");
      browserSignIn.cancel();
    });
  }

  function createMainWindow() {
    const saved = store.get("window", null);
    const bounds = onScreen(saved && saved.bounds);
    const win = new BrowserWindow({
      ...(bounds || { width: 1440, height: 900 }),
      minWidth: 900,
      minHeight: 600,
      show: false,
      title: "CAD Protocol",
      backgroundColor: "#000000",
      webPreferences: secureWebPreferences({
        preload: path.join(__dirname, "preload.js"),
        additionalArguments: [`--cadp-desktop-version=${app.getVersion()}`, `--cadp-app-origin=${config.APP_ORIGIN}`],
      }),
    });
    kinds.set(win.webContents, "main");
    if (!saved || saved.maximized !== false) win.maximize();
    win.show();

    const contents = win.webContents;
    contents.setVisualZoomLevelLimits(1, 1); // trackpad pinch goes to the drawing, not the page

    contents.on("did-fail-load", (_event, code, description, url, isMainFrame) => {
      // -3 is a load the page itself cancelled; not a failure.
      if (!isMainFrame || code === -3 || !config.isAppUrl(url)) return;
      log(`couldn't load ${url}: ${code} ${description}`);
      win.loadFile(OFFLINE_FILE, { query: { reason: `${description} (${code})` } });
    });

    // The app asks before closing with unsaved drawings. A browser shows that
    // question itself; here we ask it.
    contents.on("will-prevent-unload", (event) => {
      const choice = dialog.showMessageBoxSync(win, {
        type: "warning",
        buttons: ["Leave", "Stay"],
        defaultId: 1,
        cancelId: 1,
        noLink: true,
        title: "CAD Protocol",
        message: "You have drawings with changes that aren't saved.",
        detail: "Leave anyway? The unsaved changes will be lost.",
      });
      if (choice === 0) event.preventDefault();
    });

    contents.on("render-process-gone", (_event, details) => {
      log(`page stopped: ${details.reason}`);
      if (details.reason !== "clean-exit") contents.reload();
    });

    win.on("close", () => {
      store.set("window", { maximized: win.isMaximized(), bounds: win.getNormalBounds() });
    });
    win.on("closed", () => {
      if (mainWindow === win) mainWindow = null;
    });

    win.loadURL(config.APP_URL);
    return win;
  }

  /** The saved size and place, if it still fits on one of the screens. */
  function onScreen(bounds) {
    if (!bounds || ![bounds.x, bounds.y, bounds.width, bounds.height].every(Number.isFinite)) return null;
    const visible = screen.getAllDisplays().some(({ workArea: a }) => {
      const w = Math.min(bounds.x + bounds.width, a.x + a.width) - Math.max(bounds.x, a.x);
      const h = Math.min(bounds.y + bounds.height, a.y + a.height) - Math.max(bounds.y, a.y);
      return w >= 200 && h >= 120;
    });
    return visible ? bounds : null;
  }
}
