"use strict";
// The app's start watched (1.32; Leo, 2026-11-19 17:31: "when you run an
// update and i start the app, it get stuck in the loading screen, you need
// to close down the window and open the app again"). His log: after each
// web update the app was started twice, 20 to 70 seconds apart. Once the
// app's page starts loading, it has READY_MS to say it's up (its first
// screen: cadp:ready, or the sign-in card it asks for when signed out).
// If it doesn't, the saved copy of the app's code (its service worker and
// the files it keeps; never drawings, jobs, settings or sign-in) is cleared
// and the page loaded again from the internet, on the card. Once a start.
//
// Plain data and timers, so it's tested without Electron.

/** The page has this long to say it's up, from when it starts loading, ms. */
const READY_MS = 30000;

/**
 * `stuck()` runs when the page hasn't said it's up in time (once);
 * `alive()` is called when it has.
 */
function createStartWatch({ stuck, readyMs = READY_MS, setTimer = setTimeout, clearTimer = clearTimeout }) {
  let timer = null;
  let up = false;
  let fired = false;
  let began = null;
  return {
    /** The app's page started loading. */
    begin(now = Date.now()) {
      if (up || fired || timer) return;
      began = now;
      timer = setTimer(() => {
        timer = null;
        if (up || fired) return;
        fired = true;
        stuck();
      }, readyMs);
    },
    /** The page said it's up. How long it took (ms), or null if it was already. */
    alive(now = Date.now()) {
      if (up) return null;
      up = true;
      if (timer) clearTimer(timer);
      timer = null;
      return began === null ? null : now - began;
    },
    get fired() {
      return fired;
    },
    get up() {
      return up;
    },
  };
}

module.exports = { createStartWatch, READY_MS };
