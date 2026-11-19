"use strict";
// The drawing area's stalls written to a log on this computer (1.31; Leo,
// 2026-11-19 14:40, "yes, lets start the build": drafting as smooth as a
// video game). The page notes each frame that misses the screen's refresh
// and what took the time (its lines, text, scans, autosave, the app's
// panels; components/editor in the web app's lib/editor/stall-recorder.ts)
// and hands the lines here in small batches, so they're read from
// %APPDATA%\CAD Protocol\stalls.log without asking Leo for anything.
//
// Plain text only, a batch at most MAX_BATCH long; the log kept to
// MAX_LOG_BYTES, the last one beside it as stalls.log.old. Writing never
// stops the app.

const fs = require("node:fs");
const path = require("node:path");

/** The most one batch from the page holds. */
const MAX_BATCH = 64 * 1024;
/** The log's size before it's moved aside for a new one. */
const MAX_LOG_BYTES = 1024 * 1024;

/** A batch as written: text, cut to MAX_BATCH, no control characters but tabs and new lines, ending in a new line. */
function cleanBatch(text) {
  if (typeof text !== "string" || text.length === 0) return null;
  // biome-ignore lint/suspicious/noControlCharactersInRegex: control characters are what's taken out
  const plain = text.slice(0, MAX_BATCH).replace(/[\u0000-\u0008\u000b-\u001f\u007f]/g, "");
  if (!plain.trim()) return null;
  return plain.endsWith("\n") ? plain : `${plain}\n`;
}

/** The log at `file`: append(text) writes a batch, true when it was. */
function createStallLog(file) {
  return {
    append(text) {
      const batch = cleanBatch(text);
      if (!batch) return false;
      try {
        fs.mkdirSync(path.dirname(file), { recursive: true });
        try {
          if (fs.statSync(file).size > MAX_LOG_BYTES) fs.renameSync(file, `${file}.old`);
        } catch {
          // No log yet.
        }
        fs.appendFileSync(file, batch);
        return true;
      } catch {
        return false;
      }
    },
  };
}

module.exports = { createStallLog, cleanBatch, MAX_BATCH, MAX_LOG_BYTES };
