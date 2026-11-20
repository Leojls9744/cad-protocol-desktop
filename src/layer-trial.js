"use strict";
// The layer trial (1.34; Leo, 2026-11-20 12:08: "i want to transfer
// everything to the desktop app", and 12:11, "lets start": stage 1's
// Trial A).
//
// The plan's first stage is the scans drawn by the app's own engine instead
// of the page. For that the engine's picture has to sit UNDER the page, so
// the crosshair, grips, menus and cards still show on top of it, and that's
// the piece that had never been tried on a real computer. Windows puts a
// window of an app's own over anything its page draws, never under it; so
// the trial turns the app's window into two, one over the other:
//
// - The frame: an ordinary window, with Windows' own title bar, edges,
//   snapping and taskbar button. What's drawn in it is the layer. In this
//   trial that's a test picture (layer.html); in Trial B, the engine's scan.
// - The page: the app's page as it is, in a window with no frame of its own
//   and a see-through background, owned by the frame (so Windows keeps it in
//   front of it and takes it away and back with it) and kept exactly over
//   the frame's inside as the frame moves, sizes, maximizes or comes back.
//
// The page's drawing area is made see-through by four style rules given to
// the page as it loads (SEE_THROUGH_CSS): every box the drawing area sits in
// loses its background, and the canvas that paints the drawing's background
// and grid isn't shown. The ribbon, the cards, the tabs and the status bar
// have backgrounds of their own and stay as they are. Nothing in the web app
// changes for the trial.
//
// It's off unless it's turned on: Ctrl+Alt+Shift+L in the app's window says
// so and keeps it (desktop-settings.json's layerTrial), for the next time the
// app opens; the same keys turn it off. Started with --layer-trial, it's on
// for that run. Off, the app's window is made exactly as it was.
//
// Everything of the app's that asks its window about its frame (maximize,
// minimized or not, restore, where it was, flash) is answered by the frame
// (routeToFrame), so the rest of the app needs to know nothing of the trial.

/** On for this run. */
const FLAG = "--layer-trial";
/** Kept in the app's settings: on the next time it opens. */
const KEY = "layerTrial";

/**
 * The page's drawing area made see-through (tried on the web app with a
 * stand-in for the layer: only the drawing area shows it). Every box round
 * a drawing area, up to the page itself, without a background; the drawing
 * area's own; and its first canvas, which paints the drawing's background
 * and grid.
 */
const SEE_THROUGH_CSS = [
  "html, body { background: transparent !important; }",
  "*:has(.cpe-viewport) { background-color: transparent !important; background-image: none !important; }",
  ".cpe-viewport { background: transparent !important; }",
  ".cpe-viewport > canvas:first-child { opacity: 0 !important; }",
].join("\n");

/** Whether the trial's on: asked for on the command line, or kept on. */
function layerTrialOn(argv, store) {
  if (Array.isArray(argv) && argv.includes(FLAG)) return true;
  return Boolean(store && store.get(KEY, false) === true);
}

/** Ctrl+Alt+Shift+L pressed (Electron's before-input-event), once. */
function isToggleChord(input) {
  if (!input || input.type !== "keyDown" || input.isAutoRepeat) return false;
  if (!input.control || !input.alt || !input.shift || input.meta) return false;
  return String(input.key || "").toLowerCase() === "l" || input.code === "KeyL";
}

/** What Ctrl+Alt+Shift+L says once it's kept. */
function toggleWords(on) {
  return {
    message: on ? "The layer trial is on." : "The layer trial is off.",
    detail: on
      ? "Close CAD Protocol and open it again: its window opens with a test picture under the drawing area. Ctrl+Alt+Shift+L turns the trial off again."
      : "Close CAD Protocol and open it again: its window opens as usual.",
  };
}

/**
 * The frame's options from the app's window's (`base`): the same place,
 * size, least size and title, Windows' own frame, no preload (its page is
 * the app's own layer.html and asks for nothing).
 */
function frameOptions(base, webPreferences) {
  const { webPreferences: _page, ...rest } = base;
  return { ...rest, show: false, backgroundColor: "#000000", webPreferences };
}

/**
 * The page's window's options from the app's window's (`base`): no frame, a
 * see-through background, owned by `frame`, and nothing a person can do to
 * it by itself (sized, moved, minimized and maximized only with the frame).
 */
function pageOptions(base, frame, bounds) {
  const { minWidth: _w, minHeight: _h, backgroundColor: _b, x: _x, y: _y, width: _wd, height: _ht, ...rest } = base;
  return {
    ...rest,
    ...bounds,
    parent: frame,
    show: false,
    frame: false,
    transparent: true,
    backgroundColor: "#00000000",
    hasShadow: false,
    thickFrame: false,
    resizable: false,
    movable: false,
    minimizable: false,
    maximizable: false,
    fullscreenable: false,
    skipTaskbar: true,
  };
}

/** The frame's events after which the page's window is put over its inside again. */
const PLACE_ON = ["move", "moved", "resize", "resized", "maximize", "unmaximize", "restore", "show", "enter-full-screen", "leave-full-screen"];

/** What the app asks its window about its frame: answered by the frame. */
const FRAME_ANSWERS = ["maximize", "unmaximize", "isMaximized", "minimize", "isMinimized", "restore", "getNormalBounds", "flashFrame", "setAlwaysOnTop"];

/** `page`'s frame questions answered by `frame` from now on. */
function routeToFrame(page, frame) {
  for (const name of FRAME_ANSWERS) {
    if (typeof frame[name] !== "function") continue;
    page[name] = (...args) => frame[name](...args);
  }
}

/**
 * The two windows kept as one. `buttonDown()` says whether the mouse's left
 * button is held (the frame's being dragged by its bar or an edge: the
 * page's window isn't given the keys till it's let go, which would end the
 * drag); `later(f, ms)` calls back in a while.
 */
function glue({ frame, page, buttonDown = () => false, later = setTimeout, log = () => {} }) {
  const gone = (win) => !win || win.isDestroyed();
  let closing = false;

  /** The page's window over the frame's inside. */
  function place() {
    if (gone(frame) || gone(page) || frame.isMinimized()) return false;
    const want = frame.getContentBounds();
    const now = page.getBounds();
    if (now.x === want.x && now.y === want.y && now.width === want.width && now.height === want.height) return false;
    page.setBounds(want);
    return true;
  }
  for (const name of PLACE_ON) frame.on(name, () => place());

  // The keys go to the page: the frame's only ever in front for a moment
  // (its bar clicked, its taskbar button, Alt+Tab).
  let waits = 0;
  function giveKeys() {
    if (gone(frame) || gone(page) || closing) return;
    if (!page.isVisible() || frame.isMinimized()) return;
    if (buttonDown() && waits < 400) {
      waits += 1;
      later(giveKeys, 30);
      return;
    }
    waits = 0;
    place();
    if (frame.isFocused()) page.focus();
  }
  frame.on("focus", () => {
    waits = 0;
    later(giveKeys, 0);
  });

  // Closed by its bar: the page's window is asked, as it is without the
  // trial (unsaved drawings, signing out), and the frame goes when it has.
  frame.on("close", (event) => {
    if (gone(page)) return;
    event.preventDefault();
    page.close();
  });
  page.on("closed", () => {
    closing = true;
    if (!gone(frame)) frame.destroy();
  });

  // Shown and hidden together.
  page.on("show", () => {
    if (gone(frame)) return;
    if (!frame.isVisible()) frame.showInactive();
    place();
  });
  page.on("hide", () => {
    if (!gone(frame) && !closing && frame.isVisible()) frame.hide();
  });

  // The frame's bar says what the page's would.
  page.on("page-title-updated", (_event, title) => {
    if (!gone(frame) && title) frame.setTitle(title);
  });

  routeToFrame(page, frame);
  place();
  log("layer trial: the app's window is two, the page over a layer");
  return { frame, page, place, giveKeys };
}

/** A window's handle as a whole number's text (Electron's getNativeWindowHandle() buffer); null with none. */
function handleText(win) {
  try {
    const b = win.getNativeWindowHandle();
    if (!b || typeof b.length !== "number") return null;
    if (b.length >= 8) return String(b.readBigUInt64LE(0));
    if (b.length >= 4) return String(b.readUInt32LE(0));
  } catch {
    // No window.
  }
  return null;
}

/**
 * The frame's bar kept looking in front while the page's window has the
 * keys (1.35; Leo's screenshots of 2026-11-20 14:10: the bar dark, as
 * Windows draws a window that isn't in front, all the while the trial was
 * on): Windows is told to draw the frame's bar as the active one's when the
 * page's window comes to the front, and as it would when neither is.
 * `paint(hwndText, active)` is Windows' call (barBackend); the windows'
 * focus and blur say when.
 */
function keepBarActive({ frame, page, paint, later = setTimeout }) {
  const gone = (win) => !win || win.isDestroyed();
  const hwnd = handleText(frame);
  if (!hwnd || typeof paint !== "function") return false;
  const set = (active) => {
    if (!gone(frame)) paint(hwnd, active);
  };
  page.on("focus", () => set(true));
  // Gone from both (another app's in front): as Windows draws any window that isn't.
  const lost = () =>
    later(() => {
      if (gone(frame) || gone(page)) return;
      if (!page.isFocused() && !frame.isFocused()) set(false);
      else if (page.isFocused()) set(true);
    }, 40);
  page.on("blur", lost);
  frame.on("blur", lost);
  return true;
}

/** Windows' call for keepBarActive: WM_NCACTIVATE sent to the frame (koffi); nothing anywhere else. */
function barBackend(log = () => {}) {
  if (process.platform !== "win32") return null;
  try {
    const koffi = require("koffi");
    const user = koffi.load("user32.dll");
    const send = user.func("__stdcall", "SendMessageW", "intptr_t", ["uintptr_t", "uint32", "uintptr_t", "intptr_t"]);
    const WM_NCACTIVATE = 0x0086;
    return (hwndText, active) => {
      try {
        // lParam 0: the bar's drawn again in its new look at once.
        send(BigInt(hwndText), WM_NCACTIVATE, active ? 1 : 0, 0);
      } catch (err) {
        log(`layer trial: the frame's bar couldn't be told: ${err && err.message ? err.message : err}`);
      }
    };
  } catch (err) {
    log(`layer trial: the frame's bar can't be kept as in front: ${err && err.message ? err.message : err}`);
    return null;
  }
}

/** Whether the mouse's left button is held, from Windows (koffi); never, anywhere else. */
function buttonDownBackend(log = () => {}) {
  if (process.platform !== "win32") return () => false;
  try {
    const koffi = require("koffi");
    const user = koffi.load("user32.dll");
    const state = user.func("__stdcall", "GetAsyncKeyState", "int16", ["int32"]);
    const VK_LBUTTON = 0x01;
    return () => (state(VK_LBUTTON) & 0x8000) !== 0;
  } catch (err) {
    log(`layer trial: the mouse's buttons can't be read: ${err && err.message ? err.message : err}`);
    return () => false;
  }
}

/** A pixel of a bitmap (BGRA rows, as NativeImage.toBitmap gives), as [r, g, b]. */
function pixelAt(bitmap, width, x, y) {
  const i = (Math.round(y) * width + Math.round(x)) * 4;
  if (i < 0 || i + 2 >= bitmap.length) return null;
  return [bitmap[i + 2], bitmap[i + 1], bitmap[i]];
}

/** Whether a pixel is near a color, each part within `slack`. */
function near(pixel, color, slack = 40) {
  return Boolean(pixel) && pixel.every((v, k) => Math.abs(v - color[k]) <= slack);
}

/**
 * The build's check on real Windows (--smoke-test): the two windows made
 * with stand-in pages (a magenta layer; a page with a green bar down its
 * left, a white line across, and the rest see-through), moved, sized and
 * maximized with the frame, and the screen looked at: the bar and the line
 * over the layer, the layer through the rest. The screen's picture is kept
 * at `png` for the build to keep.
 */
async function checkLayerTrial({ BrowserWindow, screen, desktopCapturer, writeFile, png, log = () => {}, engine = null }) {
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const display = screen.getPrimaryDisplay();
  const area = display.workArea;
  const seen = { area, scale: display.scaleFactor };
  const base = { x: area.x + 40, y: area.y + 40, width: 900, height: 600, title: "Layer check" };
  const prefs = { contextIsolation: true, sandbox: true, nodeIntegration: false };
  const frame = new BrowserWindow(frameOptions(base, prefs));
  let page = null;
  try {
    await frame.loadURL("data:text/html,<title>Layer check</title><body style='margin:0;background:%23ff00ff'>");
    page = new BrowserWindow(pageOptions({ ...base, webPreferences: prefs }, frame, frame.getContentBounds()));
    const pair = glue({ frame, page, buttonDown: buttonDownBackend(log), log });
    await page.loadURL(
      "data:text/html,<title>Layer check</title><body style='margin:0;background:transparent'>" +
        "<div style='position:fixed;left:0;top:0;bottom:0;width:200px;background:%2300ff00'></div>" +
        "<div style='position:fixed;left:200px;right:0;top:300px;height:6px;background:%23ffffff'></div>",
    );
    frame.show();
    page.show();
    await wait(1200);
    seen.owned = page.getParentWindow() === frame;
    seen.first = { page: page.getBounds(), inside: frame.getContentBounds() };
    // Moved and sized by the frame.
    frame.setBounds({ x: area.x + 90, y: area.y + 70, width: 1000, height: 640 });
    await wait(900);
    pair.place();
    seen.moved = { page: page.getBounds(), inside: frame.getContentBounds() };
    // The screen: the bar, the layer through the page, the line over the layer.
    const inside = frame.getContentBounds();
    const k = display.scaleFactor || 1;
    /** The screen now: `at(dx, dy)` a pixel of it from the frame's inside's corner, and its picture kept at `file`. */
    const look = async (file) => {
      const sources = await desktopCapturer.getSources({
        types: ["screen"],
        thumbnailSize: { width: Math.round(display.size.width * k), height: Math.round(display.size.height * k) },
      });
      const shot = sources[0] && sources[0].thumbnail;
      const size = shot ? shot.getSize() : { width: 0, height: 0 };
      if (!shot || !(size.width > 0)) return null;
      const sx = size.width / display.size.width;
      const sy = size.height / display.size.height;
      const bitmap = shot.toBitmap();
      if (file && writeFile) writeFile(file, shot.toPNG());
      return {
        size,
        at: (dx, dy) => pixelAt(bitmap, size.width, (inside.x - display.bounds.x + dx) * sx, (inside.y - display.bounds.y + dy) * sy),
      };
    };
    try {
      const shot = await look(png);
      seen.shot = shot ? shot.size : { width: 0, height: 0 };
      if (shot) {
        seen.pixels = { bar: shot.at(100, 100), layer: shot.at(520, 120), line: shot.at(520, 303), below: shot.at(520, 420) };
        seen.barOver = near(seen.pixels.bar, [0, 255, 0]);
        seen.layerThrough = near(seen.pixels.layer, [255, 0, 255]) && near(seen.pixels.below, [255, 0, 255]);
        seen.lineOver = near(seen.pixels.line, [255, 255, 255]);
        if (png) seen.png = png;
      }
    } catch (err) {
      seen.shotError = String(err && err.message ? err.message : err);
    }
    // 1.35 (Trial B): the scan engine's own window in the frame. Its ring of
    // points, turning, then the check's scan from overhead: each seen on the
    // screen through the page's see-through part, with the page's bar and
    // line still over it.
    if (engine) seen.view = await checkEngineView({ engine, frame, inside, k, look, png, wait, log });
    // Maximized with the frame, and back.
    frame.maximize();
    await wait(900);
    pair.place();
    seen.maximized = { page: page.getBounds(), inside: frame.getContentBounds(), is: page.isMaximized() };
    frame.unmaximize();
    await wait(600);
    // Closed by the frame's bar: the page's window first, then the frame.
    frame.close();
    for (let i = 0; i < 40 && !(frame.isDestroyed() && page.isDestroyed()); i += 1) await wait(100);
    seen.closed = frame.isDestroyed() && page.isDestroyed();
    return seen;
  } catch (err) {
    seen.error = String(err && err.stack ? err.stack : err);
    return seen;
  } finally {
    if (page && !page.isDestroyed()) page.destroy();
    if (!frame.isDestroyed()) frame.destroy();
  }
}

/** The engine's own background (cadp-engine-view CLEAR), as the screen shows it. */
const ENGINE_CLEAR = [11, 16, 32];

/**
 * The build's check of the engine's own window (1.35, Trial B) on real
 * Windows, inside checkLayerTrial's two windows: started in the frame, its
 * ring of points drawn (white at its middle, the engine's background round
 * it, where the frame's page was magenta), then the check's scan from
 * overhead (its post red at the middle, its floor blue beside it), and how
 * it went (frames drawn, the card's name).
 */
async function checkEngineView({ engine, frame, inside, k, look, png, wait, log = () => {} }) {
  const out = {};
  try {
    const hwnd = handleText(frame);
    out.hwnd = Boolean(hwnd);
    out.told = engine.engine.viewIn(hwnd);
    // The window, the card and its first frame (the build's computer draws on its processor).
    let stats = null;
    for (let i = 0; i < 80; i += 1) {
      await wait(250);
      const said = await engine.ask({ kind: "view", ask: { test: true } });
      if (said.error && !/isn't started/.test(said.error)) {
        out.error = said.error;
        break;
      }
      stats = said.stats || null;
      if (stats && stats.error) {
        out.error = stats.error;
        break;
      }
      if (stats && stats.frames >= 3) break;
    }
    out.ring = stats;
    if (out.error || !stats || !(stats.frames >= 3)) return out;
    await wait(600);
    const cx = Math.round(inside.width / 2);
    const cy = Math.round(inside.height / 2);
    const first = await look(png ? png.replace(/\.png$/, ".ring.png") : null);
    if (first) {
      out.ringPixels = { middle: first.at(cx, cy), beside: first.at(cx + 60, cy + 40), bar: first.at(100, 100), line: first.at(520, 303) };
      out.ringDrawn = near(out.ringPixels.middle, [255, 255, 255]);
      out.engineUnder = near(out.ringPixels.beside, ENGINE_CLEAR, 12);
      out.pageOver = near(out.ringPixels.bar, [0, 255, 0]) && near(out.ringPixels.line, [255, 255, 255]);
    }
    // The check's scan from overhead: 10 × 10 units of floor at 0.03 a pixel, its post at the middle.
    const shown = await engine.ask({
      kind: "view",
      scans: [{ token: engine.token, affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0] }],
      ask: {
        target: [6000005, 2000005, 100],
        right: [1, 0, 0],
        up: [0, 1, 0],
        toEye: [0, 0, 1],
        unitsPerPixel: 0.03 / k,
        rect: [0, 0, inside.width * k, inside.height * k],
        px: 5 * k,
      },
    });
    out.shown = shown.shown;
    if (shown.error) {
      out.error = shown.error;
      return out;
    }
    let scan = null;
    for (let i = 0; i < 40; i += 1) {
      await wait(250);
      scan = (await engine.ask({ kind: "view-stats" })).stats || null;
      if (scan && scan.points >= 10000) break;
    }
    out.scan = scan;
    await wait(600);
    const second = await look(png ? png.replace(/\.png$/, ".scan.png") : null);
    if (second) {
      out.scanPixels = { post: second.at(cx, cy), floor: second.at(cx - 100, cy + 30), off: second.at(cx + 230, cy), line: second.at(520, 303) };
      out.postDrawn = near(out.scanPixels.post, [225, 70, 50], 60);
      out.floorDrawn = near(out.scanPixels.floor, [40, 80, 220], 40);
      out.offClear = near(out.scanPixels.off, ENGINE_CLEAR, 12);
    }
    return out;
  } catch (err) {
    out.error = String(err && err.stack ? err.stack : err);
    return out;
  } finally {
    try {
      engine.engine.viewIn(null);
      await wait(300);
    } catch (err) {
      log(`layer trial: the engine's window wasn't stopped: ${err && err.message ? err.message : err}`);
    }
  }
}

module.exports = {
  ENGINE_CLEAR,
  checkEngineView,
  FLAG,
  KEY,
  SEE_THROUGH_CSS,
  PLACE_ON,
  FRAME_ANSWERS,
  layerTrialOn,
  isToggleChord,
  toggleWords,
  frameOptions,
  pageOptions,
  routeToFrame,
  glue,
  handleText,
  keepBarActive,
  barBackend,
  buttonDownBackend,
  pixelAt,
  near,
  checkLayerTrial,
};
