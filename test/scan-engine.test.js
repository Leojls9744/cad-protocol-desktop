"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");
const { EventEmitter } = require("node:events");
const { addonPath, cacheDirFor, createScanEngine, scanPathOk, unpacked } = require("../src/scan-engine");

function fakeHost() {
  const h = new EventEmitter();
  h.sent = [];
  h.killed = false;
  h.postMessage = (msg, ports) => h.sent.push([msg, ports]);
  h.kill = () => {
    h.killed = true;
  };
  return h;
}

function engine(over = {}) {
  const hosts = [];
  const timers = [];
  const files = new Map([
    ["C:\\Jobs\\Lot 7\\scan 1.las", { size: 123 }],
    ["C:\\Jobs\\Lot 7\\scan 2.LAZ", { size: 456 }],
    ["C:\\Jobs\\Lot 7\\folder.las", { size: 0, dir: true }],
  ]);
  const e = createScanEngine({
    fork: (p, args, options) => {
      const h = fakeHost();
      h.args = [p, args, options];
      hosts.push(h);
      return h;
    },
    newChannel: () => ({ port1: { end: 1 }, port2: { end: 2 } }),
    // Windows' file names are the same in any case.
    statFile: async (p) => {
      const f = [...files].find(([k]) => k.toLowerCase() === p.toLowerCase())?.[1];
      return f ? { size: f.size, isFile: () => !f.dir } : null;
    },
    exists: () => true,
    cacheDir: "C:\\Users\\leo\\AppData\\Local\\CAD Protocol\\scan-engine",
    addon: "C:\\app\\resources\\app.asar.unpacked\\engine\\dist\\cadp-engine.node",
    hostPath: "C:\\app\\resources\\app.asar\\src\\engine-host.js",
    later: (f, ms) => timers.push([f, ms]),
    ...over,
  });
  return { e, hosts, timers };
}

test("only an absolute .las, .laz or .e57 path is a scan the engine opens", () => {
  assert.equal(scanPathOk("C:\\Jobs\\scan.las"), true);
  assert.equal(scanPathOk("C:\\Jobs\\scan.LAZ"), true);
  assert.equal(scanPathOk("/home/leo/scan.las"), true);
  assert.equal(scanPathOk("scan.las"), false);
  // 1.24: E57 too.
  assert.equal(scanPathOk("C:\\Jobs\\scan.e57"), true);
  assert.equal(scanPathOk("C:\\Jobs\\scan.E57"), true);
  assert.equal(scanPathOk("C:\\Jobs\\scan.e58"), false);
  assert.equal(scanPathOk("C:\\Jobs\\drawing.cadp"), false);
  assert.equal(scanPathOk("C:\\Jobs\\scan.las\u0000.exe"), false);
  assert.equal(scanPathOk(""), false);
  assert.equal(scanPathOk(null), false);
});

test("the add-on is found unpacked beside the app, the indexes in local app data", () => {
  assert.equal(unpacked("C:\\app\\resources\\app.asar\\engine\\dist\\cadp-engine.node"), "C:\\app\\resources\\app.asar.unpacked\\engine\\dist\\cadp-engine.node");
  assert.equal(addonPath(path.join("x", "src")), path.join("x", "engine", "dist", "cadp-engine.node"));
  assert.equal(cacheDirFor({ LOCALAPPDATA: "C:\\Users\\leo\\AppData\\Local" }, "C:\\roaming"), path.join("C:\\Users\\leo\\AppData\\Local", "CAD Protocol", "scan-engine"));
  assert.equal(cacheDirFor({}, "/data"), path.join("/data", "scan-engine"));
});

test("a scan file chosen is given a token, the same one each time, and opened once", async () => {
  const { e, hosts } = engine();
  assert.equal(hosts.length, 0);
  const a = await e.register("C:\\Jobs\\Lot 7\\scan 1.las");
  assert.equal(a.name, "scan 1.las");
  assert.equal(a.size, 123);
  assert.match(a.token, /^[0-9a-f]{32}$/);
  assert.equal(hosts.length, 1);
  assert.deepEqual(hosts[0].sent[0][0], {
    kind: "open",
    token: a.token,
    path: "C:\\Jobs\\Lot 7\\scan 1.las",
    cacheDir: "C:\\Users\\leo\\AppData\\Local\\CAD Protocol\\scan-engine",
  });
  assert.deepEqual(hosts[0].args[1], ["--cadp-engine-addon=C:\\app\\resources\\app.asar.unpacked\\engine\\dist\\cadp-engine.node"]);
  const again = await e.register("c:\\jobs\\lot 7\\SCAN 1.las");
  assert.equal(again.token, a.token);
  assert.equal(hosts[0].sent.length, 1);
  const b = await e.register("C:\\Jobs\\Lot 7\\scan 2.LAZ");
  assert.notEqual(b.token, a.token);
  assert.equal(hosts[0].sent.length, 2);
});

test("nothing that isn't a scan file there is opened", async () => {
  const { e, hosts } = engine();
  assert.equal(await e.register("C:\\Jobs\\Lot 7\\missing.las"), null);
  assert.equal(await e.register("C:\\Jobs\\Lot 7\\folder.las"), null);
  assert.equal(await e.register("C:\\Windows\\system.ini"), null);
  assert.equal(await e.register("relative.las"), null);
  assert.equal(hosts.length, 0);
});

test("a page gets a port to the engine; without the add-on, no engine at all", async () => {
  const { e, hosts } = engine();
  const sent = [];
  assert.equal(e.connect({ postMessage: (...a) => sent.push(a) }), true);
  assert.deepEqual(hosts[0].sent[0], [{ kind: "page" }, [{ end: 1 }]]);
  assert.deepEqual(sent[0], ["cadp:engine:port", null, [{ end: 2 }]]);
  const none = engine({ exists: () => false }).e;
  assert.equal(none.available, false);
  assert.equal(none.connect({ postMessage: () => {} }), false);
  assert.equal(await none.register("C:\\Jobs\\Lot 7\\scan 1.las"), null);
});

test("after a crash the engine starts again and opens its scans again, three times at most", async () => {
  const { e, hosts, timers } = engine();
  const a = await e.register("C:\\Jobs\\Lot 7\\scan 1.las");
  for (let k = 0; k < 4; k += 1) {
    hosts[hosts.length - 1].emit("exit", 1);
    if (timers.length > k) timers[k][0]();
  }
  assert.equal(hosts.length, 4);
  assert.equal(timers.length, 3);
  assert.deepEqual(hosts[1].sent[0][0].token, a.token);
  assert.equal(e.running, false);
});

test("quitting stops the engine's process", async () => {
  const { e, hosts } = engine();
  await e.register("C:\\Jobs\\Lot 7\\scan 1.las");
  assert.equal(e.running, true);
  e.stop();
  assert.equal(hosts[0].killed, true);
  assert.equal(e.running, false);
  // Not started again after it was stopped.
  hosts[0].emit("exit", 0);
  assert.equal(hosts.length, 1);
});
