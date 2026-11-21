"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { EventEmitter } = require("node:events");
const { createHost } = require("../src/engine-host");
const { lasBytes, samplePoints } = require("../src/las-sample");

function port() {
  const p = new EventEmitter();
  p.out = [];
  p.postMessage = (m) => p.out.push(m);
  p.start = () => {};
  return p;
}

function fakeEngine() {
  const opened = [];
  return {
    opened,
    engineVersion: () => "0.1.0",
    openProgress: () => 0.5,
    openScan: (p) => {
      opened.push(p);
      if (p.endsWith("bad.las")) return Promise.reject(new Error("not a LAS file"));
      return Promise.resolve({ handle: 7, count: 10, built: true, ms: 3, hasRgb: false, min: [0, 0, 0], max: [1, 1, 1] });
    },
    snap: (scans, ask) => ({ front: new Float64Array([0]), near: new Float64Array([1, 2]), tried: 5, cells: 1, ms: 0.1, asked: [scans, ask] }),
  };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

test("scans opened by token; a page told, then answered by token only", async () => {
  const parent = new EventEmitter();
  parent.sent = [];
  parent.postMessage = (m) => parent.sent.push(m);
  const engine = fakeEngine();
  createHost({ engine, parentPort: parent, every: () => 0, stop: () => {} });
  const page = port();
  parent.emit("message", { data: { kind: "page" }, ports: [page] });
  assert.deepEqual(page.out[0], { kind: "hello", version: "0.1.0", scans: [] });
  parent.emit("message", { data: { kind: "open", token: "t1", path: "C:\\a.las", cacheDir: "C:\\c" } });
  parent.emit("message", { data: { kind: "open", token: "t2", path: "C:\\bad.las", cacheDir: "C:\\c" } });
  // Asked before it's open: missing.
  page.emit("message", { data: { kind: "snap", id: 1, scans: [{ token: "t1", affine: [] }], ask: {} } });
  assert.deepEqual(page.out[1], { kind: "snap", id: 1, missing: ["t1"] });
  await tick();
  assert.equal(page.out[2].kind, "opened");
  assert.equal(page.out[2].token, "t1");
  assert.equal(page.out[2].info.count, 10);
  assert.deepEqual(page.out[3], { kind: "failed", token: "t2", error: "not a LAS file" });
  // 1.25: the main process told each scan settled.
  assert.deepEqual(parent.sent, [
    { kind: "opened", token: "t1" },
    { kind: "failed", token: "t2" },
  ]);
  page.emit("message", { data: { kind: "snap", id: 2, scans: [{ token: "t1", affine: [1] }], ask: { reach: 8 } } });
  assert.equal(page.out[4].kind, "snap");
  assert.equal(page.out[4].id, 2);
  assert.deepEqual([...page.out[4].near], [1, 2]);
  // A token never given: missing, nothing asked of the engine.
  page.emit("message", { data: { kind: "snap", id: 3, scans: [{ token: "nope" }], ask: {} } });
  assert.deepEqual(page.out[5], { kind: "snap", id: 3, missing: ["nope"] });
  page.emit("message", { data: { kind: "status", id: 4 } });
  assert.deepEqual(
    page.out[6].scans.map((s) => [s.token, s.state]),
    [
      ["t1", "open"],
      ["t2", "failed"],
    ],
  );
  // A later page hears what's open.
  const later = port();
  parent.emit("message", { data: { kind: "page" }, ports: [later] });
  assert.equal(later.out[0].scans[0].state, "open");
});

const addon = path.join(__dirname, "..", "engine", "dist", "cadp-engine.node");

test("the real engine: a scan written here, indexed, asked of, opened again", { skip: !fs.existsSync(addon) && "the engine's add-on isn't built (npm run build:engine)" }, async () => {
  const engine = require(addon);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "cadp-engine-host-"));
  const scan = path.join(dir, "site.las");
  fs.writeFileSync(scan, lasBytes(samplePoints([6000000, 2000000, 0]), { offset: [6000000, 2000000, 0] }));
  const run = async (token) => {
    const parent = new EventEmitter();
    createHost({ engine, parentPort: parent });
    const page = port();
    parent.emit("message", { data: { kind: "page" }, ports: [page] });
    parent.emit("message", { data: { kind: "open", token, path: scan, cacheDir: path.join(dir, "cache") } });
    for (let i = 0; i < 200 && !page.out.some((m) => m.kind === "opened" || m.kind === "failed"); i += 1) await new Promise((r) => setTimeout(r, 10));
    const opened = page.out.find((m) => m.kind === "opened");
    assert.ok(opened, JSON.stringify(page.out));
    page.emit("message", {
      data: {
        kind: "snap",
        id: 1,
        scans: [{ token, affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0] }],
        ask: { target: [6000005, 2000005, 0], right: [1, 0, 0], up: [0, 1, 0], toEye: [0, 0, 1], unitsPerPixel: 0.01, halfW: 200, halfH: 150, cursor: [200, 150], reach: 8, frontPx: 2, window: 0, most: 32, frontMost: 8 },
      },
    });
    const a = page.out.find((m) => m.kind === "snap");
    assert.equal(a.error, undefined);
    // The post's top first from above: z 104, class 5.
    assert.ok(Math.abs(a.front[4] - 104) < 1e-6, String(a.front[4]));
    assert.equal(a.front[7], 5);
    assert.ok(a.near.length / 14 >= 1);
    // 1.20: the floor's points in a box, every one; then thinned.
    page.emit("message", {
      data: { kind: "region", id: 2, token, affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0], min: [6000000.95, 2000000.95, 99], max: [6000002.05, 2000002.05, 100.05], most: 100000 },
    });
    const r = page.out.find((m) => m.kind === "region");
    assert.equal(r.error, undefined);
    assert.equal(r.count, 11 * 11);
    assert.equal(r.complete, true);
    assert.equal(r.xyz.length, r.count * 3);
    assert.ok([...r.classification].every((c) => c === 2));
    page.emit("message", {
      data: { kind: "region", id: 3, token, affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0], min: [6000000, 2000000, 0], max: [6000010, 2000010, 200], most: 1000 },
    });
    const thin = page.out.filter((m) => m.kind === "region")[1];
    assert.equal(thin.complete, false);
    assert.ok(thin.count >= 1000 && thin.count < 2000, String(thin.count));
    return opened.info;
  };
  const first = await run("t1");
  assert.equal(first.built, true);
  assert.equal(first.count, samplePoints().length);
  const again = await run("t2");
  assert.equal(again.built, false);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("1.22: cloud to cloud asked with the page's points, answered by id when the engine's done, errors said", async () => {
  const parent = new EventEmitter();
  let finish;
  const engine = {
    ...fakeEngine(),
    alignClouds: (fixed, moving, ask) =>
      new Promise((resolve) => {
        finish = () => resolve({ align: ask.fit, before: { rms: 1 }, after: { rms: 0.1 }, iterations: 4, refined: true, ms: 2, sizes: [fixed.length, moving.length] });
      }),
    checkClouds: (scans) => {
      if (scans.length < 2) throw new Error("two scans or more");
      return Promise.resolve({ samples: scans, pairs: [], gaps: scans.map((s) => new Float32Array(s.length / 3)), ms: 1 });
    },
  };
  createHost({ engine, parentPort: parent, every: () => 0, stop: () => {} });
  const page = port();
  parent.emit("message", { data: { kind: "page" }, ports: [page] });
  const fit = [1, 0, 0, 0.5, 0, 1, 0, 0, 0, 0, 1, 0];
  page.emit("message", { data: { kind: "align", id: 9, fixed: new Float64Array(6), moving: new Float64Array(3), ask: { fit, fitRms: 0.02, level: false, refine: true, foot: 1, tolerance: 0.05 } } });
  // A snap's answered while the align works.
  parent.emit("message", { data: { kind: "open", token: "t", path: "C:\\a.las", cacheDir: "C:\\c" } });
  await tick();
  page.emit("message", { data: { kind: "snap", id: 10, scans: [{ token: "t", affine: [1] }], ask: {} } });
  assert.equal(page.out.at(-1).id, 10);
  assert.equal(page.out.some((m) => m.kind === "align"), false);
  finish();
  await tick();
  const a = page.out.find((m) => m.kind === "align");
  assert.equal(a.id, 9);
  assert.deepEqual(a.align, fit);
  assert.deepEqual(a.sizes, [6, 3]);
  page.emit("message", { data: { kind: "check", id: 11, scans: [new Float64Array(3)], foot: 1, tolerance: 0.05 } });
  await tick();
  assert.deepEqual(page.out.find((m) => m.kind === "check"), { kind: "check", id: 11, error: "two scans or more" });
  page.emit("message", { data: { kind: "check", id: 12, scans: [new Float64Array(3), new Float64Array(6)], foot: 1, tolerance: 0.05 } });
  await tick();
  const c = page.out.filter((m) => m.kind === "check")[1];
  assert.equal(c.id, 12);
  assert.deepEqual(c.gaps.map((g) => g.length), [1, 2]);
});

test("1.22: the real engine lines up two scans and measures them", { skip: !fs.existsSync(addon) && "the engine's add-on isn't built (npm run build:engine)" }, async () => {
  const engine = require(addon);
  let seed = 9;
  const rnd = () => (seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0) / 4294967296;
  const scene = (n) => {
    const out = new Float64Array(n * 3);
    for (let i = 0; i < n; i += 1) {
      const k = rnd();
      const p = k < 0.5 ? [rnd() * 30, rnd() * 30, 100] : k < 0.75 ? [0, rnd() * 30, 100 + rnd() * 8] : [rnd() * 30, 0, 100 + rnd() * 8];
      for (let a = 0; a < 3; a += 1) out[i * 3 + a] = p[a] + (rnd() - 0.5) * 0.01;
    }
    return out;
  };
  const fixed = scene(30000);
  const moving = scene(30000).map((v, i) => v + (i % 3 === 0 ? -0.2 : 0));
  const parent = new EventEmitter();
  createHost({ engine, parentPort: parent });
  const page = port();
  parent.emit("message", { data: { kind: "page" }, ports: [page] });
  page.emit("message", { data: { kind: "align", id: 1, fixed, moving, ask: { fit: [1, 0, 0, 0.17, 0, 1, 0, 0, 0, 0, 1, 0], fitRms: 0.03, level: true, refine: true, foot: 1, tolerance: 0.05 } } });
  page.emit("message", { data: { kind: "check", id: 2, scans: [fixed, fixed], foot: 1, tolerance: 0.05 } });
  for (let i = 0; i < 1000 && page.out.filter((m) => m.kind === "align" || m.kind === "check").length < 2; i += 1) await new Promise((r) => setTimeout(r, 10));
  const a = page.out.find((m) => m.kind === "align");
  assert.equal(a.error, undefined);
  assert.ok(Math.abs(a.align[3] - 0.2) < 0.005, String(a.align[3]));
  assert.ok(a.after.rms < a.before.rms);
  const c = page.out.find((m) => m.kind === "check");
  assert.equal(c.error, undefined);
  assert.equal(c.pairs.length, 1);
  assert.ok(c.pairs[0].ab instanceof Float32Array);
  assert.equal(c.gaps[0].length, 30000);
});

test("1.23: Geo W. asked with the page's points and model, how far it's come said, answered by id", async () => {
  const parent = new EventEmitter();
  let finish;
  let progress = 0;
  const timers = [];
  const engine = {
    ...fakeEngine(),
    netProgress: () => progress,
    netClassify: (model, _blob, points) =>
      new Promise((resolve, reject) => {
        if (model.classes < 1) reject(new Error("no classes"));
        finish = () => resolve({ cls: new Uint8Array(points.px.length).fill(2), sure: new Float32Array(points.px.length).fill(0.9), cubes: 1, ms: 3 });
      }),
  };
  createHost({ engine, parentPort: parent, every: (f) => (timers.push(f), timers.length), stop: () => {} });
  const page = port();
  parent.emit("message", { data: { kind: "page" }, ports: [page] });
  const points = { px: new Float64Array(3), py: new Float64Array(3), pz: new Float64Array(3), intensity: new Float64Array(3) };
  page.emit("message", { data: { kind: "classify", id: 5, model: { classes: 9 }, blob: new Float32Array(4), points } });
  progress = 0.4;
  timers.at(-1)();
  assert.deepEqual(page.out.at(-1), { kind: "classify-progress", id: 5, done: 0.4 });
  timers.at(-1)();
  assert.equal(page.out.filter((m) => m.kind === "classify-progress").length, 1);
  finish();
  await tick();
  const done = page.out.find((m) => m.kind === "classify");
  assert.equal(done.id, 5);
  assert.deepEqual([...done.cls], [2, 2, 2]);
  page.emit("message", { data: { kind: "classify", id: 6, model: { classes: 0 }, blob: new Float32Array(4), points } });
  await tick();
  assert.deepEqual(page.out.filter((m) => m.kind === "classify")[1], { kind: "classify", id: 6, error: "no classes" });
});

test("1.23: the real engine runs a Geo W. model over points", { skip: !fs.existsSync(addon) && "the engine's add-on isn't built (npm run build:engine)" }, async () => {
  const engine = require(addon);
  // A version 1 model: two layers a branch, two in the head, nine classes; weights small and steady.
  let floats = 0;
  const layer = (i, o) => {
    const L = { inp: i, out: o, w: floats, b: floats + i * o };
    floats += i * o + o;
    return L;
  };
  const branches = [8, 4, 4].map((c) => [layer(c, 8), layer(8, 8)]);
  const head = [layer(8 * 3 + 2, 8), layer(8, 9)];
  const blob = new Float32Array(floats).map((_, i) => Math.sin(i) * 0.3);
  const n = 5000;
  const px = new Float64Array(n).map((_, i) => (i % 70) * 0.3);
  const py = new Float64Array(n).map((_, i) => Math.floor(i / 70) * 0.3);
  const pz = new Float64Array(n).map((_, i) => 100 + (i % 7) * 0.05);
  const intensity = new Float64Array(n).fill(0.5);
  const parent = new EventEmitter();
  createHost({ engine, parentPort: parent });
  const page = port();
  parent.emit("message", { data: { kind: "page" }, ports: [page] });
  page.emit("message", { data: { kind: "classify", id: 1, model: { branches, head, classes: 9, inputs: [] }, blob, points: { px, py, pz, intensity } } });
  for (let i = 0; i < 1000 && !page.out.some((m) => m.kind === "classify"); i += 1) await new Promise((r) => setTimeout(r, 10));
  const r = page.out.find((m) => m.kind === "classify");
  assert.equal(r.error, undefined);
  assert.equal(r.cls.length, n);
  assert.ok(r.cubes > 100 && r.cubes <= n, String(r.cubes));
  assert.ok([...r.sure].every((v) => v > 0 && v <= 1));
  assert.ok([...r.cls].every((c) => c < 9));
});

test("1.23: a question's big lists put back together from their parts first", async () => {
  const parent = new EventEmitter();
  let got;
  const engine = { ...fakeEngine(), checkClouds: (scans, foot) => ((got = { scans, foot }), Promise.resolve({ samples: [], pairs: [], gaps: [], ms: 0 })) };
  createHost({ engine, parentPort: parent, every: () => 0, stop: () => {} });
  const page = port();
  parent.emit("message", { data: { kind: "page" }, ports: [page] });
  const big = new Float64Array([1.5, 2.5, 3.5, 4.5]);
  const bytes = new Uint8Array(big.buffer);
  page.emit("message", { data: { kind: "part", id: 7, part: 0, at: 16, total: 32, bytes: bytes.slice(16) } });
  page.emit("message", { data: { kind: "part", id: 7, part: 0, at: 0, total: 32, bytes: bytes.slice(0, 16) } });
  page.emit("message", { data: { kind: "check", id: 7, scans: [{ __part: 0, type: "Float64Array", length: 4 }, new Float64Array([9])], foot: 1, tolerance: 0.05 } });
  await tick();
  assert.ok(got.scans[0] instanceof Float64Array);
  assert.deepEqual([...got.scans[0]], [1.5, 2.5, 3.5, 4.5]);
  assert.deepEqual([...got.scans[1]], [9]);
  // Parts aren't questions: nothing said back for them.
  assert.equal(page.out.filter((m) => m.kind === "part").length, 0);
  assert.equal(page.out.find((m) => m.kind === "check").id, 7);
});

test("1.24: the real engine opens an E57 of two setups, asked of each alone", { skip: !fs.existsSync(addon) && "the engine's add-on isn't built (npm run build:engine)" }, async () => {
  const engine = require(addon);
  const { e57Bytes, sampleSetups } = require("../src/e57-sample");
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "cadp-engine-e57-"));
  const scan = path.join(dir, "site.e57");
  fs.writeFileSync(scan, e57Bytes(sampleSetups()));
  const parent = new EventEmitter();
  createHost({ engine, parentPort: parent });
  const page = port();
  parent.emit("message", { data: { kind: "page" }, ports: [page] });
  parent.emit("message", { data: { kind: "open", token: "e", path: scan, cacheDir: path.join(dir, "cache") } });
  for (let i = 0; i < 500 && !page.out.some((m) => m.kind === "opened" || m.kind === "failed"); i += 1) await new Promise((r) => setTimeout(r, 10));
  const opened = page.out.find((m) => m.kind === "opened");
  assert.ok(opened, JSON.stringify(page.out.at(-1)));
  assert.equal(opened.info.count, 5202);
  assert.equal(opened.info.setups, true);
  assert.equal(opened.info.e57, true);
  const all = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0];
  const box = { min: [1915990, 639990, -1000], max: [1916050, 640010, 1000], most: 100000 };
  page.emit("message", { data: { kind: "region", id: 1, token: "e", affine: all, ...box } });
  page.emit("message", { data: { kind: "region", id: 2, token: "e", affine: all, ...box, setup: 1 } });
  const [every, second] = page.out.filter((m) => m.kind === "region");
  assert.equal(every.count, 5202);
  assert.ok(every.setupOf instanceof Uint16Array);
  assert.deepEqual([...new Set(every.setupOf)].sort(), [0, 1]);
  assert.equal(second.count, 2601);
  assert.ok([...second.setupOf].every((k) => k === 1));
  // Snapped to from above over the second setup: its floor, at 200; the first alone: nothing there.
  const ask = { target: [1916032.5, 640002.5, 0], right: [1, 0, 0], up: [0, 1, 0], toEye: [0, 0, 1], unitsPerPixel: 0.01, halfW: 200, halfH: 150, cursor: [200, 150], reach: 8, frontPx: 2, window: 0, most: 32, frontMost: 8 };
  page.emit("message", { data: { kind: "snap", id: 3, scans: [{ token: "e", affine: all }], ask } });
  page.emit("message", { data: { kind: "snap", id: 4, scans: [{ token: "e", affine: all, setup: 0 }], ask } });
  const [both, first] = page.out.filter((m) => m.kind === "snap");
  assert.ok(Math.abs(both.front[4] - 200) < 1e-3, String(both.front[4]));
  assert.equal(first.front.length, 0);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("1.28: a big region answer goes in parts the page puts back together", () => {
  const { postInParts, PART_BYTES } = require("../src/engine-host");
  const page = port();
  const n = Math.ceil((PART_BYTES * 2.5) / 12);
  const xyz = new Float32Array(n * 3).map((_, i) => i % 1000);
  const small = new Uint8Array(10).fill(3);
  const parts = postInParts(page, { kind: "region", id: 9, count: n, xyz, classification: small });
  assert.equal(parts, 1);
  const pieces = page.out.filter((m) => m.kind === "part");
  assert.equal(pieces.length, 3);
  assert.ok(pieces.every((m) => m.id === 9 && m.bytes.length <= PART_BYTES));
  const last = page.out.at(-1);
  assert.deepEqual(last.xyz, { __part: 0, type: "Float32Array", length: n * 3 });
  assert.equal(last.classification, small);
  // Put back together as the page does.
  const buf = new Uint8Array(pieces[0].total);
  for (const m of pieces) buf.set(m.bytes, m.at);
  const back = new Float32Array(buf.buffer, 0, last.xyz.length);
  assert.deepEqual(back.subarray(0, 2000), xyz.subarray(0, 2000));
  assert.equal(back[back.length - 1], xyz[xyz.length - 1]);
});

test("1.28: a region asked with parts and small enough comes whole", async () => {
  const parent = new EventEmitter();
  const engine = {
    ...fakeEngine(),
    region: () => ({ origin: [0, 0, 0], count: 1, xyz: new Float32Array(3), intensity: new Uint16Array(1), classification: new Uint8Array(1), flags: new Uint8Array(1), returns: new Uint8Array(1), complete: true, tried: 1, ms: 0 }),
  };
  createHost({ engine, parentPort: parent, every: () => 0, stop: () => {} });
  const page = port();
  parent.emit("message", { data: { kind: "page" }, ports: [page] });
  parent.emit("message", { data: { kind: "open", token: "t", path: "C:\\a.las", cacheDir: "C:\\c" } });
  await tick();
  page.emit("message", { data: { kind: "region", id: 4, token: "t", affine: [], min: [], max: [], most: 9, parts: true } });
  const r = page.out.find((m) => m.kind === "region");
  assert.equal(r.id, 4);
  assert.equal(r.count, 1);
  assert.ok(r.xyz instanceof Float32Array);
  assert.equal(page.out.filter((m) => m.kind === "part").length, 0);
});

test("1.28: the real engine cuts a scan to a limit box, every point, into a file for the main process", { skip: !fs.existsSync(addon) && "the engine's add-on isn't built (npm run build:engine)" }, async () => {
  const engine = require(addon);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "cadp-engine-crop-"));
  const scan = path.join(dir, "site.las");
  fs.writeFileSync(scan, lasBytes(samplePoints([6000000, 2000000, 0]), { offset: [6000000, 2000000, 0] }));
  const parent = new EventEmitter();
  parent.sent = [];
  parent.postMessage = (m) => parent.sent.push(m);
  createHost({ engine, parentPort: parent });
  // Asked before it's open: waited for.
  parent.emit("message", { data: { kind: "open", token: "s", path: scan, cacheDir: path.join(dir, "cache") } });
  const out = path.join(dir, "site cut.las.saving");
  const box = { min: [6000000.95, 2000000.95, -1e9], max: [6000002.05, 2000002.05, 1e9], centre: [6000001.5, 2000001.5], cos: 1, sin: 0, turned: false, outside: false, slack: [1e-6, 1e-6, 1e-6] };
  const ask = { affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0], boxes: [box], scale: [0.001, 0.001, 0.001], offset: [6000000, 2000000, 0], wkt: 'LOCAL_CS["Feet"]', laz: false };
  parent.emit("message", { data: { kind: "crop", id: 3, token: "s", out, ask } });
  for (let i = 0; i < 500 && !parent.sent.some((m) => m.kind === "crop-done"); i += 1) await new Promise((r) => setTimeout(r, 10));
  const done = parent.sent.find((m) => m.kind === "crop-done");
  assert.ok(done && !done.error, JSON.stringify(done));
  assert.equal(done.id, 3);
  // 1.0 to 2.0 each way on the floor's 0.1 grid: 11 × 11.
  assert.equal(done.kept, 121);
  assert.equal(done.processed, 10040);
  assert.ok(fs.statSync(out).size === done.bytes && done.bytes >= 375 + 121 * 30);
  const bytes = fs.readFileSync(out);
  assert.equal(bytes.toString("ascii", 0, 4), "LASF");
  assert.equal(bytes.readUInt8(104), 6);
  assert.equal(Number(bytes.readBigUInt64LE(247)), 121);
  // A scan not open: said.
  parent.emit("message", { data: { kind: "crop", id: 4, token: "nope", out, ask } });
  await tick();
  assert.match(parent.sent.find((m) => m.id === 4).error, /isn't open/);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("1.28: the real engine renders a scene's passes for a page, and stops when asked", { skip: !fs.existsSync(addon) && "the engine's add-on isn't built (npm run build:engine)" }, async () => {
  const engine = require(addon);
  const parent = new EventEmitter();
  createHost({ engine, parentPort: parent });
  const page = port();
  parent.emit("message", { data: { kind: "page" }, ports: [page] });
  const s = 50;
  const ground = { positions: new Float64Array([-s, 0, -s, s, 0, -s, s, 0, s, -s, 0, s]), indices: new Uint32Array([0, 2, 1, 0, 3, 2]), color: [0.5, 0.5, 0.5], glass: false };
  const ask = { position: [0, 5, 20], target: [0, 0, 0], fov: 50, width: 24, height: 16, sun: [0.3, 1, 0.2], lights: [], sunOff: false, size: 100, seed: 1 };
  page.emit("message", { data: { kind: "render", id: 1, meshes: [ground], ask, passes: 3 } });
  for (let i = 0; i < 300 && !page.out.some((m) => m.kind === "render"); i += 1) await new Promise((r) => setTimeout(r, 10));
  const done = page.out.find((m) => m.kind === "render");
  assert.ok(done && !done.error, JSON.stringify(done));
  assert.equal(done.passes, 3);
  const last = page.out.filter((m) => m.kind === "render-progress").at(-1);
  assert.equal(last.done, true);
  assert.ok(last.sum instanceof Float32Array && last.sum.length === 24 * 16 * 3);
  assert.ok(last.sum.some((v) => v > 0));
  // A long one stopped.
  page.emit("message", { data: { kind: "render", id: 2, meshes: [ground], ask, passes: 5000 } });
  page.emit("message", { data: { kind: "render-stop", id: 2 } });
  for (let i = 0; i < 300 && !page.out.some((m) => m.kind === "render" && m.id === 2); i += 1) await new Promise((r) => setTimeout(r, 10));
  const stopped = page.out.find((m) => m.kind === "render" && m.id === 2);
  assert.equal(stopped.stopped, true);
  assert.ok(stopped.passes < 5000);
});

test("1.29: the real engine does the classifier's middle for a page, its big lists in parts", { skip: !fs.existsSync(addon) && "the engine's add-on isn't built (npm run build:engine)" }, async () => {
  const engine = require(addon);
  const parent = new EventEmitter();
  createHost({ engine, parentPort: parent });
  const page = port();
  parent.emit("message", { data: { kind: "page" }, ports: [page] });
  // A floor 20 ft square at 100 (0.5 apart), a wall 6 ft up along one side.
  const pts = [];
  for (let i = 0; i < 40; i += 1) for (let j = 0; j < 40; j += 1) pts.push(i * 0.5, j * 0.5, 100);
  for (let i = 0; i < 40; i += 1) for (let z = 2; z < 12; z += 1) pts.push(i * 0.5, 19.5, 100 + z * 0.5);
  const xyz = new Float32Array(pts);
  const n = xyz.length / 3;
  // One tree of one leaf: everything's a building (2).
  const ask = {
    clouds: [{ origin: [0, 0, 0], xyz }],
    x0: 0, y0: 0, y1: 19.5, zMin: 100, zMax: 105.5, k: 1, r: 1.5, v: 0.75, rise: 0.5,
    gz: new Float64Array([100, 100, 100, 100]), cols: 2, rows: 2, g: 10,
    groundGreen: 0, coloured: false,
    treeF: new Float64Array([-2]), treeT: new Float64Array([0]), treeL: new Float64Array([-1]), treeR: new Float64Array([-1]), treeC: new Float64Array([2]), treeStarts: [0, 1],
  };
  page.emit("message", { data: { kind: "classify-stage", id: 5, ask } });
  for (let i = 0; i < 300 && !page.out.some((m) => m.kind === "classify-stage"); i += 1) await new Promise((r) => setTimeout(r, 10));
  const got = page.out.find((m) => m.kind === "classify-stage");
  assert.ok(got && !got.error, JSON.stringify(got && got.error));
  assert.equal(got.id, 5);
  // The floor on the ground (2), the wall in voxels, all the forest's building, one object.
  const ground = got.ground[0];
  assert.equal(ground.length, n);
  assert.equal(ground.filter((c) => c === 2).length, 1600);
  assert.ok(got.vox[0] instanceof Int32Array);
  assert.ok(got.n > 0 && got.label.every((l) => l === 2));
  assert.equal(got.starts.length, 1);
  assert.equal(got.members.length, got.n);
  // A page that sends nonsense hears why.
  page.emit("message", { data: { kind: "classify-stage", id: 6, ask: { ...ask, cols: 3 } } });
  await tick();
  assert.match(page.out.find((m) => m.id === 6).error, /ground grid/);
});

test("1.35: the engine's own window is started in the app's frame, shown a view by token, and says how it's going", async () => {
  const parent = new EventEmitter();
  parent.sent = [];
  parent.postMessage = (m) => parent.sent.push(m);
  const calls = [];
  const engine = {
    ...fakeEngine(),
    viewStart: (hwnd) => calls.push(["start", hwnd]),
    viewShow: (ask) => {
      calls.push(["show", ask]);
      return ask.scans.length;
    },
    viewStats: () => ({ started: true, running: true, frames: 12, points: 10, adapter: "A card" }),
    viewStop: () => calls.push(["stop"]),
    viewShapes: (...args) => {
      calls.push(["shapes", args]);
      return true;
    },
    viewBorrow: (...args) => {
      calls.push(["borrow", args]);
      // Told only their classes the first time: their places are needed too.
      return args[5] !== null || calls.filter((c) => c[0] === "borrow").length > 1;
    },
    viewTint: (...args) => {
      calls.push(["tint", args]);
      return args[5] !== null;
    },
    viewStrokes: (...args) => {
      calls.push(["strokes", args]);
      // No more than the card tests at once.
      return args[1].length <= 8;
    },
    viewLineStrips: (...args) => {
      calls.push(["line strips", args]);
      return true;
    },
    viewLines: (...args) => {
      calls.push(["lines", args]);
      // A batch it wasn't given: said, the list not taken.
      const missing = args[1].map((i) => i.id).filter((id) => id > 100);
      return { ok: missing.length === 0, missing };
    },
  };
  createHost({ engine, parentPort: parent, every: () => 0, stop: () => {} });
  const page = port();
  parent.emit("message", { data: { kind: "page" }, ports: [page] });
  parent.emit("message", { data: { kind: "open", token: "t1", path: "C:\\a.las", cacheDir: "C:\\c" } });
  await tick();
  // Asked before the window's started: said so.
  page.emit("message", { data: { kind: "view", id: 1, ask: { test: true } } });
  assert.equal(page.out.at(-1).error, "The engine's window isn't started.");
  parent.emit("message", { data: { kind: "view-start", hwnd: "66052" } });
  assert.deepEqual(calls[0], ["start", "66052"]);
  assert.deepEqual(parent.sent.at(-1), { kind: "view-started", started: true });
  // A view: its scans by their tokens (one not open is left out), the numbers as given.
  page.emit("message", {
    data: {
      kind: "view",
      id: 2,
      scans: [
        { token: "t1", affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0], most: 5000000, allow: [-129, -1, -1, -1, -1, -1, -1, -1], setup: 2, off: [0, 5] },
        { token: "nope", affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0] },
      ],
      ask: {
        target: [1, 2, 3],
        right: [1, 0, 0],
        up: [0, 1, 0],
        toEye: [0, 0, 1],
        unitsPerPixel: 0.5,
        rect: [10, 20, 300, 200],
        px: 2,
        clear: [0.2, 0.1, 0.3],
        hide: true,
        serial: 41,
      },
    },
  });
  const shown = calls.at(-1)[1];
  // The page's background, its scans kept and not drawn, and its number for the ask.
  assert.deepEqual(shown.clear, [0.2, 0.1, 0.3]);
  assert.equal(shown.hide, true);
  assert.equal(shown.serial, 41);
  // Which of its points too: the classes shown (bits, as unsigned numbers), a setup alone, setups off.
  assert.deepEqual(shown.scans, [
    { handle: 7, affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0], most: 5000000, allow: [4294967167, 4294967295, 4294967295, 4294967295, 4294967295, 4294967295, 4294967295, 4294967295], setup: 2, off: [0, 5] },
  ]);
  assert.deepEqual(shown.target, [1, 2, 3]);
  assert.deepEqual(shown.toEye, [0, 0, 1]);
  assert.equal(shown.unitsPerPixel, 0.5);
  assert.deepEqual(shown.rect, [10, 20, 300, 200]);
  assert.equal(shown.test, false);
  assert.deepEqual(page.out.at(-1), { kind: "view", id: 2, shown: 1, stats: { started: true, running: true, frames: 12, points: 10, adapter: "A card" } });
  // 1.36: a scan's look, its class map and the page's own classes for it go through as the engine reads them; eye-dome lighting and the hold with the view.
  const classes = Array.from({ length: 256 }, (_, c) => (c | (255 << 24)) >>> 0);
  page.emit("message", {
    data: {
      kind: "view",
      id: 20,
      scans: [
        {
          token: "t1",
          affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0],
          most: 7,
          map: new Uint8Array(256).fill(3),
          borrow: "its classes",
          look: { by: 1, px: 3, round: true, one: [1, 0.5, 0], heights: [10, 20], intensity: [5, 900], mix: [8, 800, 0.6], classes, shade: [60, 55, 50], fill: 12, junk: "left out" },
        },
        { token: "t1", affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0], look: { by: -1, px: 0, classes: [1, 2], shade: [60, 55] }, map: [1, 2] },
      ],
      ask: { target: [1, 2, 3], right: [1, 0, 0], up: [0, 1, 0], toEye: [0, 0, 1], unitsPerPixel: 0.5, rect: [0, 0, 300, 200], edl: [1, 1.4, 16.5, 0.0066], holdMs: 16.6, shade: [0.14, 0, 0.3] },
    },
  });
  const looked = calls.at(-1)[1];
  assert.deepEqual(looked.scans[0], {
    handle: 7,
    affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0],
    most: 7,
    map: new Array(256).fill(3),
    borrow: "its classes",
    // 1.37: its shading from the shape of its surface (lighting, detail, hollows).
    // And Fill gaps: the most a point grows, pixels.
    look: { by: 1, px: 3, round: true, one: [1, 0.5, 0], heights: [10, 20], intensity: [5, 900], mix: [8, 800, 0.6], classes, shade: [60, 55, 50], fill: 12 },
  });
  // What isn't as the engine reads it is left out (it draws by what it's given, or as 1.35 did).
  assert.deepEqual(looked.scans[1], { handle: 7, affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0], look: { round: false } });
  assert.deepEqual(looked.edl, [1, 1.4, 16.5, 0.0066]);
  assert.equal(looked.holdMs, 16.6);
  // 1.37: depth shading, relief and face shading with the view.
  assert.deepEqual(looked.shade, [0.14, 0, 0.3]);
  // The page's own classes for a scan: their classes alone first (the engine says their places are needed), then with their places.
  const cls = new Uint8Array([2, 6]);
  const gone = new Uint8Array([0, 1]);
  page.emit("message", { data: { kind: "view-classes", id: 21, token: "t1", key: "k1", pointsKey: "p1", cell: 0.4, origin: [100, 200, 3], classification: cls, removed: gone } });
  assert.deepEqual(page.out.at(-1), { kind: "view-classes", id: 21, key: "k1", needPoints: true });
  assert.deepEqual(calls.at(-1), ["borrow", [7, "k1", "p1", 0.4, [100, 200, 3], null, cls, gone]]);
  const xyz = new Float32Array([0, 0, 0, 1, 1, 1]);
  page.emit("message", { data: { kind: "view-classes", id: 22, token: "t1", key: "k1", pointsKey: "p1", cell: 0.4, origin: [100, 200, 3], xyz, classification: cls, removed: null } });
  assert.deepEqual(page.out.at(-1), { kind: "view-classes", id: 22, key: "k1", ok: true });
  assert.deepEqual(calls.at(-1), ["borrow", [7, "k1", "p1", 0.4, [100, 200, 3], xyz, cls, null]]);
  // 1.37: the limit boxes on, the slice's slab and the isolated shapes' key go with a view, as the engine reads them.
  const box = { min: [0, 0, 0], max: [10, 4, 3], centre: [5, 2], cos: 1, sin: 0, turned: false, outside: false, slack: [1e-6, 1e-6, 1e-6], junk: 1 };
  const slab = { min: [-1e12, -1e12, 1], max: [1e12, 1e12, 1.1], centre: [0, 0], cos: 0.5, sin: 0.5, turned: true, outside: false, slack: [1e-6, 1e-6, 1e-6] };
  const boxed = { target: [1, 2, 3], right: [1, 0, 0], up: [0, 1, 0], toEye: [0, 0, 1], unitsPerPixel: 0.5, rect: [0, 0, 300, 200] };
  page.emit("message", { data: { kind: "view", id: 30, scans: [{ token: "t1", affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0] }], ask: { ...boxed, boxes: [box], slice: slab, isolate: "its shapes" } } });
  const clipped = calls.at(-1)[1];
  assert.deepEqual(clipped.boxes, [{ min: [0, 0, 0], max: [10, 4, 3], centre: [5, 2], cos: 1, sin: 0, turned: false, outside: false, slack: [1e-6, 1e-6, 1e-6] }]);
  assert.deepEqual(clipped.slice, slab);
  assert.equal("strength" in clipped, false);
  assert.equal(clipped.isolate, "its shapes");
  // None: none said. Scans shown faint: at 0.3 of their strength.
  page.emit("message", { data: { kind: "view", id: 31, scans: [], ask: { ...boxed, boxes: [], strength: 0.3 } } });
  assert.equal(calls.at(-1)[1].strength, 0.3);
  assert.equal("slot" in calls.at(-1)[1], false);
  // The split view's other side: the frame's second view.
  page.emit("message", { data: { kind: "view", id: 36, scans: [], ask: { ...boxed, slot: 1 } } });
  assert.equal(calls.at(-1)[1].slot, 1);
  page.emit("message", { data: { kind: "view", id: 37, scans: [], ask: { ...boxed, slot: 7 } } });
  assert.equal("slot" in calls.at(-1)[1], false);
  assert.equal("boxes" in calls.at(-1)[1] || "slice" in calls.at(-1)[1] || "isolate" in calls.at(-1)[1], false);
  // A box not as the engine reads it: said, and nothing shown without it.
  const shows = calls.filter((c) => c[0] === "show").length;
  page.emit("message", { data: { kind: "view", id: 32, scans: [], ask: { ...boxed, boxes: [box, { min: [0, 0], max: [1, 1, 1] }] } } });
  assert.match(page.out.at(-1).error, /limit boxes weren't as the engine reads them/);
  page.emit("message", { data: { kind: "view", id: 33, scans: [], ask: { ...boxed, slice: { min: [0, 0, 0] } } } });
  assert.match(page.out.at(-1).error, /limit boxes weren't as the engine reads them/);
  assert.equal(calls.filter((c) => c[0] === "show").length, shows);
  // The isolated shapes, given once under their key.
  const poly = new Float64Array([0, 0, 10, 0, 10, 10]);
  page.emit("message", {
    data: {
      kind: "view-shapes",
      id: 34,
      key: "its shapes",
      shapes: [
        { right: [1, 0, 0], up: [0, 1, 0], poly },
        { right: [1, 0, 0], up: [0, 0, 1], poly: [0, 0, 1, 0, 1, 1], op: "out", dabs: [1, 1, 0.5], near: [3], off: [7, 18, 999, -1], junk: true },
      ],
    },
  });
  assert.deepEqual(page.out.at(-1), { kind: "view-shapes", id: 34, key: "its shapes", ok: true });
  assert.equal(calls.at(-1)[0], "shapes");
  assert.equal(calls.at(-1)[1][0], "its shapes");
  assert.deepEqual(calls.at(-1)[1][1], [
    { right: [1, 0, 0], up: [0, 1, 0], poly },
    { right: [1, 0, 0], up: [0, 0, 1], poly: new Float64Array([0, 0, 1, 0, 1, 1]), op: "out", dabs: new Float64Array([1, 1, 0.5]), near: new Float64Array([3]), off: [7, 18] },
  ]);
  page.emit("message", { data: { kind: "view-shapes", id: 35, key: "k", shapes: [{ right: [1, 0], up: [0, 1, 0], poly }] } });
  assert.match(page.out.at(-1).error, /isolated shapes weren't as the engine reads them/);
  // A scan that isn't open, and classes not as the engine reads them.
  page.emit("message", { data: { kind: "view-classes", id: 23, token: "nope", key: "k", pointsKey: "p", cell: 1, origin: [0, 0, 0], classification: cls } });
  assert.deepEqual(page.out.at(-1), { kind: "view-classes", id: 23, key: "k", missing: ["nope"] });
  page.emit("message", { data: { kind: "view-classes", id: 24, token: "t1", key: "k", pointsKey: "p", cell: 0, origin: [0, 0, 0], classification: cls } });
  assert.match(page.out.at(-1).error, /weren't as the engine reads them/);
  // 1.39: the page's own colouring for a scan (a colouring the engine hasn't): its colours alone first, then with their places.
  const rgba = new Uint8Array([10, 20, 30, 255, 40, 50, 60, 255]);
  page.emit("message", { data: { kind: "view-colours", id: 40, token: "t1", key: "c1", pointsKey: "p1", cell: 0.4, origin: [100, 200, 3], rgba } });
  assert.deepEqual(page.out.at(-1), { kind: "view-colours", id: 40, key: "c1", needPoints: true });
  assert.deepEqual(calls.at(-1), ["tint", [7, "c1", "p1", 0.4, [100, 200, 3], null, rgba]]);
  page.emit("message", { data: { kind: "view-colours", id: 41, token: "t1", key: "c1", pointsKey: "p1", cell: 0.4, origin: [100, 200, 3], xyz, rgba } });
  assert.deepEqual(page.out.at(-1), { kind: "view-colours", id: 41, key: "c1", ok: true });
  assert.deepEqual(calls.at(-1), ["tint", [7, "c1", "p1", 0.4, [100, 200, 3], xyz, rgba]]);
  page.emit("message", { data: { kind: "view-colours", id: 42, token: "nope", key: "c", pointsKey: "p", cell: 1, origin: [0, 0, 0], rgba } });
  assert.deepEqual(page.out.at(-1), { kind: "view-colours", id: 42, key: "c", missing: ["nope"] });
  page.emit("message", { data: { kind: "view-colours", id: 43, token: "t1", key: "c", pointsKey: "p", cell: 1, origin: [0, 0, 0], rgba: [1, 2, 3, 4] } });
  assert.match(page.out.at(-1).error, /colours weren't as the engine reads them/);
  // The brush's strokes for the card, given under their key as they change.
  page.emit("message", {
    data: {
      kind: "view-strokes",
      id: 44,
      key: "press 3",
      strokes: [
        { op: "class", cls: 6, right: [1, 0, 0], up: [0, 1, 0], dabs: new Float64Array([1, 2, 0.5, 2, 2, 0.5]), off: [7, 999], junk: 1 },
        { op: "out", right: [1, 0, 0], up: [0, 0, 1], dabs: [3, 4, 1], near: [9.5] },
        { op: "keep", right: [1, 0, 0], up: [0, 1, 0], dabs: [] },
      ],
    },
  });
  assert.deepEqual(page.out.at(-1), { kind: "view-strokes", id: 44, key: "press 3", ok: true });
  assert.deepEqual(calls.at(-1), [
    "strokes",
    [
      "press 3",
      [
        { op: "class", cls: 6, right: [1, 0, 0], up: [0, 1, 0], dabs: new Float64Array([1, 2, 0.5, 2, 2, 0.5]), off: [7] },
        { op: "out", right: [1, 0, 0], up: [0, 0, 1], dabs: new Float64Array([3, 4, 1]), near: new Float64Array([9.5]) },
        { op: "keep", right: [1, 0, 0], up: [0, 1, 0], dabs: new Float64Array([]) },
      ],
    ],
  ]);
  // More than the card tests: said, so the page draws the scans itself meanwhile.
  const one = { op: "out", right: [1, 0, 0], up: [0, 1, 0], dabs: [0, 0, 1] };
  page.emit("message", { data: { kind: "view-strokes", id: 45, key: "many", strokes: new Array(9).fill(one) } });
  assert.deepEqual(page.out.at(-1), { kind: "view-strokes", id: 45, key: "many", tooMany: true });
  // Not as the engine reads them: a class stroke with no class, a way of two numbers.
  page.emit("message", { data: { kind: "view-strokes", id: 46, key: "k", strokes: [{ op: "class", right: [1, 0, 0], up: [0, 1, 0], dabs: [] }] } });
  assert.match(page.out.at(-1).error, /strokes weren't as the engine reads them/);
  page.emit("message", { data: { kind: "view-strokes", id: 47, key: "k", strokes: [{ op: "out", right: [1, 0], up: [0, 1, 0], dabs: [] }] } });
  assert.match(page.out.at(-1).error, /strokes weren't as the engine reads them/);
  // With a view: a scan's own colouring by its key, the brush's preview, its strokes' key, and the scan's older reading drawn while it's read again.
  page.emit("message", {
    data: {
      kind: "view",
      id: 48,
      scans: [{ token: "t1", affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0], tint: "c1" }],
      ask: { ...boxed, brush: { x: 120, y: 80, r: 24, painting: true, color: [1, 0, 0], hide: false, cut: 12.5, dabs: [100, 80, 110, 80], junk: 1 }, strokes: "press 3", stale: true },
    },
  });
  const brushed = calls.at(-1)[1];
  assert.equal(brushed.scans[0].tint, "c1");
  assert.deepEqual(brushed.brush, { x: 120, y: 80, r: 24, painting: true, color: [1, 0, 0], hide: false, cut: 12.5, dabs: [100, 80, 110, 80] });
  assert.equal(brushed.strokes, "press 3");
  assert.equal(brushed.stale, true);
  // None of them: none said. A preview with no size: left out.
  page.emit("message", { data: { kind: "view", id: 49, scans: [], ask: { ...boxed, brush: { x: 1, y: 2, r: 0 } } } });
  assert.equal("brush" in calls.at(-1)[1] || "strokes" in calls.at(-1)[1] || "stale" in calls.at(-1)[1] || "staleBase" in calls.at(-1)[1], false);
  // 1.40: an older reading only till the new one's first sample is in; asked for with `stale` too, `stale` alone.
  page.emit("message", { data: { kind: "view", id: 50, scans: [], ask: { ...boxed, staleBase: true } } });
  assert.equal(calls.at(-1)[1].staleBase, true);
  assert.equal("stale" in calls.at(-1)[1], false);
  page.emit("message", { data: { kind: "view", id: 51, scans: [], ask: { ...boxed, stale: true, staleBase: true } } });
  assert.equal(calls.at(-1)[1].stale, true);
  assert.equal("staleBase" in calls.at(-1)[1], false);
  // 1.41: a batch of the page's lines (its strips, nine floats a segment), a list of batches, and a view that names the list.
  const strips = new Float32Array([0, 0, 0, 10, 0, 0, 1, 1, 0]);
  page.emit("message", { data: { kind: "view-line-strips", id: 60, batch: 7, strips } });
  assert.deepEqual(page.out.at(-1), { kind: "view-line-strips", id: 60, batch: 7, ok: true });
  assert.deepEqual(calls.at(-1), ["line strips", [7, strips]]);
  page.emit("message", { data: { kind: "view-line-strips", id: 61, batch: 8, strips: new Float32Array(8) } });
  assert.match(page.out.at(-1).error, /lines weren't as the engine reads them/);
  page.emit("message", { data: { kind: "view-line-strips", id: 62, batch: -1, strips } });
  assert.match(page.out.at(-1).error, /lines weren't as the engine reads them/);
  page.emit("message", { data: { kind: "view-lines", id: 63, key: "lines 1", items: [{ id: 7, rgba: [1, 0, 0, 1] }, { id: 9, rgba: [0, 1, 0, 0.5], world: true, junk: 1 }] } });
  assert.deepEqual(page.out.at(-1), { kind: "view-lines", id: 63, key: "lines 1", ok: true });
  assert.deepEqual(calls.at(-1), ["lines", ["lines 1", [{ id: 7, rgba: [1, 0, 0, 1] }, { id: 9, rgba: [0, 1, 0, 0.5], world: true }]]]);
  // A batch the engine let go: which, and the list isn't taken.
  page.emit("message", { data: { kind: "view-lines", id: 64, key: "lines 2", items: [{ id: 7, rgba: [1, 0, 0, 1] }, { id: 200, rgba: [1, 0, 0, 1] }] } });
  assert.deepEqual(page.out.at(-1), { kind: "view-lines", id: 64, key: "lines 2", missing: [200] });
  page.emit("message", { data: { kind: "view-lines", id: 65, key: "lines 3", items: [{ id: 7, rgba: [1, 0, 0] }] } });
  assert.match(page.out.at(-1).error, /lines weren't as the engine reads them/);
  page.emit("message", { data: { kind: "view", id: 66, scans: [], ask: { ...boxed, lines: { key: "lines 1", place: [2, 10, 20], world: [1, 0, 0, 0, 1, 0, 30, 40], half: 0.625, ratio: 1.5, junk: 1 } } } });
  assert.deepEqual(calls.at(-1)[1].lines, { key: "lines 1", place: [2, 10, 20], world: [1, 0, 0, 0, 1, 0, 30, 40], half: 0.625, ratio: 1.5 });
  // No right and up for the drawing's own: only the plane's are placed. Not whole, or the split view's other side: none.
  page.emit("message", { data: { kind: "view", id: 67, scans: [], ask: { ...boxed, lines: { key: "lines 1", place: [2, 10, 20], half: 0.625, ratio: 1 } } } });
  assert.deepEqual(calls.at(-1)[1].lines, { key: "lines 1", place: [2, 10, 20], half: 0.625, ratio: 1 });
  page.emit("message", { data: { kind: "view", id: 68, scans: [], ask: { ...boxed, lines: { key: "lines 1", place: [2, 10], half: 0.625, ratio: 1 } } } });
  assert.equal("lines" in calls.at(-1)[1], false);
  page.emit("message", { data: { kind: "view", id: 69, scans: [], ask: { ...boxed, slot: 1, lines: { key: "lines 1", place: [2, 10, 20], half: 0.625, ratio: 1 } } } });
  assert.equal("lines" in calls.at(-1)[1], false);
  // How it's going, with nothing changed.
  page.emit("message", { data: { kind: "view-stats", id: 3 } });
  assert.deepEqual(page.out.at(-1), { kind: "view-stats", id: 3, stats: { started: true, running: true, frames: 12, points: 10, adapter: "A card" } });
  assert.equal(calls.filter((c) => c[0] === "show").length, 14);
  parent.emit("message", { data: { kind: "view-stop" } });
  assert.deepEqual(calls.at(-1), ["stop"]);
  page.emit("message", { data: { kind: "view-stats", id: 4 } });
  assert.equal(page.out.at(-1).error, "The engine's window isn't started.");
});

test("1.35: an engine from before can't draw in a window of its own, and says so", () => {
  const parent = new EventEmitter();
  parent.sent = [];
  parent.postMessage = (m) => parent.sent.push(m);
  createHost({ engine: fakeEngine(), parentPort: parent, every: () => 0, stop: () => {} });
  const page = port();
  parent.emit("message", { data: { kind: "page" }, ports: [page] });
  parent.emit("message", { data: { kind: "view-start", hwnd: "1" } });
  assert.match(parent.sent.at(-1).error, /can't draw in a window of its own/);
  page.emit("message", { data: { kind: "view", id: 1, ask: {} } });
  assert.match(page.out.at(-1).error, /can't draw in a window of its own/);
  page.emit("message", { data: { kind: "view-classes", id: 2, token: "t", key: "k" } });
  assert.match(page.out.at(-1).error, /can't draw a scan with the page's own classes/);
  page.emit("message", { data: { kind: "view-line-strips", id: 3, batch: 1, strips: new Float32Array(9) } });
  assert.match(page.out.at(-1).error, /can't draw linework/);
  page.emit("message", { data: { kind: "view-lines", id: 4, key: "k", items: [] } });
  assert.match(page.out.at(-1).error, /can't draw linework/);
});
