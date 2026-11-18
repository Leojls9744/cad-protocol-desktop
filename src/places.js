"use strict";
// Files remembered by their place on this computer (1.27; Leo, 2026-11-17
// 21:14: "we shouldn't be linking folders"; 21:22: "build it where the
// drawing remembers the scans place").
//
// A job's drawings, scans, PDFs and photos, and a drawing's scans, are kept
// by where they are on this computer, so opening them again needs no
// question. The page still never names a path: a place is made only for a
// file the person picked (Windows' Open window, here) or gave the page (a
// drop or an Open window there; the preload reads its path from the File),
// or a drawing opened or saved through local-files.js. The page gets each
// place's id, name and path, and asks for the file again by id: `fill` puts
// the files into an <input type=file> of the page's own (main.js, through
// the window's debugger), so the page gets real files read from disk, as
// from a drop, never copied into memory.
//
// Kept in places.json in the app's data folder (the last one beside it as
// places.json.bak), at most MAX_PLACES (the least lately used let go).

const path = require("node:path");
const crypto = require("node:crypto");

const MAX_PLACES = 20000;

/** What Windows' Open window shows, by what's asked for. */
const KINDS = {
  scan: [{ name: "Scans", extensions: ["las", "laz", "e57"] }],
  // 1.28: a scan the engine cuts (LAS or LAZ, by the name it's given).
  cut: [
    { name: "LAS", extensions: ["las"] },
    { name: "LAZ (compressed)", extensions: ["laz"] },
  ],
  pdf: [{ name: "PDF", extensions: ["pdf"] }],
  image: [{ name: "Pictures", extensions: ["png", "jpg", "jpeg", "tif", "tiff", "webp", "gif", "bmp"] }],
  drawing: [{ name: "Drawings", extensions: ["cadp", "dwg", "dxf"] }],
  any: [
    {
      name: "Job files",
      extensions: ["cadp", "dwg", "dxf", "pdf", "las", "laz", "e57", "png", "jpg", "jpeg", "tif", "tiff", "webp", "csv", "txt", "xlsx", "docx"],
    },
    { name: "All files", extensions: ["*"] },
  ],
};

const ID = /^p[0-9a-f]{24}$/;

/** Larger than any file the page makes to save (a cut scan). */
const MAX_SAVE_BYTES = 2 * 1024 * 1024 * 1024;

/** A file name Windows takes, as suggested. */
function safeName(name) {
  const clean = String(name ?? "")
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[. ]+$/, "");
  return clean || "Untitled";
}

/** A place's id. */
const newId = () => `p${crypto.randomBytes(12).toString("hex")}`;

/** The same file, any case (Windows). */
const sameFile = (a, b) => path.resolve(a).toLowerCase() === path.resolve(b).toLowerCase();

/**
 * `file` is places.json's path; `fs` node:fs (sync, as the settings are);
 * `dialog` Electron's.
 */
function createPlaces({ file, fs, dialog, log = () => {}, now = () => Date.now() }) {
  /** id → { path, name, size, modified, added, used } */
  let places = {};
  try {
    const read = JSON.parse(fs.readFileSync(file, "utf8"));
    if (read && typeof read.places === "object" && read.places !== null) places = read.places;
  } catch {
    // None yet, or unreadable: start empty (the .bak stays as it was).
  }

  function save() {
    try {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      if (fs.existsSync(file)) fs.copyFileSync(file, `${file}.bak`);
      const temp = `${file}.tmp`;
      fs.writeFileSync(temp, JSON.stringify({ version: 1, places }));
      fs.renameSync(temp, file);
    } catch (err) {
      log(`couldn't save places: ${err && err.message ? err.message : err}`);
    }
  }

  /** The least lately used let go past MAX_PLACES. */
  function trim() {
    const ids = Object.keys(places);
    if (ids.length <= MAX_PLACES) return;
    ids.sort((a, b) => (places[a].used || 0) - (places[b].used || 0));
    for (const id of ids.slice(0, ids.length - MAX_PLACES)) delete places[id];
  }

  const shown = (id) => {
    const p = places[id];
    return { id, name: p.name, path: p.path, size: p.size, modified: p.modified };
  };

  function statOf(p) {
    try {
      const s = fs.statSync(p);
      return s.isFile() ? { size: s.size, modified: Math.round(s.mtimeMs) } : null;
    } catch {
      return null;
    }
  }

  /** Files given by their paths, kept (one already kept keeps its id); the ones that are files. */
  function add(paths) {
    const out = [];
    let changed = false;
    for (const raw of Array.isArray(paths) ? paths : []) {
      if (typeof raw !== "string" || !raw || !path.isAbsolute(raw)) continue;
      const s = statOf(raw);
      if (!s) continue;
      let id = Object.keys(places).find((k) => sameFile(places[k].path, raw));
      if (!id) {
        id = newId();
        places[id] = { path: raw, name: path.basename(raw), added: now() };
      }
      Object.assign(places[id], { path: raw, name: path.basename(raw), size: s.size, modified: s.modified, used: now() });
      changed = true;
      if (!out.some((o) => o.id === id)) out.push(shown(id));
    }
    if (changed) {
      trim();
      save();
    }
    return out;
  }

  const known = (id) => typeof id === "string" && ID.test(id) && Object.hasOwn(places, id);

  return {
    add,

    /** Windows' Open window for files of a kind; those picked, kept. */
    async pick(win, options = {}) {
      const kind = Object.hasOwn(KINDS, options.kind) ? options.kind : "any";
      const picked = await dialog.showOpenDialog(win, {
        title: typeof options.title === "string" && options.title ? options.title.slice(0, 80) : "Add files",
        filters: KINDS[kind],
        properties: options.multiple === false ? ["openFile"] : ["openFile", "multiSelections"],
      });
      if (picked.canceled || !Array.isArray(picked.filePaths)) return [];
      const out = add(picked.filePaths);
      log(`places: ${out.length} picked`);
      return out;
    },

    /** Each place as it is now: there (its size and when it changed), or not. */
    stat(ids) {
      return (Array.isArray(ids) ? ids : []).filter(known).map((id) => {
        const s = statOf(places[id].path);
        return { ...shown(id), there: Boolean(s), ...(s || {}) };
      });
    },

    /** The paths of the places that are there now, in order, and which aren't. */
    pathsOf(ids) {
      const filled = [];
      const missing = [];
      for (const id of Array.isArray(ids) ? ids : []) {
        if (!known(id)) {
          missing.push(id);
          continue;
        }
        if (statOf(places[id].path)) {
          places[id].used = now();
          filled.push({ id, path: places[id].path });
        } else missing.push(id);
      }
      if (filled.length) save();
      return { filled, missing };
    },

    /** A moved file found again (Windows' Open window, where it was): the place, now there; null if not picked. */
    async find(win, id) {
      if (!known(id)) return null;
      const was = places[id];
      const ext = path.extname(was.name).replace(/^\./, "");
      const picked = await dialog.showOpenDialog(win, {
        title: `Find ${was.name}`,
        defaultPath: path.dirname(was.path),
        filters: ext ? [{ name: ext.toUpperCase(), extensions: [ext] }, { name: "All files", extensions: ["*"] }] : KINDS.any,
        properties: ["openFile"],
      });
      const found = picked.canceled ? undefined : picked.filePaths && picked.filePaths[0];
      const s = found ? statOf(found) : null;
      if (!found || !s) return null;
      // Another place of that file: this one takes its place.
      for (const k of Object.keys(places)) if (k !== id && sameFile(places[k].path, found)) delete places[k];
      Object.assign(places[id], { path: found, name: path.basename(found), size: s.size, modified: s.modified, used: now() });
      save();
      log(`places: found ${was.name} again`);
      return shown(id);
    },

    /** Places let go (nothing on the computer touched). */
    forget(ids) {
      let changed = false;
      for (const id of Array.isArray(ids) ? ids : []) {
        if (known(id)) {
          delete places[id];
          changed = true;
        }
      }
      if (changed) save();
      return changed;
    },

    /**
     * Windows' Save window for a file the page made (a cut scan, say),
     * beside the place `beside` when it's given; written (to a file beside
     * it, then renamed over), kept, and its place given; null if cancelled.
     */
    async saveAs(win, options, data) {
      const bytes =
        data instanceof Uint8Array ? Buffer.from(data.buffer, data.byteOffset, data.byteLength) : null;
      if (!bytes) throw new Error("a file's bytes are needed");
      if (bytes.length > MAX_SAVE_BYTES) throw new Error("That file is too large to save here.");
      const got = await saveWith(win, options, (temp) => fs.promises.writeFile(temp, bytes));
      return got ? got.place : null;
    },

    /**
     * 1.28: Windows' Save window as saveAs has it, the file then made by
     * `write(temp, chosen)` (the scan engine's cut, written straight to
     * disk), renamed over and kept: its place and what `write` gave; null
     * if cancelled.
     */
    saveWith: (win, options, write) => saveWith(win, options, write),

    /** A place's path, if it's kept (for local-files.js to open a drawing). */
    pathOf(id) {
      return known(id) ? places[id].path : null;
    },
  };

  async function saveWith(win, options, write) {
    const o = options && typeof options === "object" ? options : {};
    const name = safeName(o.name);
    const dir = known(o.beside) ? path.dirname(places[o.beside].path) : undefined;
    const kind = Object.hasOwn(KINDS, o.kind) ? o.kind : "any";
    const picked = await dialog.showSaveDialog(win, {
      title: "Save to this computer",
      defaultPath: dir ? path.join(dir, name) : name,
      filters: KINDS[kind],
      properties: ["showOverwriteConfirmation", "createDirectory"],
    });
    if (picked.canceled || !picked.filePath) return null;
    const temp = `${picked.filePath}.saving`;
    let made;
    try {
      made = await write(temp, picked.filePath);
      await fs.promises.rename(temp, picked.filePath);
    } catch (err) {
      await fs.promises.rm(temp, { force: true }).catch(() => {});
      throw err;
    }
    log(`places: saved ${path.basename(picked.filePath)}`);
    const place = add([picked.filePath])[0] || null;
    return place ? { place, made } : null;
  }
}

/**
 * Jobs (1.27): the page's own list of jobs, kept as it gives it (JSON text,
 * at most MAX_JOBS_BYTES) in jobs.json, the last one beside it as
 * jobs.json.bak, so a bad save never loses them.
 */
const MAX_JOBS_BYTES = 8 * 1024 * 1024;

function createJobsFile({ file, fs, log = () => {} }) {
  return {
    load() {
      try {
        return fs.readFileSync(file, "utf8");
      } catch {
        try {
          return fs.readFileSync(`${file}.bak`, "utf8");
        } catch {
          return null;
        }
      }
    },
    save(text) {
      if (typeof text !== "string" || Buffer.byteLength(text) > MAX_JOBS_BYTES) throw new Error("jobs too large");
      const parsed = JSON.parse(text);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("jobs must be an object");
      fs.mkdirSync(path.dirname(file), { recursive: true });
      if (fs.existsSync(file)) fs.copyFileSync(file, `${file}.bak`);
      const temp = `${file}.tmp`;
      fs.writeFileSync(temp, text);
      fs.renameSync(temp, file);
      log("jobs saved");
      return true;
    },
  };
}

/**
 * The kinds of file a place may be opened as in its own program (Windows'
 * default for it: a PDF reader, Photos, Excel, Word…), never a program or a
 * script.
 */
const OPEN_OUTSIDE = new Set([
  ".pdf",
  ".png",
  ".jpg",
  ".jpeg",
  ".tif",
  ".tiff",
  ".webp",
  ".gif",
  ".bmp",
  ".csv",
  ".txt",
  ".xlsx",
  ".xls",
  ".docx",
  ".doc",
]);

/** Whether a file may be opened in its own program (OPEN_OUTSIDE). */
const opensOutside = (file) => typeof file === "string" && OPEN_OUTSIDE.has(path.extname(file).toLowerCase());

/** A marker the page gives its <input type=file> (data-cadp-fill). */
const MARKER = /^[a-z0-9-]{8,64}$/;

/**
 * Files put into the page's <input data-cadp-fill="marker"> through the
 * window's debugger (DOM.setFileInputFiles), as a drop would give them:
 * `dbg` is webContents.debugger, attached here if it isn't.
 */
async function fillInput(dbg, marker, paths) {
  if (!MARKER.test(String(marker))) throw new Error("bad marker");
  if (!paths.length) return;
  if (!dbg.isAttached()) dbg.attach("1.3");
  const found = await dbg.sendCommand("Runtime.evaluate", {
    expression: `document.querySelector('input[type="file"][data-cadp-fill="${marker}"]')`,
  });
  const objectId = found && found.result && found.result.objectId;
  if (!objectId) throw new Error("the page's input isn't there");
  try {
    await dbg.sendCommand("DOM.setFileInputFiles", { files: paths, objectId });
  } finally {
    await dbg.sendCommand("Runtime.releaseObject", { objectId }).catch(() => {});
  }
}

module.exports = { KINDS, MAX_PLACES, MAX_SAVE_BYTES, safeName, MAX_JOBS_BYTES, MARKER, OPEN_OUTSIDE, opensOutside, createPlaces, createJobsFile, fillInput };
