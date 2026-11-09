"use strict";
// Runs before the app's page in each window, in its own sandboxed world.
// It gives the CAD Protocol app, and only the app, a small window.cadpDesktop
// object: that it is running in the desktop app, its version, the pointer
// speed setting, Sign in with your browser, its update state and a restart
// into a downloaded update, a say before the window closes, and drawings
// saved to and opened from files the person picks. Nothing else of the
// computer is reachable.

const { contextBridge, ipcRenderer } = require("electron");

function arg(name) {
  const found = process.argv.find((a) => a.startsWith(`--${name}=`));
  return found ? found.slice(name.length + 3) : "";
}

const appOrigin = arg("cadp-app-origin");

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
     * 1.3: { current, latest, newer, url, phase, percent }, phase being
     * checking, up-to-date, downloading, ready, error or off; restart() puts
     * a downloaded update in now.
     */
    update: {
      get: () => ipcRenderer.invoke("cadp:update:get"),
      restart: () => ipcRenderer.invoke("cadp:update:restart"),
    },
    /** 1.7: the app's first screen is up, so its window shows (the start-up card goes). */
    ready: () => ipcRenderer.send("cadp:ready"),
    /**
     * 1.5: drawings on this computer, through Windows' Save and Open windows.
     * saveAs(name, bytes) and open() resolve with { token, name, path } (open
     * adds bytes), or null when cancelled; save(token, bytes) writes the same
     * file again, or resolves null when the token is no longer known.
     */
    files: {
      saveAs: (name, bytes) => ipcRenderer.invoke("cadp:files:save-as", name, bytes),
      save: (token, bytes) => ipcRenderer.invoke("cadp:files:save", token, bytes),
      open: () => ipcRenderer.invoke("cadp:files:open"),
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
  });
}
