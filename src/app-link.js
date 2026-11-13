"use strict";
// cadprotocol:// links (1.15; Leo, 2026-11-13 11:19: "Add a cadprotocol://
// link so other apps (like UnifyField) can open the CAD Protocol desktop
// app").
//
// The links are the app's in Windows: the installer registers them
// (build/installer.nsh; build.protocols in package.json names the scheme,
// which electron-builder's Windows installer doesn't act on by itself) and
// the app says so again each time it starts
// (app.setAsDefaultProtocolClient), which keeps it so for a copy moved or
// run from elsewhere. Windows then starts the app with the link as one of
// its arguments.
//
// cadprotocol://open starts the app, or, when it's open, brings it to the
// front as any second start does (the sign-in card or the start-up card
// first, if one's showing). That is all a link can do. Nothing else in the
// address is read (no path, no query, no name after the slashes), and a
// link is never a drawing to open, whatever it ends with: a page can make
// any link it likes, so a link must never name a file for the app to read.
// For the same reason no drawing is taken from a start that came with a
// link at all, in case something rode in beside it (local-files.js).

/** The links' scheme, as Windows knows it. */
const SCHEME = "cadprotocol";

/** An argument as it was typed: trimmed, without the quotes round it. */
function bare(arg) {
  return typeof arg === "string" ? arg.trim().replace(/^"(.*)"$/, "$1") : "";
}

/** Whether a command-line argument is one of the app's links. */
function isAppLink(arg) {
  return bare(arg).toLowerCase().startsWith(`${SCHEME}:`);
}

/** Whether the app was started (or started again) by one of its links. */
function hasAppLink(argv) {
  return Array.isArray(argv) && argv.some(isAppLink);
}

/**
 * Says to Windows that the links are this app's. Run from its source
 * (npm start), Windows needs the program and the app's folder; installed,
 * the app alone. False if Windows wouldn't take it.
 */
function registerAppLink(app, { defaultApp = false, execPath = "", argv = [], resolve = (p) => p } = {}) {
  try {
    if (defaultApp) {
      if (argv.length < 2) return false;
      return Boolean(app.setAsDefaultProtocolClient(SCHEME, execPath, [resolve(argv[1])]));
    }
    return Boolean(app.setAsDefaultProtocolClient(SCHEME));
  } catch {
    return false;
  }
}

module.exports = { SCHEME, isAppLink, hasAppLink, registerAppLink };
