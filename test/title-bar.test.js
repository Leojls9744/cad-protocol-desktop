"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const {
  CAPTION,
  DWMWA_BORDER_COLOR,
  DWMWA_CAPTION_COLOR,
  DWMWA_TEXT_COLOR,
  colorRef,
  createTitleBar,
  hwndOf,
  windowsBackend,
} = require("../src/title-bar");

function fakeDwm(answer = true) {
  const calls = [];
  return { calls, backend: { supported: true, set: (h, a, v) => (calls.push([h, a, v]), answer) } };
}

function fakeWindow(hwnd = 0x1234n) {
  const buf = Buffer.alloc(8);
  buf.writeBigUInt64LE(hwnd);
  return { getNativeWindowHandle: () => buf };
}

test("colors become Windows COLORREFs (0x00BBGGRR)", () => {
  assert.equal(colorRef("#4c1fc4"), 0x00c41f4c);
  assert.equal(colorRef("#ffffff"), 0x00ffffff);
  assert.equal(colorRef("#000000"), 0);
  assert.throws(() => colorRef("purple"));
});

test("the default bar is the app's deep purple", () => {
  assert.equal(CAPTION, "#4c1fc4");
});

test("the HWND is read from Electron's handle buffer", () => {
  assert.equal(hwndOf(fakeWindow(0xabcdefn).getNativeWindowHandle()), 0xabcdefn);
  const small = Buffer.alloc(4);
  small.writeUInt32LE(77);
  assert.equal(hwndOf(small), 77n);
  assert.equal(hwndOf(null), null);
});

test("paint sets the caption, text and border colors on the window", () => {
  const dwm = fakeDwm();
  const bar = createTitleBar({ backend: dwm.backend });
  assert.equal(bar.paint(fakeWindow(42n)), true);
  assert.deepEqual(dwm.calls, [
    [42n, DWMWA_CAPTION_COLOR, 0x00c41f4c],
    [42n, DWMWA_TEXT_COLOR, 0x00ffffff],
    [42n, DWMWA_BORDER_COLOR, 0x00c41f4c],
  ]);
});

test("Windows 10 (call refused) logs once and leaves the bar alone", () => {
  const dwm = fakeDwm(false);
  const lines = [];
  const bar = createTitleBar({ backend: dwm.backend, log: (l) => lines.push(l) });
  assert.equal(bar.paint(fakeWindow()), false);
  assert.equal(bar.paint(fakeWindow()), false);
  assert.equal(lines.length, 1);
});

test("a throwing call never breaks window creation", () => {
  const lines = [];
  const backend = {
    supported: true,
    set: () => {
      throw new Error("boom");
    },
  };
  const bar = createTitleBar({ backend, log: (l) => lines.push(l) });
  assert.equal(bar.paint(fakeWindow()), false);
  assert.match(lines[0], /boom/);
});

test("off Windows nothing is called", () => {
  if (process.platform === "win32") return;
  const backend = windowsBackend();
  assert.equal(backend.supported, false);
  const bar = createTitleBar({ backend });
  assert.equal(bar.paint(fakeWindow()), false);
});
