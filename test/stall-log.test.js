"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { createStallLog, cleanBatch, MAX_BATCH, MAX_LOG_BYTES } = require("../src/stall-log");

test("1.31: the page's stalls appended to stalls.log, plain text, moved aside when full", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "cadp-stalls-"));
  const file = path.join(dir, "data", "stalls.log");
  const log = createStallLog(file);
  assert.equal(log.append("2026-11-19T22:50:01Z 41 ms pan · lines 22"), true);
  assert.equal(log.append("second\nthird\n"), true);
  assert.equal(fs.readFileSync(file, "utf8"), "2026-11-19T22:50:01Z 41 ms pan · lines 22\nsecond\nthird\n");
  // Nothing, or not text: nothing written.
  assert.equal(log.append(""), false);
  assert.equal(log.append(42), false);
  assert.equal(log.append({ x: 1 }), false);
  assert.equal(log.append("\u0001\u0002"), false);
  // Control characters out, a batch cut to its most.
  assert.equal(cleanBatch("a\u0007b\tc"), "ab\tc\n");
  assert.equal(cleanBatch("x".repeat(MAX_BATCH + 10)).length, MAX_BATCH + 1);
  // Full: moved aside as stalls.log.old, a new one begun.
  fs.writeFileSync(file, "y".repeat(MAX_LOG_BYTES + 1));
  assert.equal(log.append("after"), true);
  assert.equal(fs.readFileSync(file, "utf8"), "after\n");
  assert.ok(fs.existsSync(`${file}.old`));
});

test("1.31: the bridge and the handler are there, the app's own pages only", () => {
  const preload = fs.readFileSync(path.join(__dirname, "../src/preload.js"), "utf8");
  const main = fs.readFileSync(path.join(__dirname, "../src/main.js"), "utf8");
  assert.match(preload, /stalls: \{\s*append: \(text\) => ipcRenderer\.invoke\("cadp:stalls:append", text\)/);
  assert.match(main, /ipcMain\.handle\("cadp:stalls:append", \(event, text\) => \{\s*if \(!fromApp\(event\)\) throw new Error\("not allowed"\);\s*return stallLog\.append\(text\);/);
  assert.match(main, /createStallLog\(path\.join\(data, "stalls\.log"\)\)/);
});
