"use strict";
// The sign-in card (1.10; Leo, 2026-11-10 14:28): when the app's signed
// out, a card of its own (four times the start-up card) instead of the
// app's full sign-in page. Its buttons go to the app's page, which signs
// in through the browser (browser-sign-in.js) as before and says how far
// it's got; the card shows that, and goes when the app's window shows.

const WIDTH = 1120;
const HEIGHT = 600;
/** How much of the screen it may take at most. */
const MAX_SHARE = 0.9;
/** Shown by then even if its picture's slow. */
const PAINT_WAIT_MS = 1500;

/** Its size on a work area: four times the start-up card, never more than 90% of the screen. */
function cardSize(workArea) {
  const width = Math.min(WIDTH, Math.floor(workArea.width * MAX_SHARE));
  const height = Math.min(HEIGHT, Math.floor(workArea.height * MAX_SHARE));
  return { width: Math.max(560, width), height: Math.max(360, height) };
}

const PHASES = new Set(["ready", "waiting", "finishing", "opening", "error", "offline"]);

/** What the app's page may say: a phase, its words and a note, kept short. */
function cleanState(state) {
  const s = state && typeof state === "object" ? state : {};
  const text = (v) => (typeof v === "string" ? v.slice(0, 300) : "");
  return { phase: PHASES.has(s.phase) ? s.phase : "ready", text: text(s.text), note: text(s.note) };
}

function createSignInCard({ BrowserWindow, workArea, url, preload, webPreferences, log = () => {} }) {
  let win = null;
  let shown = false;
  let last = { phase: "ready", text: "", note: "" };

  const apply = () => {
    if (!win || win.isDestroyed()) return;
    win.webContents
      .executeJavaScript(`window.cardState && window.cardState(${JSON.stringify(last)})`)
      .catch(() => undefined);
  };

  return {
    get isShown() {
      return Boolean(win && !win.isDestroyed() && shown);
    },
    get webContents() {
      return win && !win.isDestroyed() ? win.webContents : null;
    },
    /** Open it (once); `then` when it's on screen (the start-up card goes then). */
    open(then = () => {}) {
      if (win && !win.isDestroyed()) {
        win.show();
        win.focus();
        then();
        return;
      }
      const size = cardSize(workArea());
      win = new BrowserWindow({
        ...size,
        frame: false,
        resizable: false,
        maximizable: false,
        minimizable: true,
        fullscreenable: false,
        show: false,
        center: true,
        title: "Sign in to CAD Protocol",
        backgroundColor: "#000000",
        webPreferences: { ...webPreferences, preload },
      });
      shown = false;
      const reveal = () => {
        if (!win || win.isDestroyed() || shown) return;
        shown = true;
        clearTimeout(timer);
        win.show();
        win.focus();
        then();
      };
      const timer = setTimeout(reveal, PAINT_WAIT_MS);
      win.once("ready-to-show", () => {
        apply();
        reveal();
      });
      win.webContents.on("did-finish-load", apply);
      win.on("closed", () => {
        clearTimeout(timer);
        win = null;
        shown = false;
      });
      win.loadURL(url).catch((err) => log(`sign-in card: ${err && err.message ? err.message : err}`));
    },
    /** What's happening, from the app's page. */
    say(state) {
      last = cleanState(state);
      apply();
    },
    focus() {
      if (!win || win.isDestroyed()) return;
      if (win.isMinimized()) win.restore();
      win.show();
      win.focus();
    },
    close() {
      if (win && !win.isDestroyed()) win.close();
      win = null;
      shown = false;
      last = { phase: "ready", text: "", note: "" };
    },
  };
}

module.exports = { createSignInCard, cardSize, cleanState, WIDTH, HEIGHT };
