"use strict";
// Keeping the desktop app up to date (1.3; Leo, 2026-11-08 08:36: "can we
// have it check for updates, and force the users to update every time it
// updates? can we have an update screen where it updates the installer
// instead of having to install it again? can we add a small verification
// badge in the desktop app that says up to date?").
//
// electron-updater reads latest.yml from the newest published GitHub release
// (drafts never), downloads the new installer and checks it against the
// SHA-512 there, and runs it silently.
// - 1.3–1.25: at launch the update screen checked first and installed a new
//   version before the app opened. 1.26: the app opens at once; it checks
//   then and every 6 hours, downloads quietly, and the app's badge says
//   "Restart to update"; closing installs it anyway.
// - 1.26 (Leo, 2026-11-17 20:32: "can the user just click update at the top
//   bar ... from their is has a progress bar and does the whole update"):
//   the badge checks when clicked, shows the download's progress as it
//   comes (told, not asked every half minute), and the install shows the
//   installer's own progress window before the app opens again (it had run
//   unseen, the app gone for a while).
//
// This file keeps the state the app's badge shows; main.js wires it up.

const RELEASES_URL = "https://github.com/leojls9744/cad-protocol-desktop/releases/latest";
const EVERY_MS = 6 * 60 * 60 * 1000;

/**
 * `updater` is electron-updater's autoUpdater (or a stand-in with the same
 * events). `onChange(state)` hears every change.
 */
function createAutoUpdate({ updater, current, log = () => {}, onChange = () => {} }) {
  let state = { current, latest: null, newer: false, url: RELEASES_URL, phase: "idle", percent: 0 };
  const set = (patch) => {
    state = { ...state, ...patch };
    onChange(state);
  };
  const version = (info) => (info && typeof info.version === "string" ? info.version : null);

  updater.autoDownload = true;
  updater.autoInstallOnAppQuit = true;
  updater.on("checking-for-update", () => {
    if (state.phase !== "downloading" && state.phase !== "ready") set({ phase: "checking" });
  });
  updater.on("update-not-available", (info) => {
    set({ phase: "up-to-date", latest: version(info) || current, newer: false, percent: 0 });
  });
  updater.on("update-available", (info) => {
    set({ phase: "downloading", latest: version(info), newer: true, percent: 0 });
  });
  updater.on("download-progress", (progress) => {
    const percent = Math.max(0, Math.min(100, Math.round(Number(progress && progress.percent) || 0)));
    set({ phase: "downloading", percent });
  });
  updater.on("update-downloaded", (info) => {
    set({ phase: "ready", latest: version(info) || state.latest, newer: true, percent: 100 });
  });
  updater.on("error", (err) => {
    log(`update: ${err && err.message ? err.message : err}`);
    // A failed check leaves the app as it is: "couldn't check", not "up to date".
    if (state.phase !== "ready") set({ phase: "error" });
  });

  return {
    get state() {
      return state;
    },
    /** Check now (and download if there's something new). Never throws. */
    async check() {
      try {
        await updater.checkForUpdates();
      } catch (err) {
        log(`update check: ${err && err.message ? err.message : err}`);
        if (state.phase !== "ready") set({ phase: "error" });
      }
      return state;
    },
    /** Restart into the new version, once it's downloaded: the installer's progress shown (1.26), then the app again. */
    installNow() {
      if (state.phase !== "ready") return false;
      updater.quitAndInstall(false, true);
      return true;
    },
  };
}

module.exports = { createAutoUpdate, RELEASES_URL, EVERY_MS };
