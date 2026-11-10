"use strict";
// The small card window (1.7): CAD Protocol's start-up card on its own, no
// frame, centred, while the app's window loads out of sight, while an
// update downloads and installs, and while the app saves and signs out as
// it closes. update.html drawn in its "splash" form (the card is the whole
// window). Only words go to it, through `showUpdate`.
//
// 1.8.1 (Leo, 2026-11-09 15:05: "the desktop startup loading and signing
// out card are blank ... the open and close before the image can
// generate"): the card shows only once its picture is decoded and drawn
// (or PAINT_WAIT_MS has passed, whatever else), and once shown it stays
// at least MIN_SHOWN_MS, so it's never a blank flash. Closing early waits
// for that (`close`, `after`). Signing out, it's made out of sight at once
// (`prepare`) and shown only if the sign-out is still going a moment later.
//
// 1.8.2: the card's page comes from cadp://app/ (local-pages.js), not
// file://, which the installed app can't show (it was an empty box).

const WIDTH = 560;
const HEIGHT = 300;
/** Once shown, the card stays at least this long, ms. */
const MIN_SHOWN_MS = 1200;
/** Shown after this long even if its picture hasn't said it's drawn, ms. */
const PAINT_WAIT_MS = 1500;

/** Run in the card: resolves once its picture is loaded and decoded, a moment after. */
const PAINTED_JS = `new Promise((resolve) => {
  const done = () => setTimeout(resolve, 60);
  const img = new Image();
  img.onload = () => (img.decode ? img.decode().then(done, done) : done());
  img.onerror = done;
  img.src = "splash.jpg";
})`;

function createSplash({
  BrowserWindow,
  url,
  webPreferences,
  onClosedByPerson = () => {},
  minShownMs = MIN_SHOWN_MS,
  paintWaitMs = PAINT_WAIT_MS,
  now = Date.now,
  setTimer = setTimeout,
  clearTimer = clearTimeout,
}) {
  let win = null;
  let last = null;
  let closingOnPurpose = false;
  /** Its picture's drawn (or it's waited long enough). */
  let painted = false;
  /** It should be on screen once it's drawn. */
  let wanted = false;
  /** When it went on screen (0: it isn't). */
  let shownAt = 0;
  let paintTimer = null;
  let closeTimer = null;

  function apply() {
    if (!win || win.isDestroyed() || !last) return;
    const [title, detail, percent, note] = last;
    win.webContents
      .executeJavaScript(
        `window.showUpdate && window.showUpdate(${JSON.stringify(title)}, ${JSON.stringify(detail)}, ${percent === null ? "null" : percent}, ${JSON.stringify(note)})`,
      )
      .catch(() => undefined);
  }

  function showIfReady() {
    if (!win || win.isDestroyed() || !wanted || !painted || shownAt) return;
    win.show();
    shownAt = now();
  }

  function drawn(mine) {
    if (win !== mine || mine.isDestroyed() || painted) return;
    painted = true;
    if (paintTimer) clearTimer(paintTimer);
    paintTimer = null;
    showIfReady();
  }

  function make() {
    if (win && !win.isDestroyed()) return;
    closingOnPurpose = false;
    painted = false;
    shownAt = 0;
    win = new BrowserWindow({
      width: WIDTH,
      height: HEIGHT,
      useContentSize: true,
      frame: false,
      resizable: false,
      maximizable: false,
      minimizable: true,
      fullscreenable: false,
      show: false,
      center: true,
      title: "CAD Protocol",
      backgroundColor: "#0b0b0f",
      // Drawn while it's still out of sight.
      webPreferences: { ...webPreferences, backgroundThrottling: false },
    });
    const mine = win;
    mine.webContents.on("did-finish-load", () => {
      apply();
      Promise.resolve(mine.webContents.executeJavaScript(PAINTED_JS))
        .catch(() => undefined)
        .then(() => drawn(mine));
    });
    paintTimer = setTimer(() => drawn(mine), paintWaitMs);
    mine.on("closed", () => {
      if (win === mine) {
        win = null;
        wanted = false;
        shownAt = 0;
      }
      if (!closingOnPurpose) onClosedByPerson();
    });
    mine.loadURL(url).catch(() => undefined);
  }

  function destroy() {
    if (closeTimer) clearTimer(closeTimer);
    if (paintTimer) clearTimer(paintTimer);
    closeTimer = null;
    paintTimer = null;
    wanted = false;
    shownAt = 0;
    if (!win || win.isDestroyed()) {
      win = null;
      return;
    }
    closingOnPurpose = true;
    win.destroy();
    win = null;
  }

  /** How much longer it has to stay up, ms (0: it can go now). */
  function left() {
    if (!win || win.isDestroyed() || !shownAt) return 0;
    return Math.max(0, minShownMs - (now() - shownAt));
  }

  return {
    /** The card made and drawn out of sight (shown by `open`). */
    prepare() {
      make();
    },

    /** The card, shown as soon as its picture's drawn. */
    open() {
      if (closeTimer) clearTimer(closeTimer);
      closeTimer = null;
      make();
      wanted = true;
      showIfReady();
      if (shownAt) win.show();
    },

    /** What it says: a title, a line under it, a percentage (null: moving), a note. */
    say(title, detail = "", percent = null, note = "") {
      last = [title, detail, percent, note];
      apply();
    },

    /** Closed: at once if it isn't showing, else once it's been up long enough. */
    close() {
      if (!win || win.isDestroyed()) return;
      const wait = left();
      if (!wait) {
        destroy();
        return;
      }
      wanted = false;
      if (!closeTimer)
        closeTimer = setTimer(() => {
          closeTimer = null;
          destroy();
        }, wait);
    },

    /** `fn` once the card's been up long enough (at once if it isn't showing). */
    after(fn) {
      const wait = left();
      if (wait) setTimer(fn, wait);
      else fn();
    },

    focus() {
      if (win && !win.isDestroyed()) win.focus();
    },

    get isOpen() {
      return Boolean(win && !win.isDestroyed());
    },

    /** It's on screen. */
    get isShown() {
      return Boolean(win && !win.isDestroyed() && shownAt);
    },
  };
}

module.exports = { createSplash, WIDTH, HEIGHT, MIN_SHOWN_MS, PAINT_WAIT_MS, PAINTED_JS };
