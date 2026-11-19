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
// - 1.30 (Leo, 2026-11-19 07:30: "when there is a new update ... it still
//   happens in the backend, it causes the window to stay in the loading
//   screen until you close the app and restart it"): closing with an update
//   ready no longer installs it out of sight. electron-updater's install on
//   quit ran the installer silently, and the installer closes every copy of
//   the app it finds: opening the app again while it ran (the log: runs
//   ended within seconds, "put back after an unfinished run", as 1.28 and
//   1.29 came) left the card up and the app gone. Now closing installs it as
//   Restart to update does, the installer's progress shown and the app
//   opened again on the new version once it's done (installOnQuit).
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
  // Never installed out of sight as the app quits (1.30): installOnQuit.
  updater.autoInstallOnAppQuit = false;
  let installing = false;
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
      if (state.phase !== "ready" || installing) return false;
      installing = true;
      updater.quitAndInstall(false, true);
      return true;
    },
    /**
     * The app's closing (1.30): an update that's ready installs now, its
     * progress shown, and the app opens again on it when it's done. Once;
     * not after Restart to update (that's installing already).
     */
    installOnQuit() {
      if (state.phase !== "ready" || installing) return false;
      installing = true;
      try {
        // The installer's own window (not silent), the app run after.
        return updater.install(false, true) !== false;
      } catch (err) {
        log(`update install: ${err && err.message ? err.message : err}`);
        return false;
      }
    },
    /** Whether the update installs as the app closes (for the closing card). */
    get installsOnQuit() {
      return state.phase === "ready" && !installing;
    },
  };
}

module.exports = { createAutoUpdate, RELEASES_URL, EVERY_MS };
