"use strict";
// Runs before the app's page in each window, in its own sandboxed world.
// It gives the CAD Protocol app, and only the app, a small window.cadpDesktop
// object: that it is running in the desktop app, its version, the pointer
// speed setting, Sign in with your browser, its update state and a restart
// into a downloaded update, a say before the window closes, and drawings
// saved to and opened from files the person picks, and (1.19) the scan
// engine for scan files the person chose. Nothing else of the computer is
// reachable.

const { contextBridge, ipcRenderer, webUtils } = require("electron");

function arg(name) {
  const found = process.argv.find((a) => a.startsWith(`--${name}=`));
  return found ? found.slice(name.length + 3) : "";
}

const appOrigin = arg("cadp-app-origin");
/** Cuts asked of the scan engine (1.28), each told its own progress. */
let cutsAsked = 0;

if (appOrigin && window.location.origin === appOrigin) {
  contextBridge.exposeInMainWorld("cadpDesktop", {
    isDesktop: true,
    version: arg("cadp-desktop-version"),
    platform: process.platform,
    pointerSpeed: {
      /** { supported, chosen, automatic, off, own, inUse, defaultSpeed, min, max } */
      get: () => ipcRenderer.invoke("cadp:pointer-speed:get"),
      /** null for automatic (at least the default), 0 for off, or 1..20. */
      set: (value) => ipcRenderer.invoke("cadp:pointer-speed:set", value),
    },
    signIn: {
      /** Opens the app in the person's browser to sign in; resolves with { chain, principal }. */
      withBrowser: (publicKeyHex) => ipcRenderer.invoke("cadp:sign-in:browser", publicKeyHex),
      cancel: () => ipcRenderer.invoke("cadp:sign-in:cancel"),
    },
    /**
     * 1.10: signed out, the desktop's sign-in card instead of the app's
     * page: `signIn` shows it, `progress` says how far signing in has got,
     * `onChoice` hears its buttons ("browser", "offline", "cancel").
     * 1.14: `signIn({ window: true })` says the page can sign in in a
     * window of the app's own; the card then has that button ("window").
     */
    card: {
      signIn: (can) => ipcRenderer.invoke("cadp:card:sign-in", can),
      progress: (state) => ipcRenderer.invoke("cadp:card:progress", state),
      onChoice: (fn) => {
        const heard = (_event, kind) => fn(kind);
        ipcRenderer.on("cadp:card:choice", heard);
        return () => ipcRenderer.removeListener("cadp:card:choice", heard);
      },
    },
    /**
     * 1.9: Google Drive through the person's browser; resolves with
     * { accessToken, expiresIn, email }. 1.10.2: with { pick: { kind,
     * apiKey, title, startIn } } the browser shows Google's picker too, and
     * what's chosen comes back as `picked` (canPick says so).
     */
    drive: {
      connect: (clientId, options) => ipcRenderer.invoke("cadp:drive:connect", clientId, options),
      cancel: () => ipcRenderer.invoke("cadp:drive:cancel"),
      canPick: true,
    },
    /**
     * 1.3: { current, latest, newer, url, phase, percent }, phase being
     * checking, up-to-date, downloading, ready, error or off; restart() puts
     * a downloaded update in now. 1.26: check() looks now (resolving with
     * the state); onChange(fn) hears each change as it comes (the
     * download's percent too), and returns how to stop hearing it.
     */
    update: {
      get: () => ipcRenderer.invoke("cadp:update:get"),
      restart: () => ipcRenderer.invoke("cadp:update:restart"),
      check: () => ipcRenderer.invoke("cadp:update:check"),
      onChange: (fn) => {
        if (typeof fn !== "function") return () => {};
        const heard = (_event, state) => fn(state);
        ipcRenderer.on("cadp:update:state", heard);
        return () => ipcRenderer.removeListener("cadp:update:state", heard);
      },
    },
    /** 1.7: the app's first screen is up, so its window shows (the start-up card goes). */
    ready: () => ipcRenderer.send("cadp:ready"),
    /**
     * 1.5: drawings on this computer, through Windows' Save and Open windows.
     * saveAs(name, bytes) and open() resolve with { token, name, path } (open
     * adds bytes), or null when cancelled; save(token, bytes) writes the same
     * file again, or resolves null when the token is no longer known.
     * 1.11: .cadp files double-clicked in Windows wait for the page:
     * launchedCount() says how many, takeLaunched() hands them over once
     * each ({ token, name, path, bytes }, or { name, path, error }), and
     * onLaunched(fn) hears when another arrives while the app's open.
     * 1.12: saveBeside(token, bytes) writes that drawing's DWG copy beside
     * it (same name, .dwg): { name, path }, or null when the token's gone.
     */
    files: {
      saveAs: (name, bytes) => ipcRenderer.invoke("cadp:files:save-as", name, bytes),
      save: (token, bytes) => ipcRenderer.invoke("cadp:files:save", token, bytes),
      saveBeside: (token, bytes) => ipcRenderer.invoke("cadp:files:save-beside", token, bytes),
      open: () => ipcRenderer.invoke("cadp:files:open"),
      launchedCount: () => ipcRenderer.invoke("cadp:files:launched-count"),
      takeLaunched: () => ipcRenderer.invoke("cadp:files:take-launched"),
      onLaunched: (fn) => {
        const heard = (_event, count) => fn(count);
        ipcRenderer.on("cadp:files:launched", heard);
        return () => ipcRenderer.removeListener("cadp:files:launched", heard);
      },
    },
    /**
     * 1.27: files remembered by their place on this computer (src/places.js).
     * add(files) keeps File objects the page has (picked or dropped there),
     * by their paths, read here; pick({ kind, multiple, title }) shows
     * Windows' Open window; stat(ids) says whether each is still there;
     * fill(marker, ids) puts the files into the page's
     * <input type=file data-cadp-fill=marker>, as a drop would; find(id)
     * looks for a moved one; ofToken(token) keeps a drawing opened or saved
     * here; openDrawing(id) opens a kept drawing as Open does; reveal(id)
     * shows it in File Explorer; openOutside(id) opens a PDF, picture,
     * sheet or document in its own program; saveAs({ name, beside, kind },
     * bytes) saves a file the page made (a cut scan) in Windows' Save
     * window, beside a place, and keeps it.
     */
    places: {
      add: (list) => {
        const paths = [];
        for (const f of Array.isArray(list) ? list : []) {
          try {
            const where = webUtils.getPathForFile(f);
            if (where) paths.push(where);
          } catch {
            // Not a file from this computer.
          }
        }
        return paths.length ? ipcRenderer.invoke("cadp:places:add", paths) : Promise.resolve([]);
      },
      pick: (options) => ipcRenderer.invoke("cadp:places:pick", options),
      stat: (ids) => ipcRenderer.invoke("cadp:places:stat", ids),
      fill: (marker, ids) => ipcRenderer.invoke("cadp:places:fill", marker, ids),
      find: (id) => ipcRenderer.invoke("cadp:places:find", id),
      forget: (ids) => ipcRenderer.invoke("cadp:places:forget", ids),
      ofToken: (token) => ipcRenderer.invoke("cadp:places:of-token", token),
      openDrawing: (id) => ipcRenderer.invoke("cadp:places:open-drawing", id),
      saveAs: (options, bytes) => ipcRenderer.invoke("cadp:places:save-as", options, bytes),
      reveal: (id) => ipcRenderer.invoke("cadp:places:reveal", id),
      openOutside: (id) => ipcRenderer.invoke("cadp:places:open-outside", id),
    },
    /** 1.27: the jobs (JSON text), kept in this computer's app data. */
    jobs: {
      load: () => ipcRenderer.invoke("cadp:jobs:load"),
      save: (text) => ipcRenderer.invoke("cadp:jobs:save", text),
    },
    /** 1.31: the drawing area's stalls (text lines) added to stalls.log in this computer's app data. */
    stalls: {
      append: (text) => ipcRenderer.invoke("cadp:stalls:append", text),
    },
    /**
     * 1.28: Drawing Recovery's copies kept as files on this computer
     * (src/recovery.js) in place of the browser engine's storage: put(copy),
     * list(), remove(id), as the page's own store has them.
     */
    recovery: {
      put: (copy) => ipcRenderer.invoke("cadp:recovery:put", copy),
      list: () => ipcRenderer.invoke("cadp:recovery:list"),
      remove: (id) => ipcRenderer.invoke("cadp:recovery:remove", id),
    },
    /**
     * 1.19: the scan engine (src/scan-engine.js), for LAS, LAZ and E57 scans
     * read from this computer. register(file) hands it a scan file the
     * person chose (a File from an Open window, a drop or a linked folder):
     * { token, name, size }, or null when it isn't one it reads. connect()
     * asks for a port to it: the port comes to the page as a message
     * ({ cadpEngine: "port" }, its ports[0]) from this window's own origin.
     * 1.25: when the engine's process stops, { cadpEngine: "lost", again,
     * dropped } (whether it starts again; the tokens let go), and once it's
     * started again { cadpEngine: "back" }, so the page asks for new ports.
     */
    engine: {
      register: (file) => {
        let where = "";
        try {
          where = webUtils.getPathForFile(file);
        } catch {
          where = "";
        }
        if (!where) return Promise.resolve(null);
        return ipcRenderer.invoke("cadp:engine:register", where);
      },
      connect: () => ipcRenderer.send("cadp:engine:connect"),
      /**
       * 1.28: a scan the engine has open (its token) cut to the drawing's
       * limit boxes from its index, every point, saved where the person
       * chooses in Windows' Save window (beside the place `beside`):
       * { place, kept, processed, min, max, bytes }, or null if cancelled.
       * `progress` is told how far it's come, 0 to 1.
       */
      crop: (options, progress) => {
        const id = (cutsAsked += 1);
        const heard = (_event, m) => {
          if (m && m.id === id && typeof progress === "function") progress(Number(m.done) || 0);
        };
        ipcRenderer.on("cadp:engine:crop-progress", heard);
        const o = options && typeof options === "object" ? options : {};
        return ipcRenderer
          .invoke("cadp:engine:crop", { id, token: o.token, name: o.name, beside: o.beside, ask: o.ask })
          .finally(() => ipcRenderer.removeListener("cadp:engine:crop-progress", heard));
      },
    },
    /**
     * 1.2: `handler` runs when the window is about to close (it may save and
     * sign out); the window closes when its promise settles, or after a minute.
     * A new handler replaces the last.
     */
    onClosing: (handler) => {
      ipcRenderer.removeAllListeners("cadp:closing");
      ipcRenderer.on("cadp:closing", async (_event, id) => {
        ipcRenderer.send("cadp:close-started", id);
        try {
          await handler();
        } catch {
          // Closing goes ahead whatever the page did.
        }
        ipcRenderer.send("cadp:close-done", id);
      });
    },
    /**
     * 1.18: as onClosing, but the page may ask the person first.
     * `handler({ asking, working })` resolves "close" or "stay": asking()
     * while the person answers (the window stays, shown, with no time
     * limit), working() while it saves and signs out (the small card, a
     * minute at most); "stay" keeps the window open. A new handler
     * replaces the last (onClosing's too).
     */
    closeAsks: true,
    onCloseAsk: (handler) => {
      ipcRenderer.removeAllListeners("cadp:closing");
      ipcRenderer.on("cadp:closing", async (_event, id) => {
        let answer = "close";
        try {
          answer = await handler({
            asking: () => ipcRenderer.send("cadp:close-asking", id),
            working: () => ipcRenderer.send("cadp:close-started", id),
          });
        } catch {
          // Closing goes ahead whatever the page did.
        }
        ipcRenderer.send(answer === "stay" ? "cadp:close-cancelled" : "cadp:close-done", id);
      });
    },
  });
  // 1.19: the engine's port handed on to the page (a port can't cross the bridge).
  ipcRenderer.on("cadp:engine:port", (event) => {
    window.postMessage({ cadpEngine: "port" }, window.location.origin, event.ports);
  });
  // 1.25: the engine stopped (and whether it starts again), then started again.
  ipcRenderer.on("cadp:engine:lost", (_event, info) => {
    const dropped = info && Array.isArray(info.dropped) ? info.dropped.filter((t) => typeof t === "string") : [];
    window.postMessage({ cadpEngine: "lost", again: Boolean(info && info.again), dropped }, window.location.origin);
  });
  ipcRenderer.on("cadp:engine:back", () => {
    window.postMessage({ cadpEngine: "back" }, window.location.origin);
  });
}
