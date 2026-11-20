"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const fs = require("node:fs");
const path = require("node:path");
const {
  FLAG,
  KEY,
  SEE_THROUGH_CSS,
  FRAME_ANSWERS,
  frameOptions,
  glue,
  handleText,
  isToggleChord,
  keepBarActive,
  layerTrialOn,
  near,
  pageOptions,
  pixelAt,
  toggleWords,
} = require("../src/layer-trial");
const { FILES, fileFor } = require("../src/local-pages");

/** A stand-in window: its bounds, whether it shows, and what was asked of it. */
function fakeWindow(bounds, inset = { left: 8, top: 31, right: 8, bottom: 8 }) {
  const win = new EventEmitter();
  win.bounds = { ...bounds };
  win.shown = false;
  win.destroyed = false;
  win.minimized = false;
  win.maximized = false;
  win.focused = false;
  win.calls = [];
  win.isDestroyed = () => win.destroyed;
  win.isVisible = () => win.shown;
  win.isMinimized = () => win.minimized;
  win.isMaximized = () => win.maximized;
  win.isFocused = () => win.focused;
  win.getBounds = () => ({ ...win.bounds });
  win.getNormalBounds = () => ({ ...win.bounds, normal: true });
  win.getContentBounds = () => ({
    x: win.bounds.x + inset.left,
    y: win.bounds.y + inset.top,
    width: win.bounds.width - inset.left - inset.right,
    height: win.bounds.height - inset.top - inset.bottom,
  });
  win.setBounds = (b) => {
    win.calls.push(["setBounds", b]);
    win.bounds = { ...b };
  };
  win.show = () => {
    win.shown = true;
    win.emit("show");
  };
  win.showInactive = () => {
    win.calls.push(["showInactive"]);
    win.shown = true;
    win.emit("show");
  };
  win.hide = () => {
    win.shown = false;
    win.emit("hide");
  };
  win.focus = () => {
    win.calls.push(["focus"]);
    win.focused = true;
  };
  win.maximize = () => {
    win.calls.push(["maximize"]);
    win.maximized = true;
    win.emit("maximize");
  };
  win.restore = () => {
    win.calls.push(["restore"]);
    win.minimized = false;
    win.emit("restore");
  };
  win.flashFrame = (on) => win.calls.push(["flashFrame", on]);
  win.setAlwaysOnTop = (on) => win.calls.push(["setAlwaysOnTop", on]);
  win.setTitle = (title) => win.calls.push(["setTitle", title]);
  win.close = () => {
    win.calls.push(["close"]);
    const event = { prevented: false, preventDefault: () => (event.prevented = true) };
    win.emit("close", event);
    if (!event.prevented) win.destroy();
  };
  win.destroy = () => {
    if (win.destroyed) return;
    win.destroyed = true;
    win.shown = false;
    win.emit("closed");
  };
  return win;
}

/** The two windows glued, with the timers run by hand. */
function pair({ down = () => false } = {}) {
  const frame = fakeWindow({ x: 100, y: 80, width: 1200, height: 800 });
  const page = fakeWindow({ x: 0, y: 0, width: 10, height: 10 }, { left: 0, top: 0, right: 0, bottom: 0 });
  const waiting = [];
  const made = glue({ frame, page, buttonDown: down, later: (f) => waiting.push(f) });
  const run = () => {
    let n = 0;
    while (waiting.length && n < 1000) {
      waiting.shift()();
      n += 1;
    }
    return n;
  };
  return { frame, page, made, waiting, run };
}

test("it's on when asked for on the command line, or kept on", () => {
  const store = (v) => ({ get: (key, fallback) => (key === KEY && v !== undefined ? v : fallback) });
  assert.equal(layerTrialOn(["CAD Protocol.exe"], store(undefined)), false);
  assert.equal(layerTrialOn(["CAD Protocol.exe", FLAG], store(undefined)), true);
  assert.equal(layerTrialOn(["CAD Protocol.exe"], store(true)), true);
  assert.equal(layerTrialOn(["CAD Protocol.exe"], store(false)), false);
  // Only true turns it on.
  assert.equal(layerTrialOn(["CAD Protocol.exe"], store("yes")), false);
  assert.equal(layerTrialOn(undefined, null), false);
});

test("Ctrl+Alt+Shift+L is its switch, pressed once", () => {
  const key = (more = {}) => ({ type: "keyDown", key: "L", code: "KeyL", control: true, alt: true, shift: true, meta: false, isAutoRepeat: false, ...more });
  assert.equal(isToggleChord(key()), true);
  assert.equal(isToggleChord(key({ key: "l" })), true);
  assert.equal(isToggleChord(key({ type: "keyUp" })), false);
  assert.equal(isToggleChord(key({ isAutoRepeat: true })), false);
  assert.equal(isToggleChord(key({ shift: false })), false);
  assert.equal(isToggleChord(key({ alt: false })), false);
  assert.equal(isToggleChord(key({ control: false })), false);
  assert.equal(isToggleChord(key({ key: "K", code: "KeyK" })), false);
  assert.equal(isToggleChord(null), false);
  assert.match(toggleWords(true).message, /is on/);
  assert.match(toggleWords(true).detail, /Ctrl\+Alt\+Shift\+L/);
  assert.match(toggleWords(false).message, /is off/);
});

test("the frame is the app's window as it was; the page's has no frame and is see-through", () => {
  const base = { x: 10, y: 20, width: 1440, height: 900, minWidth: 900, minHeight: 600, show: false, title: "CAD Protocol", backgroundColor: "#000000", webPreferences: { preload: "preload.js" } };
  const plain = { sandbox: true };
  const frame = frameOptions(base, plain);
  assert.equal(frame.title, "CAD Protocol");
  assert.equal(frame.minWidth, 900);
  assert.equal(frame.width, 1440);
  assert.equal(frame.frame, undefined);
  assert.equal(frame.transparent, undefined);
  // The layer's page gets no preload: it asks the computer for nothing.
  assert.deepEqual(frame.webPreferences, plain);
  const owner = { id: 1 };
  const page = pageOptions(base, owner, { x: 18, y: 51, width: 1424, height: 861 });
  assert.equal(page.parent, owner);
  assert.equal(page.frame, false);
  assert.equal(page.transparent, true);
  assert.equal(page.backgroundColor, "#00000000");
  assert.equal(page.resizable, false);
  assert.equal(page.movable, false);
  assert.equal(page.minimizable, false);
  assert.equal(page.maximizable, false);
  assert.equal(page.skipTaskbar, true);
  assert.equal(page.hasShadow, false);
  assert.equal(page.show, false);
  assert.equal(page.minWidth, undefined);
  assert.deepEqual([page.x, page.y, page.width, page.height], [18, 51, 1424, 861]);
  // The app's page keeps its preload.
  assert.deepEqual(page.webPreferences, { preload: "preload.js" });
});

test("the page's window is kept over the frame's inside", () => {
  const { frame, page } = pair();
  assert.deepEqual(page.getBounds(), { x: 108, y: 111, width: 1184, height: 761 });
  // Moved, sized, maximized: put there again.
  for (const [name, bounds] of [
    ["move", { x: 300, y: 200, width: 1200, height: 800 }],
    ["resize", { x: 300, y: 200, width: 1000, height: 700 }],
    ["maximize", { x: -8, y: -8, width: 1936, height: 1056 }],
    ["unmaximize", { x: 100, y: 80, width: 1200, height: 800 }],
    ["restore", { x: 120, y: 90, width: 1200, height: 800 }],
  ]) {
    frame.bounds = bounds;
    frame.emit(name);
    assert.deepEqual(page.getBounds(), frame.getContentBounds(), name);
  }
  // Already there: not set again.
  const sets = page.calls.filter((c) => c[0] === "setBounds").length;
  frame.emit("move");
  assert.equal(page.calls.filter((c) => c[0] === "setBounds").length, sets);
  // Minimized: left alone (Windows takes it away with the frame).
  frame.minimized = true;
  frame.bounds = { x: -32000, y: -32000, width: 160, height: 28 };
  frame.emit("move");
  assert.notDeepEqual(page.getBounds(), frame.getContentBounds());
});

test("what the app asks its window about its frame, the frame answers", () => {
  const { frame, page } = pair();
  page.maximize();
  assert.deepEqual(frame.calls.at(-1), ["maximize"]);
  assert.equal(page.isMaximized(), true);
  assert.deepEqual(page.getNormalBounds(), { x: 100, y: 80, width: 1200, height: 800, normal: true });
  frame.minimized = true;
  assert.equal(page.isMinimized(), true);
  page.restore();
  assert.equal(frame.minimized, false);
  page.flashFrame(true);
  assert.deepEqual(frame.calls.at(-1), ["flashFrame", true]);
  page.setAlwaysOnTop(true);
  assert.deepEqual(frame.calls.at(-1), ["setAlwaysOnTop", true]);
  for (const name of ["maximize", "isMaximized", "isMinimized", "restore", "getNormalBounds", "flashFrame", "setAlwaysOnTop"])
    assert.ok(FRAME_ANSWERS.includes(name), name);
});

test("shown and hidden together, and the frame's bar says the page's title", () => {
  const { frame, page } = pair();
  assert.equal(frame.isVisible(), false);
  page.show();
  assert.equal(frame.isVisible(), true);
  // Shown without taking the keys from the page.
  assert.deepEqual(frame.calls.filter((c) => c[0] === "showInactive").length, 1);
  page.hide();
  assert.equal(frame.isVisible(), false);
  page.emit("page-title-updated", {}, "CAD Protocol — Lot 7.cadp");
  assert.deepEqual(frame.calls.at(-1), ["setTitle", "CAD Protocol — Lot 7.cadp"]);
});

test("the keys go to the page once the frame's let go of", () => {
  let down = true;
  const { frame, page, waiting, run } = pair({ down: () => down });
  page.show();
  frame.focused = true;
  frame.emit("focus");
  // Held by its bar: waited for, never taken from the drag.
  for (let i = 0; i < 5; i += 1) waiting.shift()();
  assert.equal(page.calls.some((c) => c[0] === "focus"), false);
  assert.equal(waiting.length, 1);
  down = false;
  run();
  assert.deepEqual(page.calls.at(-1), ["focus"]);
  // In front by its taskbar button, nothing held: at once.
  page.calls.length = 0;
  frame.emit("focus");
  run();
  assert.deepEqual(page.calls.at(-1), ["focus"]);
  // Not while the page's window is out of sight (the sign-in card's up).
  page.calls.length = 0;
  page.shown = false;
  frame.emit("focus");
  run();
  assert.equal(page.calls.some((c) => c[0] === "focus"), false);
});

test("closed by the frame's bar: the page's window is asked first, and the frame goes when it has", () => {
  const { frame, page } = pair();
  page.show();
  // The app holds its window open (unsaved drawings): so does the frame.
  const hold = (event) => event.preventDefault();
  page.on("close", hold);
  frame.close();
  assert.deepEqual(page.calls.at(-1), ["close"]);
  assert.equal(frame.isDestroyed(), false);
  assert.equal(page.isDestroyed(), false);
  // Let go: both.
  page.off("close", hold);
  frame.close();
  assert.equal(page.isDestroyed(), true);
  assert.equal(frame.isDestroyed(), true);
});

test("the page's styles make only the drawing area see-through", () => {
  assert.match(SEE_THROUGH_CSS, /html, body \{ background: transparent !important; \}/);
  assert.match(SEE_THROUGH_CSS, /\*:has\(\.cpe-viewport\) \{ background-color: transparent !important;/);
  assert.match(SEE_THROUGH_CSS, /\.cpe-viewport > canvas:first-child \{ opacity: 0 !important; \}/);
  // Nothing hidden but the drawing's own background.
  assert.equal(/display:\s*none/.test(SEE_THROUGH_CSS), false);
});

test("the test picture is one of the app's own pages, and asks for nothing", () => {
  assert.equal(FILES.get("layer.html"), "text/html; charset=utf-8");
  const dir = path.join(__dirname, "..", "src");
  assert.deepEqual(fileFor("cadp://app/layer.html", dir), { file: path.join(dir, "layer.html"), type: "text/html; charset=utf-8" });
  const html = fs.readFileSync(path.join(dir, "layer.html"), "utf8");
  assert.match(html, /default-src 'none'/);
  assert.equal(/https?:\/\//.test(html), false);
});

test("a screen's pixels are read as red, green, blue", () => {
  // Two pixels in a row, BGRA: magenta, then green.
  const bitmap = Buffer.from([255, 0, 255, 255, 0, 255, 0, 255]);
  assert.deepEqual(pixelAt(bitmap, 2, 0, 0), [255, 0, 255]);
  assert.deepEqual(pixelAt(bitmap, 2, 1, 0), [0, 255, 0]);
  assert.equal(pixelAt(bitmap, 2, 5, 5), null);
  assert.equal(near([250, 6, 248], [255, 0, 255]), true);
  assert.equal(near([0, 255, 0], [255, 0, 255]), false);
  assert.equal(near(null, [0, 0, 0]), false);
});

test("the app's window is two only while the trial's on", () => {
  const main = fs.readFileSync(path.join(__dirname, "..", "src", "main.js"), "utf8");
  assert.match(main, /const trial = layerTrialOn\(process\.argv, store\);/);
  assert.match(main, /new BrowserWindow\(frame \? pageOptions\(options, frame, frame\.getContentBounds\(\)\) : options\)/);
  assert.match(main, /if \(trial\)\s+win\.webContents\.on\("dom-ready"/);
  assert.match(main, /result\.layerTrial = await checkLayerTrial\(/);
});

test("1.35: the frame's bar is kept as in front while the page's window has the keys", () => {
  const frame = fakeWindow({ x: 0, y: 0, width: 800, height: 600 });
  const page = fakeWindow({ x: 0, y: 0, width: 800, height: 600 });
  const handle = Buffer.alloc(8);
  handle.writeBigUInt64LE(66052n);
  frame.getNativeWindowHandle = () => handle;
  assert.equal(handleText(frame), "66052");
  assert.equal(handleText({ getNativeWindowHandle: () => null }), null);
  const painted = [];
  const waiting = [];
  assert.equal(keepBarActive({ frame, page, paint: (hwnd, active) => painted.push([hwnd, active]), later: (f) => waiting.push(f) }), true);
  // The page's window comes to the front: the frame's bar as the one in front.
  page.focused = true;
  page.emit("focus");
  assert.deepEqual(painted.at(-1), ["66052", true]);
  // Another app comes to the front: as Windows draws any window that isn't.
  page.focused = false;
  page.emit("blur");
  waiting.shift()();
  assert.deepEqual(painted.at(-1), ["66052", false]);
  // The frame loses the keys to the page's window (its bar clicked, then let go): still as in front.
  painted.length = 0;
  page.focused = true;
  frame.emit("blur");
  waiting.shift()();
  assert.deepEqual(painted.at(-1), ["66052", true]);
  // With no way to tell Windows, nothing's done.
  assert.equal(keepBarActive({ frame, page, paint: null }), false);
});

test("1.35: the page is told the trial's on, and the engine's window is started in the frame", () => {
  const main = fs.readFileSync(path.join(__dirname, "..", "src", "main.js"), "utf8");
  assert.match(main, /options\.webPreferences\.additionalArguments\.push\("--cadp-layer-trial=1"\);/);
  assert.match(main, /scanEngine\.viewIn\(hwnd\)/);
  assert.match(main, /keepBarActive\(\{ frame, page: win, paint: barBackend\(log\) \}\);/);
  const preload = fs.readFileSync(path.join(__dirname, "..", "src", "preload.js"), "utf8");
  assert.match(preload, /layer: \{ on: arg\("cadp-layer-trial"\) === "1" \},/);
});
