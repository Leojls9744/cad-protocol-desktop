"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const { createCloseGuard } = require("../src/close-guard");
const { createAutoUpdate, RELEASES_URL } = require("../src/auto-update");

const wait = (ms) => new Promise((r) => setTimeout(r, ms));

function guard(options = {}) {
  const asked = [];
  let closed = 0;
  const g = createCloseGuard({ ask: (id) => asked.push(id), close: () => (closed += 1), ackMs: 30, maxMs: 120, ...options });
  return { g, asked, closed: () => closed };
}

test("closing asks the app, waits while it works, and closes when it's done", async () => {
  // A long limit: a slow test machine's timers mustn't reach it here.
  const { g, asked, closed } = guard({ maxMs: 5000 });
  assert.equal(g.request(), true);
  assert.equal(g.request(), true);
  assert.deepEqual(asked, [1]);
  g.started(1);
  await wait(60);
  assert.equal(closed(), 0);
  g.done(1);
  assert.equal(closed(), 1);
  assert.equal(g.request(), false);
});

test("an app that doesn't answer closes after the answer time; one that hangs, after the limit", async () => {
  const silent = guard();
  silent.g.request();
  await wait(50);
  assert.equal(silent.closed(), 1);
  const hung = guard();
  hung.g.request();
  hung.g.started(1);
  await wait(60);
  assert.equal(hung.closed(), 0);
  await wait(100);
  assert.equal(hung.closed(), 1);
});

test("answers to an earlier question are ignored, and Stay asks again next time", () => {
  const { g, asked, closed } = guard();
  g.request();
  g.started(1);
  g.reset();
  g.request();
  assert.deepEqual(asked, [1, 2]);
  g.done(1);
  assert.equal(closed(), 0);
  g.done(2);
  assert.equal(closed(), 1);
});

test("1.18: while the page asks the person, no time limit; the work after gets its minute; Cancel keeps it open", async () => {
  const { g, asked, closed } = guard({ ackMs: 30, maxMs: 120 });
  g.request();
  g.holding(1);
  // Past the answer time and the limit: still waiting on the person.
  await wait(200);
  assert.equal(closed(), 0);
  assert.equal(g.asking, true);
  // Saving and signing out: the limit counts from now.
  g.started(1);
  await wait(60);
  assert.equal(closed(), 0);
  await wait(120);
  assert.equal(closed(), 1);
  // Cancel: open, and asked again next time.
  const c = guard({ ackMs: 30, maxMs: 120 });
  c.g.request();
  c.g.holding(1);
  c.g.cancelled(2); // an earlier or later question's: ignored
  assert.equal(c.g.asking, true);
  c.g.cancelled(1);
  assert.equal(c.g.asking, false);
  await wait(200);
  assert.equal(c.closed(), 0);
  assert.equal(c.g.request(), true);
  assert.deepEqual(c.asked, [1, 2]);
  c.g.done(2);
  assert.equal(c.closed(), 1);
  assert.deepEqual(asked, [1]);
});

/** electron-updater's autoUpdater, pretend: events, a check and an install. */
function fakeUpdater(script) {
  const u = new EventEmitter();
  u.installed = [];
  u.checkForUpdates = async () => {
    u.emit("checking-for-update");
    await script(u);
  };
  u.quitAndInstall = (silent, runAfter) => u.installed.push([silent, runAfter]);
  u.install = (silent, runAfter) => {
    u.installed.push(["on quit", silent, runAfter]);
    return true;
  };
  return u;
}

test("up to date: the badge says so, with the version", async () => {
  const updater = fakeUpdater(async (u) => u.emit("update-not-available", { version: "1.3.0" }));
  const seen = [];
  const auto = createAutoUpdate({ updater, current: "1.3.0", onChange: (s) => seen.push(s.phase) });
  assert.equal(updater.autoDownload, true);
  // 1.30: never silently as the app quits.
  assert.equal(updater.autoInstallOnAppQuit, false);
  assert.deepEqual(await auto.check(), { current: "1.3.0", latest: "1.3.0", newer: false, url: RELEASES_URL, phase: "up-to-date", percent: 0 });
  assert.deepEqual(seen, ["checking", "up-to-date"]);
  assert.equal(auto.installNow(), false);
  assert.equal(auto.installOnQuit(), false);
  assert.equal(auto.installsOnQuit, false);
});

test("a new version downloads with its percent, then installs with the installer's progress shown and restarts", async () => {
  const updater = fakeUpdater(async (u) => {
    u.emit("update-available", { version: "1.3.1" });
    u.emit("download-progress", { percent: 44.6 });
    u.emit("download-progress", { percent: 250 });
    u.emit("update-downloaded", { version: "1.3.1" });
  });
  const percents = [];
  const auto = createAutoUpdate({ updater, current: "1.3.0", onChange: (s) => percents.push(s.percent) });
  const state = await auto.check();
  assert.equal(state.phase, "ready");
  assert.equal(state.latest, "1.3.1");
  assert.equal(state.newer, true);
  assert.deepEqual(percents.slice(1, 4), [0, 45, 100]);
  assert.equal(auto.installNow(), true);
  assert.deepEqual(updater.installed, [[false, true]]);
});

test("a failed check is 'couldn't check', never 'up to date', and keeps a ready update", async () => {
  const offline = fakeUpdater(async () => {
    throw new Error("net::ERR_INTERNET_DISCONNECTED");
  });
  const auto = createAutoUpdate({ updater: offline, current: "1.3.0" });
  assert.equal((await auto.check()).phase, "error");

  const updater = fakeUpdater(async (u) => u.emit("update-downloaded", { version: "1.3.2" }));
  const ready = createAutoUpdate({ updater, current: "1.3.0" });
  await ready.check();
  updater.emit("error", new Error("later check failed"));
  assert.equal(ready.state.phase, "ready");
});

test("1.30: closing with an update ready installs it in sight (its progress shown, the app opened again), once", async () => {
  const updater = fakeUpdater(async (u) => u.emit("update-downloaded", { version: "1.30.0" }));
  const auto = createAutoUpdate({ updater, current: "1.29.0" });
  await auto.check();
  assert.equal(auto.installsOnQuit, true);
  assert.equal(auto.installOnQuit(), true);
  assert.equal(auto.installsOnQuit, false);
  assert.equal(auto.installOnQuit(), false);
  assert.deepEqual(updater.installed, [["on quit", false, true]]);
  // Restart to update first: not installed again as it quits.
  const restarted = fakeUpdater(async (u) => u.emit("update-downloaded", { version: "1.30.0" }));
  const r = createAutoUpdate({ updater: restarted, current: "1.29.0" });
  await r.check();
  assert.equal(r.installNow(), true);
  assert.equal(r.installOnQuit(), false);
  assert.deepEqual(restarted.installed, [[false, true]]);
});
