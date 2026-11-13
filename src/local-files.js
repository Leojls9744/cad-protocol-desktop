"use strict";
// Drawings as files on this computer (1.5).
//
// Spatial Canvas keeps drawings in the person's encrypted vault, which needs
// a connection. In the desktop app it can also save a drawing as a .cadp file
// in any folder and open one again, online or offline, through Windows' own
// Save and Open windows.
//
// The page never names a path itself. It only gets files the person picked
// in one of those windows, each as a token. Saving to the same file again
// uses that token, so the page can only write files it was given. Tokens
// last until the app closes, and after that the person picks the file again.
// A save is written to a file beside the drawing and then renamed over it,
// so a failed save never leaves half a drawing.
//
// 1.11: a .cadp double-clicked in Windows starts the app with its path (or
// hands it to the open app). Those count as picked too: they wait here
// until the page takes them, each read once and given a token like Open's.
//
// 1.12: a drawing's DWG copy beside it (the app's Companion .dwg). The page
// still names no path: it gives a drawing's token, and only the file of the
// same name ending .dwg in the same folder is written.

const path = require("node:path");
const { hasAppLink } = require("./app-link");
const crypto = require("node:crypto");

const EXTENSION = ".cadp";
/** 1.12: a drawing's DWG copy ends this, beside it. */
const COMPANION = ".dwg";
const FILTERS = [{ name: "CAD Protocol drawing", extensions: ["cadp"] }];
/** Larger than any drawing (scans aren't kept in a .cadp). */
const MAX_BYTES = 1024 * 1024 * 1024;

/** A safe file name for the suggestion: "Lot 7.cadp". */
function cadpName(name) {
  const base = String(name ?? "")
    .replace(/\.cadp$/i, "")
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[. ]+$/, "");
  return `${base || "Untitled"}${EXTENSION}`;
}

/** 1.12: the DWG beside a drawing: "Lot 7.cadp" → "Lot 7.dwg" (any other name gets .dwg added). */
function companionOf(file) {
  const base = file.toLowerCase().endsWith(EXTENSION) ? file.slice(0, -EXTENSION.length) : file;
  return `${base}${COMPANION}`;
}

/** The picked path, with .cadp on the end. */
function withExtension(file) {
  return file.toLowerCase().endsWith(EXTENSION) ? file : `${file}${EXTENSION}`;
}

/** At most this many double-clicked drawings wait for the page at once. */
const MAX_LAUNCHED = 20;

/**
 * 1.11: the .cadp files Windows started the app with (a double-click, or
 * Open with), from its command line: whole paths ending .cadp, not the
 * switches Electron and Chromium add.
 * 1.15: none from a start that came with a cadprotocol:// link (app-link.js).
 * A link is never a drawing, whatever it ends with, and nothing that rode
 * in beside one is either: a link comes from a page, not from a double-click.
 */
function launchedPaths(argv) {
  const seen = new Set();
  const out = [];
  if (hasAppLink(argv)) return out;
  for (const raw of Array.isArray(argv) ? argv : []) {
    if (typeof raw !== "string") continue;
    const a = raw.trim().replace(/^"(.*)"$/, "$1");
    if (!a || a.startsWith("-")) continue;
    if (!a.toLowerCase().endsWith(EXTENSION)) continue;
    if (!path.isAbsolute(a) && !path.win32.isAbsolute(a)) continue;
    const key = a.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(a);
  }
  return out;
}

/** What the page sent, as bytes, or an error. */
function asBuffer(data) {
  if (data instanceof Uint8Array) return Buffer.from(data.buffer, data.byteOffset, data.byteLength);
  if (data instanceof ArrayBuffer) return Buffer.from(data);
  throw new Error("a drawing's bytes are needed");
}

/**
 * `dialog` is Electron's (showSaveDialog, showOpenDialog), `fs` is
 * node:fs/promises and `documentsDir` is the first folder offered.
 */
function createLocalFiles({ dialog, fs, documentsDir, log = () => {} }) {
  const known = new Map();
  let lastDir = documentsDir;

  function remember(file) {
    const token = crypto.randomUUID();
    known.set(token, file);
    lastDir = path.dirname(file);
    return token;
  }

  async function writeSafely(file, bytes) {
    if (bytes.length > MAX_BYTES) throw new Error("The drawing is too large to save as one file.");
    const temp = `${file}.saving`;
    try {
      await fs.writeFile(temp, bytes);
      await fs.rename(temp, file);
    } catch (err) {
      await fs.rm(temp, { force: true }).catch(() => {});
      throw err;
    }
  }

  const answer = (token, file) => ({ token, name: path.basename(file), path: file });

  /** A file's bytes, read for the page, with its token. */
  async function read(file) {
    const stat = await fs.stat(file);
    if (stat.size > MAX_BYTES) throw new Error("That file is too large to be a drawing.");
    const bytes = await fs.readFile(file);
    return {
      ...answer(remember(file), file),
      bytes: new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength),
    };
  }

  /** 1.11: double-clicked drawings waiting for the page to take them. */
  const launched = [];

  return {
    /** The Save window, then the file written; null when the person cancels. */
    async saveAs(win, suggested, data) {
      const bytes = asBuffer(data);
      const picked = await dialog.showSaveDialog(win, {
        title: "Save to this computer",
        defaultPath: path.join(lastDir, cadpName(suggested)),
        filters: FILTERS,
        properties: ["showOverwriteConfirmation", "createDirectory"],
      });
      if (picked.canceled || !picked.filePath) return null;
      const file = withExtension(picked.filePath);
      await writeSafely(file, bytes);
      log(`saved a drawing to ${file}`);
      return answer(remember(file), file);
    },

    /** The file the token names, written again; null when the token isn't known (pick it again). */
    async save(token, data) {
      const file = typeof token === "string" ? known.get(token) : undefined;
      if (!file) return null;
      await writeSafely(file, asBuffer(data));
      return answer(token, file);
    },

    /**
     * 1.12: the DWG copy of the drawing the token names, written beside it
     * (over the last one, as each save does): { name, path }. null when the
     * token isn't known, as save's.
     */
    async saveBeside(token, data) {
      const file = typeof token === "string" ? known.get(token) : undefined;
      if (!file) return null;
      const beside = companionOf(file);
      if (beside.toLowerCase() === file.toLowerCase()) return null;
      await writeSafely(beside, asBuffer(data));
      return { name: path.basename(beside), path: beside };
    },

    /** The Open window, then the file's bytes; null when the person cancels. */
    async open(win) {
      const picked = await dialog.showOpenDialog(win, {
        title: "Open from this computer",
        defaultPath: lastDir,
        filters: FILTERS,
        properties: ["openFile"],
      });
      const file = picked.canceled ? undefined : picked.filePaths && picked.filePaths[0];
      if (!file) return null;
      const opened = await read(file);
      log(`opened a drawing from ${file}`);
      return opened;
    },

    /**
     * 1.11: drawings Windows handed the app (a double-click on a .cadp), kept
     * until the page takes them; how many now wait.
     */
    launch(paths) {
      for (const file of launchedPaths(paths)) {
        if (launched.some((f) => f.toLowerCase() === file.toLowerCase())) continue;
        if (launched.length >= MAX_LAUNCHED) break;
        launched.push(file);
      }
      return launched.length;
    },

    /** How many double-clicked drawings wait. */
    launchedCount() {
      return launched.length;
    },

    /**
     * The double-clicked drawings, read and handed over once each: { token,
     * name, path, bytes }, or { name, path, error } for one that couldn't be
     * read (moved, say). Saving again writes to the same file, as after Open.
     */
    async takeLaunched() {
      const files = launched.splice(0, launched.length);
      const out = [];
      for (const file of files) {
        try {
          out.push(await read(file));
          log(`opened a drawing Windows handed over: ${file}`);
        } catch (err) {
          log(`couldn't open ${file}: ${err && err.message}`);
          out.push({
            name: path.basename(file),
            path: file,
            error: err && err.code === "ENOENT" ? "The file isn't there any more." : (err && err.message) || "It couldn't be read.",
          });
        }
      }
      return out;
    },
  };
}

module.exports = {
  EXTENSION,
  COMPANION,
  companionOf,
  FILTERS,
  MAX_BYTES,
  MAX_LAUNCHED,
  cadpName,
  withExtension,
  launchedPaths,
  createLocalFiles,
};
