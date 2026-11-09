"use strict";
// The small card window (1.7): CAD Protocol's start-up card on its own, no
// frame, centred, while the app's window loads out of sight, while an
// update downloads and installs, and while the app saves and signs out as
// it closes. update.html drawn in its "splash" form (the card is the whole
// window). Only words go to it, through `showUpdate`.

const WIDTH = 560;
const HEIGHT = 300;

function createSplash({ BrowserWindow, file, webPreferences, onClosedByPerson = () => {} }) {
  let win = null;
  let last = null;
  let closingOnPurpose = false;

  function apply() {
    if (!win || win.isDestroyed() || !last) return;
    const [title, detail, percent, note] = last;
    win.webContents
      .executeJavaScript(
        `window.showUpdate && window.showUpdate(${JSON.stringify(title)}, ${JSON.stringify(detail)}, ${percent === null ? "null" : percent}, ${JSON.stringify(note)})`,
      )
      .catch(() => undefined);
  }

  return {
    /** The card, shown as soon as it's drawn. */
    open() {
      if (win && !win.isDestroyed()) {
        win.show();
        return;
      }
      closingOnPurpose = false;
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
        webPreferences,
      });
      const mine = win;
      mine.once("ready-to-show", () => {
        if (!mine.isDestroyed()) mine.show();
      });
      mine.webContents.on("did-finish-load", apply);
      mine.on("closed", () => {
        if (win === mine) win = null;
        if (!closingOnPurpose) onClosedByPerson();
      });
      mine.loadFile(file, { query: { splash: "1" } }).catch(() => undefined);
    },

    /** What it says: a title, a line under it, a percentage (null: moving), a note. */
    say(title, detail = "", percent = null, note = "") {
      last = [title, detail, percent, note];
      apply();
    },

    close() {
      if (!win || win.isDestroyed()) return;
      closingOnPurpose = true;
      win.destroy();
      win = null;
    },

    focus() {
      if (win && !win.isDestroyed()) win.focus();
    },

    get isOpen() {
      return Boolean(win && !win.isDestroyed());
    },
  };
}

module.exports = { createSplash, WIDTH, HEIGHT };
