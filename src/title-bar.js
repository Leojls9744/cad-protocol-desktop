"use strict";
// A purple title bar.
//
// Windows paints a window's title bar in the person's accent color (often
// teal). CAD Protocol's own color is purple, so every CAD Protocol window asks
// Windows for a purple bar with white text and a purple edge instead. This is
// Windows 11 only (build 22000 and later); Windows 10 ignores the request and
// keeps its usual bar, which is fine.
//
// Only the color changes: the bar, its buttons and snapping are still
// Windows' own.

/** The deep purple the app's buttons start from. */
const CAPTION = "#4c1fc4";
const TEXT = "#ffffff";
const BORDER = "#4c1fc4";

/** DwmSetWindowAttribute's attributes (dwmapi.h). */
const DWMWA_BORDER_COLOR = 34;
const DWMWA_CAPTION_COLOR = 35;
const DWMWA_TEXT_COLOR = 36;

/** "#rrggbb" as a Windows COLORREF, 0x00BBGGRR. */
function colorRef(hex) {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(String(hex));
  if (!m) throw new Error(`not a #rrggbb color: ${hex}`);
  const [r, g, b] = [m[1], m[2], m[3]].map((h) => parseInt(h, 16));
  return ((b << 16) | (g << 8) | r) >>> 0;
}

/** The window's HWND from Electron's getNativeWindowHandle() buffer. */
function hwndOf(buffer) {
  if (!buffer || typeof buffer.length !== "number") return null;
  if (buffer.length >= 8) return buffer.readBigUInt64LE(0);
  if (buffer.length >= 4) return BigInt(buffer.readUInt32LE(0));
  return null;
}

const unsupported = Object.freeze({ supported: false, set: () => false });

/** The Windows call, through koffi. Anywhere else, or if koffi can't load, the bar is left alone. */
function windowsBackend(log = () => {}) {
  if (process.platform !== "win32") return unsupported;
  try {
    const koffi = require("koffi");
    const dwm = koffi.load("dwmapi.dll");
    const setAttribute = dwm.func("__stdcall", "DwmSetWindowAttribute", "int32", [
      "uintptr_t",
      "uint32",
      "void *",
      "uint32",
    ]);
    return {
      supported: true,
      /** True when Windows took it (S_OK). */
      set(hwnd, attribute, value) {
        return setAttribute(hwnd, attribute, Uint32Array.of(value), 4) === 0;
      },
    };
  } catch (err) {
    log(`title bar color unavailable: ${err && err.message ? err.message : err}`);
    return unsupported;
  }
}

/** Paints each window's bar; says once in the log if Windows won't. */
function createTitleBar({ backend, log = () => {}, colors = {} }) {
  const wanted = [
    [DWMWA_CAPTION_COLOR, colorRef(colors.caption || CAPTION)],
    [DWMWA_TEXT_COLOR, colorRef(colors.text || TEXT)],
    [DWMWA_BORDER_COLOR, colorRef(colors.border || BORDER)],
  ];
  let told = false;
  return {
    /** `win` is a BrowserWindow (anything with getNativeWindowHandle()). */
    paint(win) {
      if (!backend.supported || !win || typeof win.getNativeWindowHandle !== "function") return false;
      try {
        const hwnd = hwndOf(win.getNativeWindowHandle());
        if (hwnd === null) return false;
        let all = true;
        for (const [attribute, value] of wanted) {
          if (!backend.set(hwnd, attribute, value)) all = false;
        }
        if (!all && !told) {
          told = true;
          log("title bar color not taken (Windows 11 is needed); the usual bar stays");
        }
        return all;
      } catch (err) {
        if (!told) {
          told = true;
          log(`title bar color failed: ${err && err.message ? err.message : err}`);
        }
        return false;
      }
    },
  };
}

module.exports = {
  CAPTION,
  TEXT,
  BORDER,
  DWMWA_BORDER_COLOR,
  DWMWA_CAPTION_COLOR,
  DWMWA_TEXT_COLOR,
  colorRef,
  hwndOf,
  windowsBackend,
  createTitleBar,
};
