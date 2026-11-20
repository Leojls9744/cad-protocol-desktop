"use strict";
// Updates on the start-up card (1.32; Leo, 2026-11-19 17:31: "when the load
// card is running, can it check if their are any updates and if so run
// them on the opening card"). As the app starts, the card looks for a new
// version of the desktop app for CHECK_MS at most. None (or no answer, or
// offline): the app opens. One: it downloads with its progress on the
// card, then installs in sight (the installer's own progress) and the app
// opens again on it. A download still going after DOWNLOAD_MS opens the
// app anyway, the download going on behind it (the top bar's badge, as
// 1.26). 1.26 had opened the app at once and looked behind it.
//
// Plain data and timers, so it's tested without Electron.

/** The card waits this long for the check's answer, ms. */
const CHECK_MS = 6000;
/** A download still going this long after it began: the app opens, the download goes on, ms. */
const DOWNLOAD_MS = 120000;

/**
 * `updates` is auto-update.js's (check(), installNow(), state). `say(title,
 * detail, percent)` writes on the card. Resolves "open" (load the app) or
 * "installing" (the app quits into the installer).
 */
function createStartCheck({
  updates,
  say,
  log = () => {},
  checkMs = CHECK_MS,
  downloadMs = DOWNLOAD_MS,
  setTimer = setTimeout,
  clearTimer = clearTimeout,
}) {
  let settle = null;
  let timer = null;
  let phase = "idle";

  function finish(how) {
    if (!settle) return;
    const done = settle;
    settle = null;
    if (timer) clearTimer(timer);
    timer = null;
    phase = how;
    done(how);
  }

  function install(latest) {
    say("Installing the update…", latest ? `CAD Protocol ${latest}` : "CAD Protocol", 100);
    log(`update: installing ${latest || "the new version"} from the start-up card`);
    if (updates.installNow()) finish("installing");
    else finish("open");
  }

  /** An update's state as it changes (main.js tells every change). */
  function heard(state) {
    if (!settle || !state) return;
    if (state.phase === "ready") {
      install(state.latest);
      return;
    }
    if (state.phase === "downloading") {
      if (phase !== "downloading") {
        phase = "downloading";
        if (timer) clearTimer(timer);
        // Too long: the app opens, the download goes on behind it.
        timer = setTimer(() => {
          log("update: still downloading; the app opens meanwhile");
          finish("open");
        }, downloadMs);
      }
      say(
        "Updating CAD Protocol…",
        state.latest ? `Downloading ${state.latest} · ${state.percent || 0}%` : `Downloading · ${state.percent || 0}%`,
        Number(state.percent) || 0,
      );
      return;
    }
    if (state.phase === "up-to-date" || state.phase === "error") finish("open");
  }

  return {
    heard,
    /** Look for an update on the card; resolves how the start goes on. */
    run() {
      if (!updates) return Promise.resolve("open");
      return new Promise((resolve) => {
        settle = resolve;
        say("Checking for updates…", "CAD Protocol for Windows");
        // No answer in time (a slow network): the app opens.
        timer = setTimer(() => {
          if (phase !== "downloading") finish("open");
        }, checkMs);
        const now = updates.state;
        // Downloaded already (before the last close): installed now.
        if (now && now.phase === "ready") {
          install(now.latest);
          return;
        }
        Promise.resolve()
          .then(() => updates.check())
          .then((state) => heard(state))
          .catch(() => finish("open"));
      });
    },
    get phase() {
      return phase;
    },
  };
}

module.exports = { createStartCheck, CHECK_MS, DOWNLOAD_MS };
