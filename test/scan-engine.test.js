"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");
const { EventEmitter } = require("node:events");
const { STEADY_MS, addonPath, cacheDirFor, createScanEngine, scanPathOk, unpacked } = require("../src/scan-engine");

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
  hosts[0].emit("message", { kind: "opened", token: a.token });
  for (let k = 0; k < 4; k += 1) {
    hosts[hosts.length - 1].emit("exit", 1);
    if (timers.length > k) timers[k][0]();
  }
  assert.equal(hosts.length, 4);
  assert.equal(timers.length, 3);
  assert.deepEqual(hosts[1].sent[0][0].token, a.token);
  assert.equal(e.running, false);
  // 1.25: given up, scans go through the page.
  assert.equal(await e.register("C:\\Jobs\\Lot 7\\scan 2.LAZ"), null);
  assert.equal(e.connect({ postMessage: () => {} }), false);
  assert.equal(hosts.length, 4);
});

test("a page is told when the engine stops and when it's back; scans it was still reading are let go", async () => {
  const { e, hosts, timers } = engine();
  const told = [];
  const page = { postMessage: (channel, msg) => told.push([channel, msg]) };
  e.connect(page);
  const a = await e.register("C:\\Jobs\\Lot 7\\scan 1.las");
  const b = await e.register("C:\\Jobs\\Lot 7\\scan 2.LAZ");
  hosts[0].emit("message", { kind: "opened", token: a.token });
  hosts[0].emit("exit", 1);
  assert.deepEqual(told[1], ["cadp:engine:lost", { again: true, dropped: [b.token] }]);
  timers[0][0]();
  assert.deepEqual(told[2], ["cadp:engine:back", {}]);
  // Started again with the scan it had opened, not the one it was reading.
  assert.deepEqual(
    hosts[1].sent.map(([m]) => [m.kind, m.token]),
    [["open", a.token]],
  );
  // That one chosen again: a new token, tried again.
  const b2 = await e.register("C:\\Jobs\\Lot 7\\scan 2.LAZ");
  assert.notEqual(b2.token, b.token);
  assert.equal(hosts[1].sent[1][0].token, b2.token);
  // A page gone isn't told.
  let closed = false;
  const gone = {
    postMessage: (channel) => {
      if (channel !== "cadp:engine:port") assert.fail("told a page that's gone");
    },
    isDestroyed: () => closed,
  };
  e.connect(gone);
  closed = true;
  hosts[1].emit("exit", 1);
  assert.deepEqual(told[told.length - 1][0], "cadp:engine:lost");
});

test("a process that ran a good while before it stopped starts the count again", async () => {
  let clock = 0;
  const { e, hosts, timers } = engine({ now: () => clock });
  await e.register("C:\\Jobs\\Lot 7\\scan 1.las");
  for (let k = 0; k < 6; k += 1) {
    clock += STEADY_MS;
    hosts[hosts.length - 1].emit("exit", 1);
    timers[timers.length - 1][0]();
  }
  assert.equal(hosts.length, 7);
  assert.equal(e.running, true);
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

test("1.28: a cut asked of the engine, its progress and its end relayed by id", async () => {
  const { e, hosts } = engine();
  const r = await e.register("C:\\Jobs\\Lot 7\\scan 1.las");
  assert.equal(e.pathOf(r.token), "C:\\Jobs\\Lot 7\\scan 1.las");
  const told = [];
  const cut = e.crop(r.token, "C:\\Jobs\\Lot 7\\scan 1 cut.las.saving", { boxes: [] }, (f) => told.push(f));
  const [msg] = hosts[0].sent.at(-1);
  assert.equal(msg.kind, "crop");
  assert.equal(msg.token, r.token);
  hosts[0].emit("message", { kind: "crop-progress", id: msg.id, done: 0.5 });
  hosts[0].emit("message", { kind: "crop-done", id: msg.id, kept: 9, processed: 20 });
  assert.deepEqual(await cut, { kind: "crop-done", id: msg.id, kept: 9, processed: 20 });
  assert.deepEqual(told, [0.5]);
  // Another token, or the engine stopping: an error, nothing waits for good.
  assert.match((await e.crop("nope", "x", {})).error, /isn't open/);
  const second = e.crop(r.token, "y", {});
  hosts[0].emit("exit", 1);
  assert.match((await second).error, /stopped/);
});

test("1.35: the engine's own window is in the app's frame from its start, and again after a crash", () => {
  const { e, hosts, timers } = engine();
  // Told the frame before the engine's running: it starts, and is told as it does.
  assert.equal(e.viewIn(66052n), true);
  assert.equal(hosts.length, 1);
  assert.deepEqual(hosts[0].sent.map(([m]) => m), [{ kind: "view-start", hwnd: "66052" }]);
  // Another frame while it runs: told at once.
  assert.equal(e.viewIn("77"), true);
  assert.deepEqual(hosts[0].sent.at(-1)[0], { kind: "view-start", hwnd: "77" });
  // It stops by itself: started again, in the frame it was in.
  hosts[0].emit("exit", 1);
  timers.shift()[0]();
  assert.equal(hosts.length, 2);
  assert.deepEqual(hosts[1].sent.map(([m]) => m), [{ kind: "view-start", hwnd: "77" }]);
  // Taken away: said, and not started in a frame again.
  assert.equal(e.viewIn(null), true);
  assert.deepEqual(hosts[1].sent.at(-1)[0], { kind: "view-stop" });
  hosts[1].emit("exit", 1);
  timers.shift()[0]();
  assert.deepEqual(hosts[2].sent, []);
});
