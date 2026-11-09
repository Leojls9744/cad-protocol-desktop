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

const path = require("node:path");
const crypto = require("node:crypto");

const EXTENSION = ".cadp";
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

/** The picked path, with .cadp on the end. */
function withExtension(file) {
  return file.toLowerCase().endsWith(EXTENSION) ? file : `${file}${EXTENSION}`;
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
      const stat = await fs.stat(file);
      if (stat.size > MAX_BYTES) throw new Error("That file is too large to be a drawing.");
      const bytes = await fs.readFile(file);
      log(`opened a drawing from ${file}`);
      return {
        ...answer(remember(file), file),
        bytes: new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength),
      };
    },
  };
}

module.exports = { EXTENSION, FILTERS, MAX_BYTES, cadpName, withExtension, createLocalFiles };
