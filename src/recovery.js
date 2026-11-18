"use strict";
// Drawing Recovery's copies kept as files on this computer (1.28; Leo,
// 2026-11-18 06:04: "lets build all 8 of you recommendations to make a true
// desktop app"): the page's autosave copies (each drawing with unsaved
// changes, every few minutes and as the window closes) written to
// %APPDATA%\CAD Protocol\recovery, one file each, in place of the browser
// engine's storage, which Windows' browser engine can clear and which a
// reinstall or a cleared profile loses. Each is written beside itself, then
// renamed over, so a crash part way never leaves a broken copy.
//
// The page names a copy only by its id (the drawing's file, or its tab);
// the file it's kept in is named by that id's hash, never by anything the
// page gives as a path.

const path = require("node:path");
const crypto = require("node:crypto");

/** The most one copy holds (a drawing's text and what's said of it). */
const MAX_COPY_BYTES = 512 * 1024 * 1024;
/** The most copies kept (the oldest let go past it). */
const MOST_COPIES = 200;

const str = (v, most = 4096) => typeof v === "string" && v.length <= most;
const strOrNull = (v) => v === null || str(v);

/** Whether `c` is a copy as the page makes them (components/editor/autosave.ts RecoveryCopy). */
function isCopy(c) {
  return (
    c !== null &&
    typeof c === "object" &&
    str(c.id, 1024) &&
    c.id.length > 0 &&
    str(c.user, 512) &&
    str(c.name, 1024) &&
    strOrNull(c.projectId) &&
    strOrNull(c.fileId) &&
    strOrNull(c.owner) &&
    Number.isFinite(c.savedAt) &&
    Number.isFinite(c.objects) &&
    typeof c.text === "string"
  );
}

function createRecovery({ dir, fs, log = () => {} }) {
  const fileOf = (id) => path.join(dir, `${crypto.createHash("sha256").update(id).digest("hex").slice(0, 40)}.json`);
  const own = (c) => ({
    id: c.id,
    user: c.user,
    name: c.name,
    projectId: c.projectId,
    fileId: c.fileId,
    owner: c.owner,
    savedAt: c.savedAt,
    objects: c.objects,
    text: c.text,
  });

  async function list() {
    let names = [];
    try {
      names = await fs.promises.readdir(dir);
    } catch {
      return [];
    }
    const out = [];
    for (const n of names) {
      if (!n.endsWith(".json")) continue;
      try {
        const c = JSON.parse(await fs.promises.readFile(path.join(dir, n), "utf8"));
        if (isCopy(c)) out.push(own(c));
      } catch {
        // Not one of ours, or cut short: left as it is.
      }
    }
    return out;
  }

  return {
    async put(copy) {
      if (!isCopy(copy)) throw new Error("not a recovery copy");
      const text = JSON.stringify(own(copy));
      if (Buffer.byteLength(text) > MAX_COPY_BYTES) throw new Error("That drawing is too large to keep a copy of.");
      await fs.promises.mkdir(dir, { recursive: true });
      const file = fileOf(copy.id);
      const temp = `${file}.saving`;
      try {
        await fs.promises.writeFile(temp, text);
        await fs.promises.rename(temp, file);
      } catch (err) {
        await fs.promises.rm(temp, { force: true }).catch(() => {});
        throw err;
      }
      // Too many: the oldest let go.
      const all = await list();
      if (all.length > MOST_COPIES) {
        all.sort((a, b) => a.savedAt - b.savedAt);
        for (const c of all.slice(0, all.length - MOST_COPIES)) await fs.promises.rm(fileOf(c.id), { force: true }).catch(() => {});
        log(`recovery: ${all.length - MOST_COPIES} old copies let go`);
      }
      return true;
    },
    list,
    async remove(id) {
      if (!str(id, 1024) || !id) return false;
      await fs.promises.rm(fileOf(id), { force: true });
      return true;
    },
  };
}

module.exports = { createRecovery, isCopy, MAX_COPY_BYTES, MOST_COPIES };
