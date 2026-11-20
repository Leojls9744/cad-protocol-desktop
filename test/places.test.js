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

test("1.33: finds a scan another computer linked, from the drawing's own folder, and keeps it under the drawing's id", () => {
  const { relSteps } = require("../src/places");
  // Two computers with the same shared folder in different places.
  const mine = tempDir();
  const theirs = tempDir();
  for (const root of [mine, theirs]) {
    fs.mkdirSync(path.join(root, "Job", "Drawings"), { recursive: true });
    fs.mkdirSync(path.join(root, "Job", "Scans", "Day 2"), { recursive: true });
    fs.writeFileSync(path.join(root, "Job", "Drawings", "Topo.cadp"), "drawing");
    fs.writeFileSync(path.join(root, "Job", "Scans", "Site.e57"), "e57-site-points");
    fs.writeFileSync(path.join(root, "Job", "Scans", "Day 2", "Roof.laz"), "laz-roof");
  }
  const dialog = { showOpenDialog: async () => ({ canceled: true, filePaths: [] }) };
  const a = createPlaces({ file: path.join(mine, "data", "places.json"), fs, dialog });
  const b = createPlaces({ file: path.join(theirs, "data", "places.json"), fs, dialog });
  // Linked on mine: the drawing keeps the place's id, its path there, and where it is from the drawing.
  const site = a.add([path.join(mine, "Job", "Scans", "Site.e57")])[0];
  const want = { id: site.id, rel: "../Scans/Site.e57", path: site.path, name: site.name, size: site.size };
  const drawingThere = path.join(theirs, "Job", "Drawings", "Topo.cadp");
  // Theirs doesn't know the id.
  assert.deepEqual(b.stat([site.id]), []);
  const found = b.resolve(drawingThere, want);
  assert.equal(found.id, site.id);
  assert.equal(found.path, path.join(theirs, "Job", "Scans", "Site.e57"));
  // Kept under the same id: asked for like any place from now on.
  assert.deepEqual(b.pathsOf([site.id]).filled.map((f) => f.path), [found.path]);
  const later = createPlaces({ file: path.join(theirs, "data", "places.json"), fs, dialog });
  assert.equal(later.stat([site.id])[0].there, true);
  // Backslashes too, as Windows writes them.
  const c = createPlaces({ file: path.join(theirs, "data2", "places.json"), fs, dialog });
  assert.equal(c.resolve(drawingThere, { ...want, rel: "..\\Scans\\Site.e57" }).path, found.path);

  // No `rel` (a drawing saved before): by its name round the drawing, a folder deeper too.
  const roof = a.add([path.join(mine, "Job", "Scans", "Day 2", "Roof.laz")])[0];
  const d = createPlaces({ file: path.join(theirs, "data3", "places.json"), fs, dialog });
  const byName = d.resolve(drawingThere, { id: roof.id, path: roof.path.replace(mine, "Q:\\nowhere"), name: roof.name, size: roof.size });
  assert.equal(byName.path, path.join(theirs, "Job", "Scans", "Day 2", "Roof.laz"));
  // The very path it had, when it's the same on both (no drawing file needed).
  const e = createPlaces({ file: path.join(theirs, "data4", "places.json"), fs, dialog });
  assert.equal(e.resolve(null, { id: site.id, path: site.path, name: site.name, size: site.size }).path, site.path);
  assert.equal(e.resolve(null, { id: roof.id, rel: "../Scans/Day 2/Roof.laz", name: roof.name, size: roof.size }), null);

  // Not the same file (another size), not a scan, a bad id or path: nothing.
  const f = createPlaces({ file: path.join(theirs, "data5", "places.json"), fs, dialog });
  assert.equal(f.resolve(drawingThere, { ...want, size: site.size + 1 }), null);
  assert.equal(f.resolve(drawingThere, { ...want, size: 0 }), null);
  assert.equal(f.resolve(drawingThere, { ...want, id: "nope" }), null);
  const drawingSize = fs.statSync(drawingThere).size;
  assert.equal(f.resolve(drawingThere, { id: site.id, rel: "Topo.cadp", name: "Topo.cadp", size: drawingSize }), null);
  assert.equal(f.resolve(drawingThere, { id: site.id, rel: "C:\\Windows\\win.ini", name: "win.ini", size: 10 }), null);
  assert.deepEqual(f.stat([site.id]), []);

  // A relative path's steps: up only at its start, no root, no drive.
  assert.deepEqual(relSteps("../Scans/Site.e57"), ["..", "Scans", "Site.e57"]);
  assert.deepEqual(relSteps("Scans\\Day 2\\Roof.laz"), ["Scans", "Day 2", "Roof.laz"]);
  assert.equal(relSteps("/etc/passwd"), null);
  assert.equal(relSteps("C:\\x.e57"), null);
  assert.equal(relSteps("Scans/../../x.e57"), null);
  assert.equal(relSteps("../".repeat(9) + "x.e57"), null);
  assert.equal(relSteps(".."), null);
  assert.equal(relSteps(""), null);
});
