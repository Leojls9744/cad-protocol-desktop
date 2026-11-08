"use strict";
// A few desktop settings in one small JSON file in the app's own data folder
// (%APPDATA%\CAD Protocol on Windows). Drawings and files are never kept
// here: they stay in the person's encrypted vault in the web app.

const fs = require("node:fs");
const path = require("node:path");

function createStore(file, log = () => {}) {
  let data = {};
  try {
    const read = JSON.parse(fs.readFileSync(file, "utf8"));
    if (read && typeof read === "object" && !Array.isArray(read)) data = read;
  } catch {
    // No file yet, or an unreadable one: start fresh.
  }

  function save() {
    try {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      const temp = `${file}.tmp`;
      fs.writeFileSync(temp, JSON.stringify(data, null, 2));
      fs.renameSync(temp, file);
    } catch (err) {
      log(`couldn't save settings: ${err && err.message ? err.message : err}`);
    }
  }

  return {
    get(key, fallback) {
      return Object.hasOwn(data, key) ? data[key] : fallback;
    },
    set(key, value) {
      data[key] = value;
      save();
    },
  };
}

/** A short log beside the settings, kept under about 256 KB, for when something needs looking into. */
function createLog(file) {
  return (line) => {
    try {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      try {
        if (fs.statSync(file).size > 256 * 1024) fs.renameSync(file, `${file}.old`);
      } catch {
        // No log yet.
      }
      fs.appendFileSync(file, `${new Date().toISOString()} ${line}\n`);
    } catch {
      // Logging must never stop the app.
    }
  };
}

module.exports = { createStore, createLog };
