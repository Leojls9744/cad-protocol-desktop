"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const { createCloseGuard } = require("../src/close-guard");
const { createAutoUpdate, launchDecision, RELEASES_URL } = require("../src/auto-update");

const wait = (ms) => new Promise((r) => setTimeout(r, ms));

function guard(options = {}) {
  const asked = [];
  let closed = 0;
  const g = createCloseGuard({ ask: (id) => asked.push(id), close: () => (closed += 1), ackMs: 30, maxMs: 120, ...options });
  return { g, asked, closed: () => closed };
}

test("closing asks the app, waits while it works, and closes when it's done", async () => {
  const { g, asked, closed } = guard();
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

/** electron-updater's autoUpdater, pretend: events, a check and an install. */
function fakeUpdater(script) {
  const u = new EventEmitter();
  u.installed = [];
  u.checkForUpdates = async () => {
    u.emit("checking-for-update");
    await script(u);
  };
  u.quitAndInstall = (silent, runAfter) => u.installed.push([silent, runAfter]);
  return u;
}

test("up to date: the badge says so, with the version", async () => {
  const updater = fakeUpdater(async (u) => u.emit("update-not-available", { version: "1.3.0" }));
  const seen = [];
  const auto = createAutoUpdate({ updater, current: "1.3.0", onChange: (s) => seen.push(s.phase) });
  assert.equal(updater.autoDownload, true);
  assert.equal(updater.autoInstallOnAppQuit, true);
  assert.deepEqual(await auto.check(), { current: "1.3.0", latest: "1.3.0", newer: false, url: RELEASES_URL, phase: "up-to-date", percent: 0 });
  assert.deepEqual(seen, ["checking", "up-to-date"]);
  assert.equal(auto.installNow(), false);
});

test("a new version downloads with its percent, then installs silently and restarts", async () => {
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
  assert.deepEqual(updater.installed, [[true, true]]);
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

test("at launch: an update found means the update screen stays; otherwise the app opens", async () => {
  const newer = createAutoUpdate({ updater: fakeUpdater(async (u) => u.emit("update-available", { version: "2.0.0" })), current: "1.3.0" });
  assert.equal(await launchDecision(newer, 200), "update");
  const same = createAutoUpdate({ updater: fakeUpdater(async (u) => u.emit("update-not-available", { version: "1.3.0" })), current: "1.3.0" });
  assert.equal(await launchDecision(same, 200), "app");
  const slow = createAutoUpdate({ updater: fakeUpdater(() => new Promise(() => {})), current: "1.3.0" });
  assert.equal(await launchDecision(slow, 30), "app");
});
