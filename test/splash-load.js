"use strict";
// Temporary (splash-check branch): the card (as 1.8.1) with the app's own
// window loading out of sight, as at start-up. CADP_MODE:
//   now   - the app's page loads at once (1.7 and 1.8.1)
//   blank - the app's window loads nothing
//   after - the app's page loads once the card is on screen and drawn
const path = require("node:path");
const { app, BrowserWindow } = require("electron");
const { createTitleBar, windowsBackend } = require("../src/title-bar");
const { createSplash } = require("../src/splash");

const mode = process.env.CADP_MODE || "now";
const t0 = Date.now();
const say = (m) => console.log(`${Date.now() - t0}ms ${m}`);
const prefs = (extra = {}) => ({
  contextIsolation: true,
  sandbox: true,
  nodeIntegration: false,
  nodeIntegrationInSubFrames: false,
  webSecurity: true,
  allowRunningInsecureContent: false,
  spellcheck: false,
  ...extra,
});

app.whenReady().then(() => {
  const titleBar = createTitleBar({ backend: windowsBackend(say) });
  app.on("browser-window-created", (_e, win) => titleBar.paint(win));
  const splash = createSplash({
    BrowserWindow,
    file: path.join(__dirname, "..", "src", "update.html"),
    webPreferences: prefs(),
  });
  splash.open();
  splash.say(`Starting (${mode})`, "CAD Protocol for Windows");
  const main = new BrowserWindow({
    width: 1440,
    height: 900,
    show: false,
    backgroundColor: "#000000",
    webPreferences: prefs({
      preload: path.join(__dirname, "..", "src", "preload.js"),
      additionalArguments: ["--cadp-desktop-version=1.8.1", "--cadp-app-origin=https://cad-dza.caffeine.xyz"],
    }),
  });
  main.webContents.on("did-finish-load", () => say("app page loaded"));
  const load = () => {
    say("app page loading");
    main.loadURL(mode === "blank" ? "about:blank" : "https://cad-dza.caffeine.xyz/");
  };
  if (mode === "after") {
    const wait = setInterval(() => {
      if (!splash.isShown) return;
      clearInterval(wait);
      say("card on screen");
      setTimeout(load, 300);
    }, 20);
  } else load();
  const shownCheck = setInterval(() => {
    if (splash.isShown) {
      say("card shown");
      clearInterval(shownCheck);
    }
  }, 20);
  setTimeout(() => app.quit(), 7000);
});
