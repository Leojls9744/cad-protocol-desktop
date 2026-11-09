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

test("the card is a small window of its own, no frame, and says what it's told", () => {
  let quit = 0;
  const splash = createSplash({
    BrowserWindow: FakeWindow,
    file: "update.html",
    webPreferences: { sandbox: true },
    onClosedByPerson: () => (quit += 1),
  });
  splash.open();
  const win = FakeWindow.made.at(-1);
  assert.equal(win.options.frame, false);
  assert.equal(win.options.width, 560);
  assert.equal(win.options.height, 300);
  assert.equal(win.options.resizable, false);
  assert.deepEqual(win.loaded, { file: "update.html", options: { query: { splash: "1" } } });
  win.emit("ready-to-show");
  assert.equal(win.visible, true);
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
