"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { cadpName, createLocalFiles, withExtension } = require("../src/local-files");

async function folder() {
  return fs.mkdtemp(path.join(os.tmpdir(), "cadp-files-"));
}

function fakeDialog({ save = null, open = null } = {}) {
  const asked = [];
  return {
    asked,
    showSaveDialog: async (_win, options) => {
      asked.push(["save", options]);
      return save ? { canceled: false, filePath: save } : { canceled: true };
    },
    showOpenDialog: async (_win, options) => {
      asked.push(["open", options]);
      return open ? { canceled: false, filePaths: [open] } : { canceled: true, filePaths: [] };
    },
  };
}

test("suggested names are safe .cadp file names", () => {
  assert.equal(cadpName("Lot 7.cadp"), "Lot 7.cadp");
  assert.equal(cadpName("Lot 7"), "Lot 7.cadp");
  assert.equal(cadpName('a/b\\c:d*e?"f<g>h|'), "a b c d e f g h.cadp");
  assert.equal(cadpName(""), "Untitled.cadp");
  assert.equal(cadpName("..."), "Untitled.cadp");
  assert.equal(withExtension("C:\\x\\Lot"), "C:\\x\\Lot.cadp");
  assert.equal(withExtension("C:\\x\\Lot.CADP"), "C:\\x\\Lot.CADP");
});

test("Save to this computer writes the picked file and gives a token for next time", async () => {
  const dir = await folder();
  const dialog = fakeDialog({ save: path.join(dir, "Lot 7") });
  const files = createLocalFiles({ dialog, fs, documentsDir: dir });
  const saved = await files.saveAs(null, "Lot 7.cadp", Uint8Array.of(1, 2, 3));
  assert.equal(saved.name, "Lot 7.cadp");
  assert.equal(saved.path, path.join(dir, "Lot 7.cadp"));
  assert.deepEqual([...(await fs.readFile(saved.path))], [1, 2, 3]);
  assert.equal(dialog.asked[0][1].defaultPath, path.join(dir, "Lot 7.cadp"));
  // Ctrl+S: the same file, no window.
  const again = await files.save(saved.token, Uint8Array.of(4, 5));
  assert.equal(again.path, saved.path);
  assert.deepEqual([...(await fs.readFile(saved.path))], [4, 5]);
  assert.equal(dialog.asked.length, 1);
  // Nothing left beside it.
  assert.deepEqual(await fs.readdir(dir), ["Lot 7.cadp"]);
});

test("a cancelled window saves nothing", async () => {
  const dir = await folder();
  const files = createLocalFiles({ dialog: fakeDialog(), fs, documentsDir: dir });
  assert.equal(await files.saveAs(null, "x", Uint8Array.of(1)), null);
  assert.equal(await files.open(null), null);
  assert.deepEqual(await fs.readdir(dir), []);
});

test("only files picked in a window can be written", async () => {
  const dir = await folder();
  const files = createLocalFiles({ dialog: fakeDialog(), fs, documentsDir: dir });
  assert.equal(await files.save("made-up", Uint8Array.of(1)), null);
  assert.equal(await files.save(path.join(dir, "x.cadp"), Uint8Array.of(1)), null);
  assert.deepEqual(await fs.readdir(dir), []);
  await assert.rejects(files.saveAs(null, "x", "not bytes"));
});

test("Open from this computer reads the picked file and remembers it", async () => {
  const dir = await folder();
  const file = path.join(dir, "Topo.cadp");
  await fs.writeFile(file, Buffer.from([9, 8, 7]));
  const dialog = fakeDialog({ open: file });
  const files = createLocalFiles({ dialog, fs, documentsDir: dir });
  const opened = await files.open(null);
  assert.equal(opened.name, "Topo.cadp");
  assert.ok(opened.bytes instanceof Uint8Array);
  assert.deepEqual([...opened.bytes], [9, 8, 7]);
  await files.save(opened.token, Uint8Array.of(6));
  assert.deepEqual([...(await fs.readFile(file))], [6]);
  assert.deepEqual(dialog.asked[0][1].filters, [{ name: "CAD Protocol drawing", extensions: ["cadp"] }]);
});

test("a failed write leaves the drawing as it was", async () => {
  const dir = await folder();
  const file = path.join(dir, "Keep.cadp");
  await fs.writeFile(file, Buffer.from([1]));
  const broken = { ...fs, rename: async () => Promise.reject(new Error("disk full")) };
  const files = createLocalFiles({ dialog: fakeDialog({ open: file }), fs: broken, documentsDir: dir });
  const opened = await files.open(null);
  await assert.rejects(files.save(opened.token, Uint8Array.of(2)), /disk full/);
  assert.deepEqual([...(await fs.readFile(file))], [1]);
  assert.deepEqual(await fs.readdir(dir), ["Keep.cadp"]);
});
