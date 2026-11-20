"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { createStartCheck } = require("../src/start-check");
const { createStartWatch } = require("../src/start-watch");
const { createLaunch } = require("../src/launch");
const { cardsFrom, createDeviceSpecs } = require("../src/device-specs");

/** Timers run by hand. */
function clock() {
  const timers = new Map();
  let id = 0;
  return {
    setTimer: (f, ms) => {
      id += 1;
      timers.set(id, { f, ms });
      return id;
    },
    clearTimer: (t) => timers.delete(t),
    fire: (ms) => {
      for (const [t, v] of [...timers]) if (v.ms === ms) {
        timers.delete(t);
        v.f();
      }
    },
    count: () => timers.size,
  };
}

const fakeUpdates = (answer, start = { phase: "idle" }) => {
  const u = { state: start, installed: 0, checks: 0 };
  u.check = async () => {
    u.checks += 1;
    u.state = answer;
    return answer;
  };
  u.installNow = () => {
    u.installed += 1;
    return true;
  };
  return u;
};

test("1.32: the card looks for an update first; none, the app opens", async () => {
  const c = clock();
  const said = [];
  const check = createStartCheck({ updates: fakeUpdates({ phase: "up-to-date" }), say: (t) => said.push(t), ...c });
  assert.equal(await check.run(), "open");
  assert.deepEqual(said, ["Checking for updates…"]);
});

test("1.32: one found downloads on the card with its progress, then installs", async () => {
  const c = clock();
  const said = [];
  const u = fakeUpdates({ phase: "downloading", latest: "1.33.0", percent: 0 });
  const check = createStartCheck({ updates: u, say: (t, d, p) => said.push([t, d, p]), ...c });
  const how = check.run();
  await new Promise((r) => setImmediate(r));
  check.heard({ phase: "downloading", latest: "1.33.0", percent: 42 });
  assert.deepEqual(said.at(-1), ["Updating CAD Protocol…", "Downloading 1.33.0 · 42%", 42]);
  check.heard({ phase: "ready", latest: "1.33.0", percent: 100 });
  assert.equal(await how, "installing");
  assert.equal(u.installed, 1);
});

test("1.32: no answer in time, or a download too long, opens the app", async () => {
  const c = clock();
  const slow = { state: { phase: "idle" }, check: () => new Promise(() => {}), installNow: () => true };
  const a = createStartCheck({ updates: slow, say: () => {}, checkMs: 6000, ...c });
  const howA = a.run();
  c.fire(6000);
  assert.equal(await howA, "open");
  const c2 = clock();
  const u = fakeUpdates({ phase: "downloading", latest: "1.33.0", percent: 1 });
  const b = createStartCheck({ updates: u, say: () => {}, checkMs: 6000, downloadMs: 120000, ...c2 });
  const howB = b.run();
  await new Promise((r) => setImmediate(r));
  c2.fire(6000);
  assert.equal(b.phase, "downloading");
  c2.fire(120000);
  assert.equal(await howB, "open");
  assert.equal(u.installed, 0);
});

test("1.32: one downloaded before installs on the card at once", async () => {
  const u = fakeUpdates({ phase: "up-to-date" }, { phase: "ready", latest: "1.33.0" });
  const check = createStartCheck({ updates: u, say: () => {}, ...clock() });
  assert.equal(await check.run(), "installing");
  assert.equal(u.checks, 0);
});

test("1.32: a page that doesn't say it's up in time is loaded fresh, once", () => {
  const c = clock();
  let stuck = 0;
  const w = createStartWatch({ stuck: () => (stuck += 1), readyMs: 30000, ...c });
  w.begin(1000);
  c.fire(30000);
  assert.equal(stuck, 1);
  assert.equal(w.fired, true);
  w.begin(40000);
  assert.equal(c.count(), 0);
  assert.equal(w.alive(45000), 44000);
  assert.equal(w.alive(46000), null);
  const ok = createStartWatch({ stuck: () => (stuck += 1), readyMs: 30000, ...c });
  ok.begin(0);
  assert.equal(ok.alive(2500), 2500);
  c.fire(30000);
  assert.equal(stuck, 1);
});

test("1.32: the launch begins its timers again for a fresh load", () => {
  const c = clock();
  const shown = [];
  const l = createLaunch({ showMain: (why) => shown.push(why), closeSplash: () => {}, maxMs: 25000, fallbackMs: 4000, ...c });
  l.loading();
  l.again();
  assert.equal(c.count(), 1);
  c.fire(25000);
  assert.deepEqual(shown, ["slow"]);
});

test("1.32: Windows' cards read from their drivers, full memory size, each once", () => {
  const json = JSON.stringify([
    { name: "NVIDIA GeForce RTX 4070 SUPER", q: 12878610432, m: 4293918720 },
    { name: "Microsoft Basic Display Adapter", q: null, m: null },
    { name: "NVIDIA GeForce RTX 4070 SUPER", q: 12878610432, m: null },
    { name: "Intel(R) UHD Graphics 770", q: null, m: 134217728 },
  ]);
  assert.deepEqual(cardsFrom(json), [
    { name: "NVIDIA GeForce RTX 4070 SUPER", vramMB: 12282 },
    { name: "Intel(R) UHD Graphics 770", vramMB: 128 },
    { name: "Microsoft Basic Display Adapter", vramMB: 0 },
  ]);
  assert.deepEqual(cardsFrom(JSON.stringify({ name: "One", q: 1048576 * 2048 })), [{ name: "One", vramMB: 2048 }]);
  assert.deepEqual(cardsFrom("not json"), []);
});

test("1.32: the specs: the computer's memory, and the cards on Windows", async () => {
  const off = createDeviceSpecs({ run: null, platform: "linux", totalmem: () => 32 * 1073741824 });
  assert.deepEqual(await off.read(), { ramMB: 32768, gpus: [] });
  let calls = 0;
  const win = createDeviceSpecs({
    platform: "win32",
    totalmem: () => 16 * 1073741824,
    run: (file, args, opts, done) => {
      calls += 1;
      assert.equal(file, "powershell.exe");
      assert.equal(opts.windowsHide, true);
      done(null, JSON.stringify({ name: "Card", q: 8 * 1073741824 }));
    },
  });
  assert.deepEqual(await win.read(), { ramMB: 16384, gpus: [{ name: "Card", vramMB: 8192 }] });
  await win.read();
  assert.equal(calls, 1);
  const failing = createDeviceSpecs({ platform: "win32", totalmem: () => 1073741824, run: (f, a, o, done) => done(new Error("no")) });
  assert.deepEqual(await failing.read(), { ramMB: 1024, gpus: [] });
});

test("1.32: main checks updates on the card, watches the start, answers the specs; the bridge has them", () => {
  const main = fs.readFileSync(path.join(__dirname, "../src/main.js"), "utf8");
  const preload = fs.readFileSync(path.join(__dirname, "../src/preload.js"), "utf8");
  assert.match(main, /const how = await startCheck\.run\(\);/);
  assert.match(main, /storages: \["serviceworkers", "cachestorage"\],/);
  assert.match(main, /ipcMain\.handle\("cadp:specs", \(event\) => \{\s*if \(!fromApp\(event\)\) throw new Error\("not allowed"\);\s*return deviceSpecs\.read\(\);/);
  assert.match(main, /log\(`window shown: \$\{why\}/);
  assert.match(preload, /specs: \(\) => ipcRenderer\.invoke\("cadp:specs"\),/);
});
