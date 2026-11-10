"use strict";
// Temporary (splash-check branch): the card window in several variants side
// by side, each with one option changed, to see which one Windows leaves blank.
const path = require("node:path");
const { app, BrowserWindow } = require("electron");
const { createTitleBar, windowsBackend } = require("../src/title-bar");

const FILE = path.join(__dirname, "..", "src", "update.html");
const W = 320;
const H = 170;
const base = {
  width: W,
  height: H,
  useContentSize: true,
  frame: false,
  resizable: false,
  maximizable: false,
  minimizable: true,
  fullscreenable: false,
  show: false,
  backgroundColor: "#0b0b0f",
  webPreferences: { contextIsolation: true, sandbox: true, nodeIntegration: false },
};
const variants = [
  ["1 as 1.8.1", {}, { paint: true }],
  ["2 no purple bar", {}, {}],
  ["3 frame: true", { frame: true }, { paint: true }],
  ["4 resizable", { resizable: true }, { paint: true }],
  ["5 no drag", {}, { paint: true, noDrag: true }],
  ["6 thickFrame off", { thickFrame: false }, { paint: true }],
  ["7 shown at once", { show: true }, { paint: true }],
  ["8 invalidate", {}, { paint: true, invalidate: true }],
  ["9 bar, drag, no-throttle", { webPreferences: { ...base.webPreferences, backgroundThrottling: false } }, { paint: true }],
];

app.whenReady().then(() => {
  const titleBar = createTitleBar({ backend: windowsBackend(console.log) });
  variants.forEach(([name, options, how], i) => {
    const x = 20 + (i % 3) * (W + 20);
    const y = 20 + Math.floor(i / 3) * (H + 50);
    const win = new BrowserWindow({ ...base, ...options, x, y });
    if (how.paint) titleBar.paint(win);
    win.webContents.on("did-finish-load", async () => {
      if (how.noDrag) await win.webContents.insertCSS("body.splash .card { -webkit-app-region: no-drag !important; }");
      await win.webContents.executeJavaScript(`window.showUpdate(${JSON.stringify(name)}, "variant", null, "")`);
      setTimeout(() => {
        if (!win.isVisible()) win.show();
        if (how.invalidate) {
          win.webContents.invalidate();
          setTimeout(() => win.webContents.invalidate(), 100);
        }
        console.log(`${name}: shown`);
      }, 400);
    });
    win.loadFile(FILE, { query: { splash: "1" } });
  });
  setTimeout(() => app.quit(), 9000);
});
