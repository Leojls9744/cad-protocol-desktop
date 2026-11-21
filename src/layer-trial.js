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
// The way to the layer is opened by two style rules given to the page as it
// loads (SEE_THROUGH_CSS): every box the drawing area sits in, up to the page
// itself, loses its background. The drawing area's own background is the
// page's to give up (1.35; the web app's v942 on, told the trial's on by
// window.cadpDesktop.layer): see-through in model space, where the engine
// draws the background and the scans, and its own as ever on a layout. (1.34
// took the drawing area's background and its grid's canvas away itself, for
// a page that knew nothing of the trial; a page from before v942 now simply
// covers the layer.) The ribbon, the cards, the tabs and the status bar have
// backgrounds of their own and stay as they are.
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
 * The way from the page's drawing area down to the layer opened: every box
 * round a drawing area, up to the page itself, without a background. The
 * drawing area's own background is the page's to give up (the web app's
 * v942 on): see-through in model space, its own on a layout.
 */
const SEE_THROUGH_CSS = [
  "html, body { background: transparent !important; }",
  "*:has(.cpe-viewport) { background-color: transparent !important; background-image: none !important; }",
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
  // No menu bar, as the app's frame has none: the frame's inside is all the engine's window fills.
  if (typeof frame.removeMenu === "function") frame.removeMenu();
  let page = null;
  try {
    await frame.loadURL("data:text/html,<title>Layer check</title><body style='margin:0;background:%23ff00ff'>");
    page = new BrowserWindow(pageOptions({ ...base, webPreferences: prefs }, frame, frame.getContentBounds()));
    const pair = glue({ frame, page, buttonDown: buttonDownBackend(log), log });
    await page.loadURL(
      "data:text/html,<title>Layer check</title><body style='margin:0;background:transparent'>" +
        "<div style='position:fixed;left:0;top:0;bottom:0;width:200px;background:%2300ff00'></div>" +
        // The line a third of the way down: clear of the inside's middle, where the engine's ring and scan are looked at (1.35).
        "<div style='position:fixed;left:200px;right:0;top:200px;height:6px;background:%23ffffff'></div>",
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
        seen.pixels = { bar: shot.at(100, 100), layer: shot.at(520, 120), line: shot.at(520, 203), below: shot.at(520, 420) };
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
/** The background the check asks for behind its scan (a page's own). */
const SCAN_CLEAR = [51, 26, 77];
/** The page's elevation colours' ends (the engine's RAMP), and the colour the check gives the floor's class. */
const RAMP_LOW = [59, 82, 139];
const RAMP_HIGH = [189, 223, 38];
const CLASS_FLOOR = [200, 120, 40];

/**
 * The build's check of the engine's own window (1.35, Trial B) on real
 * Windows, inside checkLayerTrial's two windows: started in the frame, its
 * ring of points drawn (white at its middle, the engine's background round
 * it, where the frame's page was magenta), then the check's scan from
 * overhead: by height (its post's top the high colour at the middle, its
 * floor the low one beside it), by class with the post's layer off, with
 * eye-dome lighting, shaded as the page shades (1.37), by a limit box and
 * by a slice, faint, with its gaps filled, and in two views at once (1.37),
 * and kept undrawn;
 * and how it went (frames drawn, the card's name).
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
      out.ringPixels = { middle: first.at(cx, cy), beside: first.at(cx + 60, cy + 40), bar: first.at(100, 100), line: first.at(520, 203) };
      out.ringDrawn = near(out.ringPixels.middle, [255, 255, 255]);
      out.engineUnder = near(out.ringPixels.beside, ENGINE_CLEAR, 12);
      out.pageOver = near(out.ringPixels.bar, [0, 255, 0]) && near(out.ringPixels.line, [255, 255, 255]);
    }
    // The check's scan from overhead: 10 × 10 units of floor at 0.03 a pixel, its post at the middle.
    const affine = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0];
    const view = {
      target: [6000005, 2000005, 100],
      right: [1, 0, 0],
      up: [0, 1, 0],
      toEye: [0, 0, 1],
      unitsPerPixel: 0.03 / k,
      rect: [0, 0, inside.width * k, inside.height * k],
      // A background of the page's own choosing.
      clear: SCAN_CLEAR.map((v) => v / 255),
    };
    /** Shown, and looked at once a frame has shown it (the page's number for the ask said back). */
    const showAndLook = async (serial, scan, ask, file) => {
      const said = await engine.ask({ kind: "view", scans: [{ token: engine.token, affine, ...scan }], ask: { ...view, ...ask, serial } });
      if (said.error) throw new Error(said.error);
      let stats = said.stats || null;
      for (let i = 0; i < 60 && !(stats && stats.drawn === serial && (ask.hide || stats.scans >= 1)); i += 1) {
        await wait(250);
        stats = (await engine.ask({ kind: "view-stats" })).stats || null;
      }
      await wait(600);
      return { shown: said.shown, stats, picture: await look(file) };
    };
    // By height, over its own 100 to 104 (1.36: the page says the colours' span): the floor the page's low colour, the post's top its high one.
    const heights = await showAndLook(7, { look: { by: 2, px: 5 * k, heights: [100, 104] } }, {}, png ? png.replace(/\.png$/, ".scan.png") : null);
    out.shown = heights.shown;
    out.scan = heights.stats;
    if (heights.picture) {
      const second = heights.picture;
      out.scanPixels = { post: second.at(cx, cy), floor: second.at(cx - 100, cy + 30), off: second.at(cx + 230, cy), line: second.at(520, 203) };
      out.postDrawn = near(out.scanPixels.post, RAMP_HIGH, 30);
      out.floorDrawn = near(out.scanPixels.floor, RAMP_LOW, 30);
      out.offClear = near(out.scanPixels.off, SCAN_CLEAR, 12);
    }
    // By class (1.36): the floor (class 2) the colour its layer has; the post (class 5) with its layer off isn't there, so the floor shows where it stood.
    const table = new Array(256).fill((175 | (175 << 8) | (175 << 16) | (255 << 24)) >>> 0);
    table[2] = (CLASS_FLOOR[0] | (CLASS_FLOOR[1] << 8) | (CLASS_FLOOR[2] << 16) | (255 << 24)) >>> 0;
    table[5] = 255;
    const classes = await showAndLook(8, { look: { by: 1, px: 5 * k, classes: table } }, {}, null);
    out.classes = classes.stats;
    if (classes.picture) {
      out.classPixels = { post: classes.picture.at(cx, cy), floor: classes.picture.at(cx - 100, cy + 30) };
      out.classDrawn = near(out.classPixels.floor, CLASS_FLOOR, 12) && near(out.classPixels.post, CLASS_FLOOR, 12);
    }
    // Eye-dome lighting on (1.36): the floor beside the post darkens, the floor away from it doesn't.
    const lit = await showAndLook(9, { look: { by: 1, px: 5 * k, classes: table.map((v, c) => (c === 5 ? (255 | (255 << 24)) >>> 0 : v)) } }, { edl: [1, 1.4 * k, inside.width * k * 0.055, 0] }, null);
    if (lit.picture) {
      // The floor's pixels just left of the post (whichever of them the card gives the post's own edge to).
      const beside = [];
      for (let dx = -9; dx <= -2; dx += 1) beside.push(lit.picture.at(cx + dx, cy));
      out.litPixels = { beside, away: lit.picture.at(cx - 100, cy + 30) };
      const floorDarkened = (p) => p[0] < CLASS_FLOOR[0] - 40 && p[1] > 30 && p[1] < CLASS_FLOOR[1] - 20;
      out.litDrawn = near(out.litPixels.away, CLASS_FLOOR, 12) && beside.some(floorDarkened);
    }
    // Shaded as the page shades (1.37): its points lit by the shape of their surface (level ground stays its
    // colour), depth shading (the floor at the post's foot in a fold: darker) and face shading (the post's own
    // pixels stand steep against Z: darker); the floor away from the post as it was.
    const postOn = table.map((v, c) => (c === 5 ? (175 | (175 << 8) | (175 << 16) | (255 << 24)) >>> 0 : v));
    const shaded = await showAndLook(11, { look: { by: 1, px: 5 * k, classes: postOn, shade: [100, 100, 100] } }, { shade: [0.14, 0, 0.6] }, png ? png.replace(/\.png$/, ".shade.png") : null);
    out.shaded = shaded.stats;
    if (shaded.picture) {
      const beside = [];
      for (let dx = -9; dx <= -2; dx += 1) beside.push(shaded.picture.at(cx + dx, cy));
      out.shadePixels = { beside, post: shaded.picture.at(cx, cy), away: shaded.picture.at(cx - 100, cy + 30) };
      const inFold = (p) => p[0] < CLASS_FLOOR[0] - 90 && p[0] > p[2];
      out.shadeDrawn = near(out.shadePixels.away, CLASS_FLOOR, 12) && beside.some(inFold) && out.shadePixels.post[0] < 130 && out.shadePixels.post[0] > 30;
    }
    // A limit box over the floor's west 4 units (1.37): the floor inside it, the page's background east of it (the post too);
    // then a slice's slab through the post's top alone: the post, and no floor.
    const whole = (min, max) => ({ min, max, centre: [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2], cos: 1, sin: 0, turned: false, outside: false, slack: [1e-6, 1e-6, 1e-6] });
    const byClass = { look: { by: 1, px: 5 * k, classes: postOn } };
    const boxed = await showAndLook(12, byClass, { boxes: [whole([6000000, 2000000, 0], [6000004, 2000010, 500])] }, null);
    const sliced = await showAndLook(13, byClass, { slice: whole([-1e12, -1e12, 103], [1e12, 1e12, 104.5]) }, null);
    out.clipped = sliced.stats;
    if (boxed.picture && sliced.picture) {
      out.clipPixels = {
        inside: boxed.picture.at(cx - 100, cy + 30),
        outside: boxed.picture.at(cx + 100, cy + 30),
        postBoxed: boxed.picture.at(cx, cy),
        postSliced: sliced.picture.at(cx, cy),
        floorSliced: sliced.picture.at(cx - 100, cy + 30),
      };
      const c = out.clipPixels;
      out.clipDrawn = near(c.inside, CLASS_FLOOR, 12) && near(c.outside, SCAN_CLEAR, 12) && near(c.postBoxed, SCAN_CLEAR, 12) && near(c.postSliced, [175, 175, 175], 12) && near(c.floorSliced, SCAN_CLEAR, 12);
    }
    // Shown faint (1.37; the page's Show switches): the floor at 0.3 of its strength, the page's background through the rest.
    const faint = await showAndLook(14, byClass, { strength: 0.3 }, null);
    // Fill gaps (1.37): points a pixel each leave the page's background between them (they're 3.3 px apart here); filled, the floor closes up.
    const small = { by: 1, px: 1, classes: postOn };
    const open = await showAndLook(15, { look: small }, {}, null);
    const filled = await showAndLook(16, { look: { ...small, fill: 12 * k } }, {}, null);
    out.filled = filled.stats;
    if (faint.picture && open.picture && filled.picture) {
      const block = (picture) => {
        let floor = 0;
        for (let dy = 0; dy < 4; dy += 1) for (let dx = 0; dx < 4; dx += 1) if (near(picture.at(cx - 100 + dx, cy + 30 + dy), CLASS_FLOOR, 12)) floor += 1;
        return floor;
      };
      const want = CLASS_FLOOR.map((v, i) => Math.round(SCAN_CLEAR[i] + (v - SCAN_CLEAR[i]) * 0.3));
      out.fillPixels = { faint: faint.picture.at(cx - 100, cy + 30), want, open: block(open.picture), filled: block(filled.picture) };
      out.fillDrawn = near(out.fillPixels.faint, want, 12) && out.fillPixels.open < 16 && out.fillPixels.open > 0 && out.fillPixels.filled === 16;
    }
    // Two views in one frame (1.37; the split view): the drawing area's in the window's left half by class, the other
    // side's in its right half by height, each about the post; the page's background between them. Then the other side gone.
    const wide = inside.width * k;
    const half = Math.floor(wide / 2);
    const leftRect = [0, 0, half - 4, inside.height * k];
    const rightRect = [half + 4, 0, wide - half - 4, inside.height * k];
    await engine.ask({ kind: "view", scans: [{ token: engine.token, affine, look: { by: 2, px: 5 * k, heights: [100, 104] } }], ask: { ...view, rect: rightRect, slot: 1, serial: 18 } });
    let two = await showAndLook(19, byClass, { rect: leftRect }, null);
    for (let i = 0; i < 60 && !(two.stats && Array.isArray(two.stats.views) && two.stats.views[5] === 18 && two.stats.views[3] >= 1); i += 1) {
      await wait(250);
      two = { ...two, stats: (await engine.ask({ kind: "view-stats" })).stats || null };
    }
    await wait(400);
    two.picture = await look(png ? png.replace(/\.png$/, ".split.png") : null);
    out.split = two.stats;
    if (two.picture) {
      const lx = Math.round((half - 4) / 2 / k);
      const rx = Math.round((half + 4 + (wide - half - 4) / 2) / k);
      out.splitPixels = {
        leftPost: two.picture.at(lx, cy),
        // (East of the post: the check page's own green bar is over the window's left edge.)
        leftFloor: two.picture.at(lx + 60, cy + 30),
        between: two.picture.at(Math.round(half / k), cy + 30),
        rightPost: two.picture.at(rx, cy),
        rightFloor: two.picture.at(rx - 60, cy + 30),
      };
      const c = out.splitPixels;
      out.splitDrawn = near(c.leftPost, [175, 175, 175], 12) && near(c.leftFloor, CLASS_FLOOR, 12) && near(c.between, SCAN_CLEAR, 12) && near(c.rightPost, RAMP_HIGH, 30) && near(c.rightFloor, RAMP_LOW, 30);
    }
    await engine.ask({ kind: "view", scans: [], ask: { slot: 1 } });
    // 1.39. The brush's preview: hovering over the floor (20 px from its middle out, in red) tints the points under it;
    // painting with Hide takes them out at once. The floor away from it as it was.
    const tinted = CLASS_FLOOR.map((v, i) => Math.round(v * 0.55 + [255, 0, 0][i] * 0.45));
    const brushAt = { x: (cx - 100) * k, y: (cy + 30) * k, r: 20 * k, color: [1, 0, 0] };
    const hovering = await showAndLook(21, byClass, { brush: brushAt }, png ? png.replace(/\.png$/, ".brush.png") : null);
    const hiding = await showAndLook(22, byClass, { brush: { ...brushAt, painting: true, hide: true } }, null);
    if (hovering.picture && hiding.picture) {
      out.brushPixels = {
        under: hovering.picture.at(cx - 100, cy + 30),
        want: tinted,
        away: hovering.picture.at(cx - 40, cy + 30),
        hidden: hiding.picture.at(cx - 100, cy + 30),
        kept: hiding.picture.at(cx - 40, cy + 30),
      };
      const c = out.brushPixels;
      out.brushDrawn = near(c.under, tinted, 12) && near(c.away, CLASS_FLOOR, 12) && near(c.hidden, SCAN_CLEAR, 12) && near(c.kept, CLASS_FLOOR, 12);
    }
    // Its strokes on the card: a press's dabs over that same floor (0.6 units from each one's middle out: 20 px), across the
    // plan view. Giving the post's class: that floor in the post's colour. Hiding: gone. Keeping only: the rest gone.
    const way = { right: [1, 0, 0], up: [0, 1, 0] };
    const dabs = new Float64Array([6000005 - 100 * 0.03, 2000005 - 30 * 0.03, 0.6]);
    const strokeShot = async (serial, key, stroke) => {
      const took = await engine.ask({ kind: "view-strokes", key, strokes: [{ ...way, dabs, ...stroke }] });
      if (took.error || !took.ok) throw new Error(took.error || "the strokes weren't taken");
      return showAndLook(serial, byClass, { strokes: key }, null);
    };
    const classed = await strokeShot(23, "check class", { op: "class", cls: 5 });
    const struck = await strokeShot(24, "check out", { op: "out" });
    const keptOnly = await strokeShot(25, "check keep", { op: "keep" });
    out.stroked = keptOnly.stats;
    if (classed.picture && struck.picture && keptOnly.picture) {
      const here = (p) => p.at(cx - 100, cy + 30);
      const there = (p) => p.at(cx - 40, cy + 30);
      out.strokePixels = { classed: here(classed.picture), classedAway: there(classed.picture), out: here(struck.picture), outAway: there(struck.picture), keep: here(keptOnly.picture), keepAway: there(keptOnly.picture) };
      const c = out.strokePixels;
      out.strokeDrawn =
        near(c.classed, [175, 175, 175], 12) && near(c.classedAway, CLASS_FLOOR, 12) && near(c.out, SCAN_CLEAR, 12) && near(c.outAway, CLASS_FLOOR, 12) && near(c.keep, CLASS_FLOOR, 12) && near(c.keepAway, SCAN_CLEAR, 12);
    }
    // The page's own colouring for the scan (one the engine hasn't): the page's points of it (one every unit over the
    // floor, each magenta) given with their places; the scan in its own colours is then theirs. Filed on a thread of
    // the engine's own: asked for till it's in.
    const mine = [];
    for (let i = 0; i <= 10; i += 1) for (let j = 0; j <= 10; j += 1) mine.push(i, j, 0);
    const took = await engine.ask({
      kind: "view-colours",
      token: engine.token,
      key: "check colours",
      pointsKey: "check points",
      cell: 2,
      origin: [6000000, 2000000, 100],
      xyz: Float32Array.from(mine),
      rgba: Uint8Array.from(mine.filter((_, n) => n % 3 === 0).flatMap(() => [255, 0, 255, 255])),
    });
    out.coloursTaken = took.ok === true;
    if (took.ok) {
      const inColours = { tint: "check colours", look: { by: 0, px: 5 * k } };
      for (let i = 0; i < 40; i += 1) {
        const said = await engine.ask({ kind: "view", scans: [{ token: engine.token, affine, ...inColours }], ask: { ...view, serial: 26 } });
        if (said.error || said.shown >= 1) break;
        await wait(100);
      }
      const coloured = await showAndLook(27, inColours, {}, null);
      out.coloured = coloured.stats;
      if (coloured.picture) {
        out.colourPixels = { floor: coloured.picture.at(cx - 100, cy + 30), off: coloured.picture.at(cx + 230, cy) };
        out.coloursDrawn = near(out.colourPixels.floor, [255, 0, 255], 12) && near(out.colourPixels.off, SCAN_CLEAR, 12);
      }
    }
    // A scan asked for another way with its older reading kept meanwhile: never left out. Read again (here with its
    // shading worked out), it's drawn all the while, from the older reading and then the new.
    const again = await engine.ask({ kind: "view", scans: [{ token: engine.token, affine, look: { by: 1, px: 5 * k, classes: postOn, shade: [100, 100, 100] } }], ask: { ...view, serial: 60, stale: true } });
    let kept = again.stats || null;
    let seenOlder = 0;
    let leftOut = 0;
    for (let i = 0; i < 80; i += 1) {
      if (kept && kept.drawn === 60) {
        const older = Array.isArray(kept.stale) ? kept.stale[0] || 0 : 0;
        if (older) seenOlder += 1;
        if (kept.scans + older < 1) leftOut += 1;
        if (kept.scans >= 1 && !older) break;
      }
      await wait(50);
      kept = (await engine.ask({ kind: "view-stats" })).stats || null;
    }
    out.keptOlder = { seenOlder, leftOut, stats: kept };
    out.olderKept = Boolean(kept && kept.drawn === 60 && kept.scans >= 1 && leftOut === 0 && Array.isArray(kept.stale));
    // 1.40: the reading the scan had before that one (in the page's colouring) is kept on the card beside it. Asked
    // for again (an undo), it's the first frame of the ask that draws it, itself, with nothing read.
    /** The stats of the first frame that showed an ask. */
    const firstOf = async (serial, said) => {
      let stats = said.stats || null;
      for (let i = 0; i < 200 && !(stats && stats.drawn === serial); i += 1) {
        await wait(20);
        stats = (await engine.ask({ kind: "view-stats" })).stats || null;
      }
      return stats;
    };
    const atOnce = (stats, serial) => Boolean(stats && stats.drawn === serial && stats.scans >= 1 && !(Array.isArray(stats.stale) && stats.stale[0]));
    const back = await firstOf(61, await engine.ask({ kind: "view", scans: [{ token: engine.token, affine, tint: "check colours", look: { by: 0, px: 5 * k } }], ask: { ...view, serial: 61, staleBase: true } }));
    out.wentBack = back;
    out.backAtOnce = atOnce(back, 61);
    // With points isolated (the floor's west 3 units), a base of the scan with none isolated is read too and kept
    // ready: every point shown again, the first frame of the ask draws it (the east floor's there).
    const west = await engine.ask({ kind: "view-shapes", key: "check west", shapes: [{ right: [1, 0, 0], up: [0, 1, 0], poly: [5999990, 1999990, 6000003, 1999990, 6000003, 2000020, 5999990, 2000020] }] });
    out.shapesTaken = west.ok === true;
    if (west.ok) {
      const isolated = await showAndLook(62, byClass, { isolate: "check west" }, null);
      out.isolated = isolated.stats;
      // The spare's read once there's nothing else to read.
      await wait(1500);
      const all = await firstOf(63, await engine.ask({ kind: "view", scans: [{ token: engine.token, affine, ...byClass }], ask: { ...view, serial: 63, staleBase: true } }));
      out.shownAll = all;
      await wait(600);
      const shownAll = await look(null);
      if (isolated.picture && shownAll) {
        out.sparePixels = { west: isolated.picture.at(cx - 100, cy + 30), east: isolated.picture.at(cx + 100, cy + 30), westAll: shownAll.at(cx - 100, cy + 30), eastAll: shownAll.at(cx + 100, cy + 30) };
        const c = out.sparePixels;
        out.spareAtOnce = atOnce(all, 63) && near(c.west, CLASS_FLOOR, 12) && near(c.east, SCAN_CLEAR, 12) && near(c.westAll, CLASS_FLOOR, 12) && near(c.eastAll, CLASS_FLOOR, 12);
      }
    }
    // 1.41 (stage 2): the page's linework drawn by the engine over the scan, in the same frame. One line across the
    // floor 60 px below its middle, four of the screen's pixels wide, magenta: a batch's strips (the page's own
    // packing, nine floats a segment), a list that names it, and a view that names the list and says where the
    // page puts it (x = e + r·scale, y = f − u·scale, the page's pixels from the view's corner).
    const gave = await engine.ask({ kind: "view-line-strips", batch: 1, strips: Float32Array.from([cx - 120, -(cy + 60), 0, cx + 120, -(cy + 60), 0, 1, 1, 0]) });
    const listed = gave.ok ? await engine.ask({ kind: "view-lines", key: "check lines", items: [{ id: 1, rgba: [1, 0, 1, 1] }] }) : gave;
    out.linesTaken = listed.ok === true;
    if (listed.ok) {
      const lined = await showAndLook(64, byClass, { lines: { key: "check lines", place: [1, 0, 0], half: 2 * k, ratio: k } }, png ? png.replace(/\.png$/, ".lines.png") : null);
      out.lined = lined.stats;
      if (lined.picture) {
        out.linePixels = { on: lined.picture.at(cx - 50, cy + 60), beside: lined.picture.at(cx - 50, cy + 80), past: lined.picture.at(cx + 140, cy + 60) };
        const c = out.linePixels;
        out.linesDrawn = near(c.on, [255, 0, 255], 12) && near(c.beside, CLASS_FLOOR, 12) && near(c.past, CLASS_FLOOR, 12) && Array.isArray(lined.stats && lined.stats.lines) && lined.stats.lines[0] === "check lines";
      }
      // 1.42: the same line dashed (30 px on, 30 off, running on along it; its ends' third floats say how far along
      // each is): a dash where it was, the floor in a gap.
      const gaveDashed = await engine.ask({ kind: "view-line-strips", batch: 2, strips: Float32Array.from([cx - 120, -(cy + 60), 0, cx + 120, -(cy + 60), 240, 1, 1, 0]) });
      const listedDashed = gaveDashed.ok ? await engine.ask({ kind: "view-lines", key: "check dashes", items: [{ id: 2, rgba: [1, 0, 1, 1], dash: [30, 30] }] }) : gaveDashed;
      if (listedDashed.ok) {
        const dashedShot = await showAndLook(66, byClass, { lines: { key: "check dashes", place: [1, 0, 0], half: 2 * k, ratio: k, dot: 2 * k } }, null);
        if (dashedShot.picture) {
          // 70 px along: ten into the second dash. 40 px along: ten into the first gap.
          out.dashPixels = { dash: dashedShot.picture.at(cx - 50, cy + 60), gap: dashedShot.picture.at(cx - 80, cy + 60) };
          out.dashesDrawn = near(out.dashPixels.dash, [255, 0, 255], 12) && near(out.dashPixels.gap, CLASS_FLOOR, 12);
        }
      } else out.dashesError = listedDashed.error || "not taken";
      // 1.44: a filled shape of the page's under that line: 80 px across and 60 down around it, with a hole 20 px wide
      // above the line, green, filled even-odd through the card's stencil (its two loops as triangle fans, nine
      // floats a triangle): green in it, the floor through its hole, the line over it.
      const fanOf = (c) => {
        const o = [];
        for (let i = 1; i + 1 < c.length; i += 1) for (const q of [c[0], c[i], c[i + 1]]) o.push(q[0], -q[1], 0);
        return o;
      };
      const boxOf = (x0, y0, x1, y1) => [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
      // (1.45: its left side half way across a column of pixels: that column's half its colour, the fills' edges being smooth.)
      const gaveArea = await engine.ask({ kind: "view-line-strips", batch: 3, strips: Float32Array.from([...fanOf(boxOf(cx - 90.5, cy + 25, cx - 10, cy + 85)), ...fanOf(boxOf(cx - 60, cy + 30, cx - 40, cy + 50))]) });
      const listedArea = gaveArea.ok ? await engine.ask({ kind: "view-lines", key: "check areas", items: [{ id: 1, rgba: [1, 0, 1, 1] }, { id: 3, rgba: [0, 1, 0, 1], area: true }] }) : gaveArea;
      if (listedArea.ok) {
        const areaShot = await showAndLook(67, byClass, { lines: { key: "check areas", place: [1, 0, 0], half: 2 * k, ratio: k } }, null);
        if (areaShot.picture) {
          out.areaPixels = { fill: areaShot.picture.at(cx - 80, cy + 75), hole: areaShot.picture.at(cx - 50, cy + 40), line: areaShot.picture.at(cx - 50, cy + 60), past: areaShot.picture.at(cx + 30, cy + 40), edge: areaShot.picture.at(cx - 91, cy + 75), beside: areaShot.picture.at(cx - 92, cy + 75) };
          const c = out.areaPixels;
          out.areasDrawn = near(c.fill, [0, 255, 0], 12) && near(c.hole, CLASS_FLOOR, 12) && near(c.line, [255, 0, 255], 12) && near(c.past, CLASS_FLOOR, 12);
          // Half the fill's green over half the floor, at a screen that draws a pixel a pixel.
          const half = [0, 1, 2].map((i) => Math.round(([0, 255, 0][i] + CLASS_FLOOR[i]) / 2));
          out.smoothDrawn = k !== 1 || (near(c.edge, half, 24) && near(c.beside, CLASS_FLOOR, 12));
        }
      } else out.areasError = listedArea.error || "not taken";
      // A view that names no lines: none drawn (the page's own again).
      const unlined = await showAndLook(65, byClass, {}, null);
      if (unlined.picture) {
        out.unlinedPixel = unlined.picture.at(cx - 50, cy + 60);
        out.linesDrawn = out.linesDrawn === true && near(out.unlinedPixel, CLASS_FLOOR, 12) && !(unlined.stats && unlined.stats.lines && unlined.stats.lines[0]);
      }
    }
    // Kept on the card and not drawn (the page drawing its scans itself): the background where the post was.
    const hidden = await showAndLook(70, { look: { by: 2, px: 5 * k, heights: [100, 104] } }, { hide: true }, null);
    out.hidden = hidden.stats;
    if (hidden.picture) {
      out.hiddenPixels = { post: hidden.picture.at(cx, cy), floor: hidden.picture.at(cx - 100, cy + 30) };
      out.hiddenClear = near(out.hiddenPixels.post, SCAN_CLEAR, 12) && near(out.hiddenPixels.floor, SCAN_CLEAR, 12);
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
  SCAN_CLEAR,
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
