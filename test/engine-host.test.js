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
