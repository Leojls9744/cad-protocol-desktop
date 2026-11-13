"use strict";
// Internet Identity in a window of the app's own (1.14; Leo, 2026-11-13
// 10:09, of another app of his: "the way the other app has you sign in to
// the desktop app is way better and shorter it has a chrome window with the
// app icon on every window, it jumps right to the internet identity page").
//
// The card's Sign in has the app's page start its ordinary sign-in, the one
// a browser runs. Its window.open of Internet Identity is let through as a
// window of this app (config.openKind: "signin"), and this says what that
// window is: Internet Identity's size, over the sign-in card (or the app's
// window), owned by it so it stays in front of it, with the app's icon and
// purple bar like every window here. Internet Identity may hand over to a
// sign-in provider (Google) in a further window: that opens over the first,
// the same way.
//
// Nothing's handed between programs this way: Internet Identity answers
// the app's page through the window's own link to it, as in a browser, and
// the app remembers the identity for next time. The person's browser is
// still there (browser-sign-in.js) as the card's second choice, for a
// sign-in an app's window can't do: Google has refused them, and a passkey
// kept by a browser's extension lives in that browser.

/** Internet Identity's window. */
const WIDTH = 520;
const HEIGHT = 760;
/** A provider's window, opened from Internet Identity's. */
const NEXT_WIDTH = 520;
const NEXT_HEIGHT = 700;

/** A rectangle of `size` centred on `anchor`, kept inside `workArea` (made smaller only if the screen is). */
function overBounds(anchor, workArea, size = { width: WIDTH, height: HEIGHT }) {
  const width = Math.min(size.width, workArea.width);
  const height = Math.min(size.height, workArea.height);
  const within = (value, low, high) => Math.max(low, Math.min(value, high));
  return {
    x: within(Math.round(anchor.x + (anchor.width - width) / 2), workArea.x, workArea.x + workArea.width - width),
    y: within(Math.round(anchor.y + (anchor.height - height) / 2), workArea.y, workArea.y + workArea.height - height),
    width,
    height,
  };
}

/**
 * What a sign-in window is made with.
 * - `anchor`: what it opens over (the card's or a window's bounds), on the
 *   screen whose `workArea` is given;
 * - `parent`: the window it stays in front of, if any;
 * - `next`: opened from a sign-in window (a provider's page), not the app;
 * - `dark`: Windows is in dark mode (Internet Identity's page is dark then,
 *   so the window is while it loads).
 */
function signInWindowOptions({ anchor, workArea, parent = null, next = false, dark = false, webPreferences = {} }) {
  const size = next ? { width: NEXT_WIDTH, height: NEXT_HEIGHT } : { width: WIDTH, height: HEIGHT };
  return {
    ...overBounds(anchor, workArea, size),
    title: next ? "Sign in" : "Internet Identity",
    show: true,
    autoHideMenuBar: true,
    minimizable: false,
    fullscreenable: false,
    backgroundColor: dark && !next ? "#121212" : "#ffffff",
    ...(parent ? { parent } : {}),
    webPreferences,
  };
}

/**
 * The app's sign-in windows that are open: Internet Identity's and any it
 * opened. `closeAll` is Cancel on the card.
 */
function createSignInWindows() {
  const open = new Set();
  return {
    get count() {
      return open.size;
    },
    add(win) {
      if (!win || open.has(win)) return;
      open.add(win);
      win.once("closed", () => open.delete(win));
    },
    closeAll() {
      for (const win of [...open]) {
        open.delete(win);
        if (!win.isDestroyed()) win.close();
      }
    },
  };
}

module.exports = { WIDTH, HEIGHT, NEXT_WIDTH, NEXT_HEIGHT, overBounds, signInWindowOptions, createSignInWindows };
