"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const { createLaunch } = require("../src/launch");
const { createSplash } = require("../src/splash");
const { createCloseGuard } = require("../src/close-guard");

function fakeTimers() {
  const timers = [];
  return {
    setTimer: (fn, ms) => {
      const t = { fn, ms, cleared: false };
      timers.push(t);
      return t;
    },
    clearTimer: (t) => {
      if (t) t.cleared = true;
    },
    fire: (ms) => {
      for (const t of timers) if (!t.cleared && t.ms === ms) {
        t.cleared = true;
        t.fn();
      }
    },
  };
}

function launchWith(options = {}) {
  const shown = [];
  let closed = 0;
  const timers = fakeTimers();
  const launch = createLaunch({
    showMain: (why) => shown.push(why),
    closeSplash: () => (closed += 1),
    fallbackMs: 4000,
    maxMs: 25000,
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
    ...options,
  });
  return { launch, shown, closed: () => closed, timers };
}

test("the app's window shows when it says it's ready, and the card closes, once", () => {
  const { launch, shown, closed } = launchWith();
  launch.loading();
  launch.loaded();
  assert.deepEqual(shown, []);
  launch.ready();
  launch.ready();
  assert.deepEqual(shown, ["ready"]);
  assert.equal(closed(), 1);
  assert.equal(launch.shown, true);
});

test("an app that never says so shows a little after it loads; one that never loads, at the limit", () => {
  const a = launchWith();
  a.launch.loading();
  a.launch.loaded();
  a.timers.fire(4000);
  assert.deepEqual(a.shown, ["loaded"]);
  const b = launchWith();
  b.launch.loading();
  b.timers.fire(25000);
  assert.deepEqual(b.shown, ["slow"]);
});

test("the offline page shows at once", () => {
  const { launch, shown } = launchWith();
  launch.loading();
  launch.failed();
  assert.deepEqual(shown, ["failed"]);
});

test("while an update's looked for, the app waits behind the card, then shows if it's ready", () => {
  const { launch, shown, closed } = launchWith();
  launch.hold();
  launch.loading();
  launch.loaded();
  launch.ready();
  assert.deepEqual(shown, []);
  assert.equal(closed(), 0);
  launch.release();
  assert.deepEqual(shown, ["ready"]);
  // Released before it was ready: it shows when it is.
  const later = launchWith();
  later.launch.hold();
  later.launch.release();
  assert.deepEqual(later.shown, []);
  later.launch.ready();
  assert.deepEqual(later.shown, ["ready"]);
});

class FakeWindow extends EventEmitter {
  static made = [];
  constructor(options) {
    super();
    this.options = options;
    this.destroyed = false;
    this.visible = false;
    this.ran = [];
    this.loaded = null;
    this.webContents = new EventEmitter();
    this.webContents.executeJavaScript = async (code) => {
      this.ran.push(code);
    };
    FakeWindow.made.push(this);
  }
  isDestroyed() {
    return this.destroyed;
  }
  show() {
    this.visible = true;
  }
  focus() {}
  async loadFile(file, options) {
    this.loaded = { file, options };
  }
  destroy() {
    this.destroyed = true;
    this.emit("closed");
  }
}

test("the card is a small window of its own, no frame, and says what it's told", async () => {
  let quit = 0;
  const timers = fakeTimers();
  const splash = createSplash({
    BrowserWindow: FakeWindow,
    file: "update.html",
    webPreferences: { sandbox: true },
    onClosedByPerson: () => (quit += 1),
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });
  splash.open();
  const win = FakeWindow.made.at(-1);
  assert.equal(win.options.frame, false);
  assert.equal(win.options.width, 560);
  assert.equal(win.options.height, 300);
  assert.equal(win.options.resizable, false);
  assert.equal(win.options.webPreferences.backgroundThrottling, false);
  assert.deepEqual(win.loaded, { file: "update.html", options: { query: { splash: "1" } } });
  splash.say("Loading CAD Protocol…", "CAD Protocol for Windows");
  assert.match(win.ran.at(-1), /showUpdate\("Loading CAD Protocol…", "CAD Protocol for Windows", null, ""\)/);
  // Closed by the app: not the person's doing.
  splash.close();
  assert.equal(quit, 0);
  assert.equal(splash.isOpen, false);
  // Closed by the person before the app showed: the app's told.
  splash.open();
  FakeWindow.made.at(-1).emit("closed");
  assert.equal(quit, 1);
});

test("the card shows only once its picture's drawn, and stays long enough to read (1.8.1)", async () => {
  const timers = fakeTimers();
  let clock = 1000;
  const splash = createSplash({
    BrowserWindow: FakeWindow,
    file: "update.html",
    webPreferences: {},
    now: () => clock,
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });
  splash.open();
  const win = FakeWindow.made.at(-1);
  // Loaded, but its picture isn't drawn yet: not shown, never blank.
  win.webContents.emit("did-finish-load");
  assert.equal(win.visible, false);
  assert.match(win.ran.at(-1), /img\.src = "splash\.jpg"/);
  await new Promise((r) => setImmediate(r));
  assert.equal(win.visible, true);
  assert.equal(splash.isShown, true);
  // The app's ready at once: it shows, and the card goes, once it's been up 1.2 s.
  const seen = [];
  splash.after(() => seen.push("main"));
  splash.close();
  assert.deepEqual(seen, []);
  assert.equal(splash.isOpen, true);
  clock += 1200;
  timers.fire(1200);
  assert.deepEqual(seen, ["main"]);
  assert.equal(splash.isOpen, false);
  // Its picture never says it's drawn: shown anyway after 1.5 s.
  const late = createSplash({
    BrowserWindow: class extends FakeWindow {
      constructor(options) {
        super(options);
        this.webContents.executeJavaScript = () => new Promise(() => {});
      }
    },
    file: "update.html",
    webPreferences: {},
    now: () => clock,
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });
  late.open();
  const lateWin = FakeWindow.made.at(-1);
  lateWin.webContents.emit("did-finish-load");
  assert.equal(lateWin.visible, false);
  timers.fire(1500);
  assert.equal(lateWin.visible, true);
  // Made out of sight for signing out, and closed before it was wanted: never seen.
  const quiet = createSplash({
    BrowserWindow: FakeWindow,
    file: "update.html",
    webPreferences: {},
    now: () => clock,
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });
  quiet.prepare();
  const quietWin = FakeWindow.made.at(-1);
  quietWin.webContents.emit("did-finish-load");
  await new Promise((r) => setImmediate(r));
  assert.equal(quietWin.visible, false);
  quiet.close();
  assert.equal(quiet.isOpen, false);
  assert.equal(quietWin.visible, false);
});

test("the close guard says it's still asking until the app's done", () => {
  let closed = 0;
  const g = createCloseGuard({ ask: () => {}, close: () => (closed += 1), ackMs: 5000, maxMs: 5000 });
  assert.equal(g.asking, false);
  g.request();
  assert.equal(g.asking, true);
  g.started(1);
  assert.equal(g.asking, true);
  g.done(1);
  assert.equal(g.asking, false);
  assert.equal(closed, 1);
});
