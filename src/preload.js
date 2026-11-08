"use strict";
// Runs before the app's page in each window, in its own sandboxed world.
// It gives the CAD Protocol app, and only the app, a small window.cadpDesktop
// object: that it is running in the desktop app, its version, the pointer
// speed setting, and Sign in with your browser. Nothing else of the computer
// is reachable.

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
  });
}
