"use strict";
// Starting up behind the small card (1.7; Leo, 2026-11-09 09:08: "i only
// want to see the small progress card, but it still opens the entire
// window"). The app's window stays hidden while the card (splash.js) says
// what's happening; it's shown, and the card closed, once:
// - the app says it's ready (its first screen is up: `cadp:ready`), or
// - the page has loaded and FALLBACK_MS has passed (an app from before it
//   said so), or
// - the page couldn't load (the offline page shows), or
// - MAX_MS has passed since it started loading, whatever else.
// While an update's being looked for or installed (`hold`), the app loads
// behind the card but isn't shown; `release` shows it if it's ready by
// then (or as soon as it is).
// Plain data and timers, so it's tested without Electron.

const FALLBACK_MS = 4000;
const MAX_MS = 25000;

function createLaunch({
  showMain,
  closeSplash,
  fallbackMs = FALLBACK_MS,
  maxMs = MAX_MS,
  setTimer = setTimeout,
  clearTimer = clearTimeout,
}) {
  let shown = false;
  let fallback = null;
  let max = null;
  let held = false;
  let wanted = null;

  function reveal(why) {
    if (shown) return;
    if (held) {
      if (!wanted) wanted = why;
      return;
    }
    shown = true;
    if (fallback) clearTimer(fallback);
    if (max) clearTimer(max);
    fallback = null;
    max = null;
    showMain(why);
    closeSplash();
  }

  return {
    /** The app's page started loading. */
    loading() {
      if (shown || max) return;
      max = setTimer(() => reveal("slow"), maxMs);
    },
    /** The page loaded (the app's start-up may still be going on). */
    loaded() {
      if (shown || fallback) return;
      fallback = setTimer(() => reveal("loaded"), fallbackMs);
    },
    /** The app's first screen is up. */
    ready() {
      reveal("ready");
    },
    /** The page couldn't load: the offline page shows at once. */
    failed() {
      reveal("failed");
    },
    /** An update's being looked for or installed: not shown meanwhile. */
    hold() {
      held = true;
    },
    /** No update to install after all: shown, now or once it's ready. */
    release() {
      held = false;
      if (wanted) reveal(wanted);
    },
    get shown() {
      return shown;
    },
  };
}

module.exports = { createLaunch, FALLBACK_MS, MAX_MS };
