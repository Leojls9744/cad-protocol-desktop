"use strict";
// The sign-in card's one bridge (1.10): its buttons tell the desktop app
// what was chosen (signin-card.html). Nothing else is exposed to it.
const { contextBridge, ipcRenderer } = require("electron");

const KINDS = new Set(["browser", "offline", "cancel", "quit"]);

contextBridge.exposeInMainWorld("cadpCard", {
  choose(kind) {
    if (KINDS.has(kind)) ipcRenderer.send("cadp:card:choose", kind);
  },
});
