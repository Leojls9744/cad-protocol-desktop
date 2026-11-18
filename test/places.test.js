"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { createJobsFile, createPlaces, fillInput, opensOutside, MAX_JOBS_BYTES } = require("../src/places");

const tempDir = () => fs.mkdtempSync(path.join(os.tmpdir(), "cadp-places-"));

test("keeps picked and given files by their place, one id a file, and finds a moved one", async () => {
  const dir = tempDir();
  const scan = path.join(dir, "Smith.e57");
  const plan = path.join(dir, "Plat.pdf");
  fs.writeFileSync(scan, "e57");
  fs.writeFileSync(plan, "pdf");
  const asked = [];
  let answer = { canceled: false, filePaths: [scan, plan] };
  const dialog = { showOpenDialog: async (_win, o) => (asked.push(o), answer) };
  const file = path.join(dir, "data", "places.json");
  const places = createPlaces({ file, fs, dialog });

  const picked = await places.pick(null, { kind: "any" });
  assert.deepEqual(picked.map((p) => p.name), ["Smith.e57", "Plat.pdf"]);
  assert.match(picked[0].id, /^p[0-9a-f]{24}$/);
  assert.ok(asked[0].properties.includes("multiSelections"));
  // Given again (a drop): the same id.
  const again = places.add([scan]);
  assert.equal(again[0].id, picked[0].id);
  // Not a file, or not a whole path: nothing.
  assert.deepEqual(places.add([dir, "Plat.pdf", 7]), []);

  // Kept: read again from places.json.
  const later = createPlaces({ file, fs, dialog });
  assert.deepEqual(later.stat([picked[0].id]).map((p) => [p.name, p.there]), [["Smith.e57", true]]);
  assert.deepEqual(later.stat(["../etc/passwd", "p" + "0".repeat(24)]), []);

  // Moved: not there, then found again where it went.
  const moved = path.join(dir, "Moved.e57");
  fs.renameSync(scan, moved);
  assert.equal(later.stat([picked[0].id])[0].there, false);
  const { filled, missing } = later.pathsOf([picked[0].id, picked[1].id]);
  assert.deepEqual(filled.map((f) => f.path), [plan]);
  assert.deepEqual(missing, [picked[0].id]);
  answer = { canceled: false, filePaths: [moved] };
  const found = await later.find(null, picked[0].id);
  assert.equal(found.name, "Moved.e57");
  assert.equal(asked.at(-1).defaultPath, dir);
  assert.equal(later.pathOf(picked[0].id), moved);
  assert.ok(fs.existsSync(`${file}.bak`));

  // Forgotten: nothing on the computer touched.
  assert.equal(later.forget([picked[1].id]), true);
  assert.equal(later.pathOf(picked[1].id), null);
  assert.ok(fs.existsSync(plan));
  // Cancelled: nothing.
  answer = { canceled: true, filePaths: [] };
  assert.deepEqual(await later.pick(null, {}), []);
});

test("keeps the jobs as given, the last beside them, and refuses what isn't a list of jobs", () => {
  const dir = tempDir();
  const file = path.join(dir, "jobs.json");
  const jobs = createJobsFile({ file, fs });
  assert.equal(jobs.load(), null);
  jobs.save(JSON.stringify({ version: 1, jobs: [{ id: "j1", name: "Smith" }] }));
  jobs.save(JSON.stringify({ version: 1, jobs: [] }));
  assert.equal(JSON.parse(jobs.load()).jobs.length, 0);
  assert.equal(JSON.parse(fs.readFileSync(`${file}.bak`, "utf8")).jobs[0].name, "Smith");
  assert.throws(() => jobs.save("[1]"));
  assert.throws(() => jobs.save("not json"));
  assert.throws(() => jobs.save(" ".repeat(MAX_JOBS_BYTES + 1)));
  // Unreadable: the last one.
  fs.writeFileSync(file, "");
  fs.rmSync(file);
  assert.equal(JSON.parse(jobs.load()).jobs[0].name, "Smith");
});

test("fills the page's own file input through the debugger, by its marker only", async () => {
  const sent = [];
  let attached = false;
  const dbg = {
    isAttached: () => attached,
    attach: () => {
      attached = true;
    },
    sendCommand: async (method, params) => {
      sent.push([method, params]);
      return method === "Runtime.evaluate" ? { result: { objectId: "o1" } } : {};
    },
  };
  await fillInput(dbg, "fill-abc12345", ["C:\\Jobs\\a.e57"]);
  assert.equal(attached, true);
  assert.match(sent[0][1].expression, /data-cadp-fill="fill-abc12345"/);
  assert.deepEqual(sent[1], ["DOM.setFileInputFiles", { files: ["C:\\Jobs\\a.e57"], objectId: "o1" }]);
  assert.equal(sent[2][0], "Runtime.releaseObject");
  await assert.rejects(fillInput(dbg, "x\"]; alert(1); //", ["a"]));
});

test("opens only documents and pictures in their own programs, never a program", () => {
  for (const ok of ["C:\\Jobs\\Plat.PDF", "a.jpg", "b.xlsx", "c.docx", "d.csv"]) assert.equal(opensOutside(ok), true, ok);
  for (const no of ["setup.exe", "run.bat", "x.cmd", "y.ps1", "z.lnk", "a.cadp", "b.e57", "noext", null]) assert.equal(opensOutside(no), false, String(no));
});

test("saves a file the page made beside a place, keeps it, and never names a path itself", async () => {
  const dir = tempDir();
  const scan = path.join(dir, "Site.las");
  fs.writeFileSync(scan, "las");
  const asked = [];
  const out = path.join(dir, "Site cut.las");
  const dialog = {
    showOpenDialog: async () => ({ canceled: false, filePaths: [scan] }),
    showSaveDialog: async (_w, o) => (asked.push(o), { canceled: false, filePath: out }),
  };
  const places = createPlaces({ file: path.join(dir, "places.json"), fs, dialog });
  const [original] = await places.pick(null, { kind: "scan" });
  const saved = await places.saveAs(null, { name: 'Site: "cut".las', beside: original.id, kind: "scan" }, new Uint8Array([1, 2, 3]));
  assert.equal(asked[0].defaultPath, path.join(dir, "Site cut .las"));
  assert.equal(saved.name, "Site cut.las");
  assert.deepEqual([...fs.readFileSync(out)], [1, 2, 3]);
  assert.equal(fs.existsSync(`${out}.saving`), false);
  await assert.rejects(places.saveAs(null, {}, "not bytes"));
  dialog.showSaveDialog = async () => ({ canceled: true });
  assert.equal(await places.saveAs(null, { name: "x" }, new Uint8Array(1)), null);
});
