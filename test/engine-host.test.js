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
    closeScan: () => true,
    snap: (scans, ask) => ({ front: new Float64Array([0]), near: new Float64Array([1, 2]), tried: 5, cells: 1, ms: 0.1, asked: [scans, ask] }),
  };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

test("scans opened by token; a page told, then answered by token only", async () => {
  const parent = new EventEmitter();
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
    return opened.info;
  };
  const first = await run("t1");
  assert.equal(first.built, true);
  assert.equal(first.count, samplePoints().length);
  const again = await run("t2");
  assert.equal(again.built, false);
  fs.rmSync(dir, { recursive: true, force: true });
});
