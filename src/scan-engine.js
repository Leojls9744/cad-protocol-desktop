"use strict";
// The scan engine (1.19; Leo, 2026-11-17 13:16: "lets make cad protocol a
// desktop app and pull away from the browser, can we build the engine so
// that snap search run on the computer?").
//
// The engine (engine/, Rust) runs in a background process of the app's own
// (src/engine-host.js). It reads the person's LAS and LAZ scans straight
// from disk, files every point by where it is in an index kept in
// %LOCALAPPDATA%\CAD Protocol\scan-engine (a scan opened before opens again
// at once), and finds the scan's points under the cursor for the page's
// snaps.
//
// The page never names a path. It hands over a scan file the person chose
// (an Open window, a dropped file, a linked folder's file): the preload
// asks Electron where that file is, and only a .las, .laz or .e57 file that's
// there is opened, read only, given a token. The page then talks to the
// engine's process over a port of its own, by tokens.

const path = require("node:path");
const crypto = require("node:crypto");

/** The scan files the engine reads (1.24: E57 too). */
const SCAN_EXTENSIONS = [".las", ".laz", ".e57"];
/** At most this many scans opened in a session. */
const MOST_SCANS = 500;
/** After a crash, the engine's process is started again at most this many times in a row. */
const MOST_RESTARTS = 3;
/** A process that ran this long before it stopped starts the count again (1.25). */
const STEADY_MS = 10 * 60 * 1000;

/** Whether a path the preload gave is a scan file the engine may open. */
function scanPathOk(p) {
  if (typeof p !== "string" || !p || p.length > 4096 || p.includes("\u0000")) return false;
  if (!path.isAbsolute(p) && !path.win32.isAbsolute(p)) return false;
  return SCAN_EXTENSIONS.includes(path.extname(p).toLowerCase());
}

/** A path inside the app's asar as Electron unpacks it (native add-ons can't load from inside it). */
function unpacked(p) {
  return p.replace(/app\.asar([\\/])/, "app.asar.unpacked$1");
}

/** Where the add-on is, from src/. */
function addonPath(dir) {
  return unpacked(path.join(dir, "..", "engine", "dist", "cadp-engine.node"));
}

/** Where the indexes are kept: this computer's own app data (not the roaming one). */
function cacheDirFor(env, userData) {
  return env.LOCALAPPDATA ? path.join(env.LOCALAPPDATA, "CAD Protocol", "scan-engine") : path.join(userData, "scan-engine");
}

/**
 * The engine, started when first needed.
 * fork(hostPath, args, options) starts its process (Electron's utilityProcess.fork);
 * newChannel() a MessageChannelMain; statFile(path) resolves fs.Stats or null.
 */
function createScanEngine({ fork, newChannel, statFile, exists, cacheDir, addon, hostPath, log = () => {}, later = setTimeout, now = Date.now }) {
  let host = null;
  let restarts = 0;
  let stopped = false;
  /** Stopped after crashing MOST_RESTARTS times (1.25): scans go through the page. */
  let gaveUp = false;
  /** token → path, and path (lower case) → token. */
  const tokens = new Map();
  const byPath = new Map();
  /** Tokens whose scans the engine has opened or turned down (1.25). */
  const settled = new Set();
  /** The pages given a port, told when the engine stops and starts again (1.25). */
  const pages = new Set();
  const available = typeof exists === "function" ? exists(addon) : true;
  /** The frame the engine's own window is in (1.35, the layer trial), as a whole number's text; null: none. */
  let viewFrame = null;
  /** Cuts asked of the engine (1.28), by id: told how far they've come, then done. */
  const cuts = new Map();
  let nextCut = 1;

  /** A word to each page still there. */
  function tellPages(channel, msg) {
    for (const contents of pages) {
      try {
        if (typeof contents.isDestroyed === "function" && contents.isDestroyed()) pages.delete(contents);
        else contents.postMessage(channel, msg);
      } catch {
        pages.delete(contents);
      }
    }
  }

  function start() {
    if (stopped || gaveUp || !available) return null;
    const child = fork(hostPath, [`--cadp-engine-addon=${addon}`], {
      serviceName: "CAD Protocol scan engine",
      stdio: "pipe",
    });
    host = child;
    const startedAt = now();
    const lines = (stream) => {
      if (!stream || typeof stream.on !== "function") return;
      stream.on("data", (chunk) => {
        for (const line of String(chunk).split(/\r?\n/)) if (line.trim()) log(line.trim());
      });
    };
    lines(child.stdout);
    lines(child.stderr);
    child.on("message", (msg) => {
      if (msg && (msg.kind === "opened" || msg.kind === "failed") && typeof msg.token === "string") settled.add(msg.token);
      // 1.35: whether its own window started in the frame.
      if (msg && msg.kind === "view-started" && msg.error) log(`scan engine: no window of its own: ${msg.error}`);
      // 1.28: a cut's progress, and its end.
      const cut = msg && cuts.get(msg.id);
      if (cut && msg.kind === "crop-progress") cut.progress(Number(msg.done) || 0);
      else if (cut && msg.kind === "crop-done") {
        cuts.delete(msg.id);
        cut.done(msg);
      }
    });
    child.on("exit", (code) => {
      if (host === child) host = null;
      // Cuts out on it end with it (1.28).
      for (const [id, cut] of cuts) {
        cuts.delete(id);
        cut.done({ kind: "crop-done", id, error: "The scan engine stopped while it was cutting the scan." });
      }
      if (stopped) return;
      log(`scan engine: its process ended (${code})`);
      if (now() - startedAt >= STEADY_MS) restarts = 0;
      // 1.25: the scans it was still reading let go, so one can't stop it again and again.
      const dropped = [...tokens.keys()].filter((t) => !settled.has(t));
      for (const t of dropped) {
        byPath.delete(tokens.get(t).toLowerCase());
        tokens.delete(t);
      }
      const again = restarts < MOST_RESTARTS;
      if (!again) gaveUp = true;
      tellPages("cadp:engine:lost", { again, dropped });
      if (!again) return;
      restarts += 1;
      later(() => {
        if (ensure()) tellPages("cadp:engine:back", {});
      }, 500 * restarts);
    });
    // Its scans opened again (their indexes open at once).
    for (const [token, p] of tokens) child.postMessage({ kind: "open", token, path: p, cacheDir });
    // 1.35: its own window again, in the frame it was in (the layer trial).
    if (viewFrame !== null) child.postMessage({ kind: "view-start", hwnd: viewFrame });
    return child;
  }

  function ensure() {
    return host || start();
  }

  /** A scan file the page was given: its token (the same each time for the same file), or null. */
  async function register(filePath) {
    if (!available || gaveUp || !scanPathOk(filePath)) return null;
    const stat = await statFile(filePath);
    if (!stat || typeof stat.isFile !== "function" || !stat.isFile()) return null;
    const key = filePath.toLowerCase();
    let token = byPath.get(key);
    if (!token) {
      if (tokens.size >= MOST_SCANS) return null;
      const h = ensure();
      if (!h) return null;
      token = crypto.randomBytes(16).toString("hex");
      tokens.set(token, filePath);
      byPath.set(key, token);
      h.postMessage({ kind: "open", token, path: filePath, cacheDir });
    }
    const name = filePath.includes("\\") ? path.win32.basename(filePath) : path.basename(filePath);
    return { token, name, size: stat.size };
  }

  /** A port to the engine for a page (its own end sent to the page's preload). */
  function connect(contents) {
    if (!available) return false;
    const h = ensure();
    if (!h) return false;
    const { port1, port2 } = newChannel();
    h.postMessage({ kind: "page" }, [port1]);
    contents.postMessage("cadp:engine:port", null, [port2]);
    if (!pages.has(contents)) {
      pages.add(contents);
      if (typeof contents.once === "function") contents.once("destroyed", () => pages.delete(contents));
    }
    return true;
  }

  /**
   * 1.28: a scan (by its token) cut to the drawing's limit boxes by the
   * engine, written to `out`; `progress` told how far, 0 to 1. What was cut
   * ({ kept, processed, min, max, bytes }) or { error }.
   */
  function crop(token, out, ask, progress = () => {}) {
    const h = host;
    if (!h || !tokens.has(token)) return Promise.resolve({ error: "The scan isn't open in the scan engine." });
    const id = nextCut;
    nextCut += 1;
    return new Promise((resolve) => {
      cuts.set(id, { progress, done: resolve });
      h.postMessage({ kind: "crop", id, token, out, ask });
    });
  }

  /** A token's scan file (1.28: a cut is saved beside it). */
  function pathOf(token) {
    return tokens.get(token) ?? null;
  }

  /**
   * 1.35 (the layer trial): the engine draws in a window of its own inside
   * the frame whose window handle is `hwnd` (a whole number, as text or a
   * BigInt), started with the engine and again whenever it restarts; null
   * takes it away. Whether the engine's there to be told.
   */
  function viewIn(hwnd) {
    viewFrame = hwnd === null || hwnd === undefined ? null : String(hwnd);
    if (viewFrame === null) {
      if (host) host.postMessage({ kind: "view-stop" });
      return true;
    }
    // A new process is told as it starts (start()); one that's running, now.
    const running = host;
    const h = ensure();
    if (!h) return false;
    if (running) h.postMessage({ kind: "view-start", hwnd: viewFrame });
    return true;
  }

  function stop() {
    stopped = true;
    if (host) {
      try {
        host.kill();
      } catch {
        // Gone already.
      }
      host = null;
    }
  }

  return {
    available,
    register,
    connect,
    crop,
    pathOf,
    viewIn,
    stop,
    tokens,
    get running() {
      return host !== null;
    },
    /** The engine's process (the build's check stops it, to see it start again). */
    get pid() {
      return host && typeof host.pid === "number" ? host.pid : null;
    },
  };
}

module.exports = { SCAN_EXTENSIONS, MOST_SCANS, STEADY_MS, addonPath, cacheDirFor, createScanEngine, scanPathOk, unpacked };
