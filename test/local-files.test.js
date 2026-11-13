"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { MAX_LAUNCHED, cadpName, companionOf, createLocalFiles, launchedPaths, withExtension } = require("../src/local-files");

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

test("1.12: a drawing's DWG copy is written beside it, and only there", async () => {
  assert.equal(companionOf("C:\\x\\Lot 7.cadp"), "C:\\x\\Lot 7.dwg");
  assert.equal(companionOf("C:\\x\\Lot 7.CADP"), "C:\\x\\Lot 7.dwg");
  assert.equal(companionOf("C:\\x\\Lot 7.json"), "C:\\x\\Lot 7.json.dwg");
  const dir = await folder();
  const dialog = fakeDialog({ save: path.join(dir, "Lot 7") });
  const files = createLocalFiles({ dialog, fs, documentsDir: dir });
  const saved = await files.saveAs(null, "Lot 7.cadp", Uint8Array.of(1, 2, 3));
  const beside = await files.saveBeside(saved.token, Uint8Array.of(7, 8));
  assert.deepEqual(beside, { name: "Lot 7.dwg", path: path.join(dir, "Lot 7.dwg") });
  assert.deepEqual([...(await fs.readFile(beside.path))], [7, 8]);
  // Written over on the next save; the drawing itself untouched.
  await files.saveBeside(saved.token, Uint8Array.of(9));
  assert.deepEqual([...(await fs.readFile(beside.path))], [9]);
  assert.deepEqual([...(await fs.readFile(saved.path))], [1, 2, 3]);
  assert.deepEqual((await fs.readdir(dir)).sort(), ["Lot 7.cadp", "Lot 7.dwg"]);
  // A token it never gave, or a path, writes nothing.
  assert.equal(await files.saveBeside("made-up", Uint8Array.of(1)), null);
  assert.equal(await files.saveBeside(path.join(dir, "x.cadp"), Uint8Array.of(1)), null);
  await assert.rejects(files.saveBeside(saved.token, "not bytes"));
  assert.deepEqual((await fs.readdir(dir)).sort(), ["Lot 7.cadp", "Lot 7.dwg"]);
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

test("1.11: the .cadp files Windows started the app with, from its command line", () => {
  const argv = [
    "C:\\Users\\leo\\AppData\\Local\\Programs\\cad-protocol\\CAD Protocol.exe",
    "--allow-file-access-from-files",
    "--original-process-start-time=13370000000000000",
    "C:\\Jobs\\26258 Smith\\Smith ALTA.cadp",
    '"C:\\Jobs\\Lot 7.CADP"',
    "C:\\Jobs\\26258 Smith\\smith alta.cadp",
    "C:\\Jobs\\notes.txt",
    "Lot 9.cadp",
    ".",
    42,
  ];
  assert.deepEqual(launchedPaths(argv), ["C:\\Jobs\\26258 Smith\\Smith ALTA.cadp", "C:\\Jobs\\Lot 7.CADP"]);
  assert.deepEqual(launchedPaths(undefined), []);
  assert.deepEqual(launchedPaths(["/home/leo/Topo.cadp"]), ["/home/leo/Topo.cadp"]);
});

test("1.11: double-clicked drawings wait, are handed over once each, and save back to their file", async () => {
  const dir = await folder();
  const one = path.join(dir, "Smith ALTA.cadp");
  const two = path.join(dir, "Lot 7.cadp");
  await fs.writeFile(one, Buffer.from([1, 2]));
  await fs.writeFile(two, Buffer.from([3]));
  const dialog = fakeDialog();
  const files = createLocalFiles({ dialog, fs, documentsDir: dir });
  assert.equal(files.launchedCount(), 0);
  assert.deepEqual(await files.takeLaunched(), []);
  assert.equal(files.launch(["app.exe", one]), 1);
  // The same file again (two double-clicks) waits once.
  assert.equal(files.launch(["app.exe", "--x", one.toUpperCase(), two]), 2);
  const taken = await files.takeLaunched();
  assert.deepEqual(
    taken.map((t) => [t.name, [...t.bytes]]),
    [["Smith ALTA.cadp", [1, 2]], ["Lot 7.cadp", [3]]],
  );
  assert.ok(taken[0].bytes instanceof Uint8Array);
  assert.equal(files.launchedCount(), 0);
  assert.deepEqual(await files.takeLaunched(), []);
  // Ctrl+S writes the file that was double-clicked, no window.
  await files.save(taken[0].token, Uint8Array.of(9));
  assert.deepEqual([...(await fs.readFile(one))], [9]);
  assert.equal(dialog.asked.length, 0);
});

test("1.11: a double-clicked drawing that's gone says so, and the rest still open", async () => {
  const dir = await folder();
  const here = path.join(dir, "Here.cadp");
  await fs.writeFile(here, Buffer.from([5]));
  const files = createLocalFiles({ dialog: fakeDialog(), fs, documentsDir: dir });
  files.launch([path.join(dir, "Moved.cadp"), here]);
  const [gone, ok] = await files.takeLaunched();
  assert.equal(gone.name, "Moved.cadp");
  assert.equal(gone.error, "The file isn't there any more.");
  assert.equal(gone.token, undefined);
  assert.deepEqual([...ok.bytes], [5]);
});

test("1.11: at most MAX_LAUNCHED drawings wait at once", () => {
  const files = createLocalFiles({ dialog: fakeDialog(), fs, documentsDir: os.tmpdir() });
  const many = Array.from({ length: MAX_LAUNCHED + 5 }, (_, i) => path.join(os.tmpdir(), `d${i}.cadp`));
  assert.equal(files.launch(many), MAX_LAUNCHED);
  assert.equal(files.launch([path.join(os.tmpdir(), "late.cadp")]), MAX_LAUNCHED);
});
