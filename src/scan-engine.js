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
// asks Electron where that file is, and only a .las or .laz file that's
// there is opened, read only, given a token. The page then talks to the
// engine's process over a port of its own, by tokens.

const path = require("node:path");
const crypto = require("node:crypto");

/** The scan files the engine reads (E57 still goes through the page). */
const SCAN_EXTENSIONS = [".las", ".laz"];
/** At most this many scans opened in a session. */
const MOST_SCANS = 500;
/** After a crash, the engine's process is started again at most this many times. */
const MOST_RESTARTS = 3;

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
function createScanEngine({ fork, newChannel, statFile, exists, cacheDir, addon, hostPath, log = () => {}, later = setTimeout }) {
  let host = null;
  let restarts = 0;
  let stopped = false;
  /** token → path, and path (lower case) → token. */
  const tokens = new Map();
  const byPath = new Map();
  const available = typeof exists === "function" ? exists(addon) : true;

  function start() {
    if (stopped || !available) return null;
    const child = fork(hostPath, [`--cadp-engine-addon=${addon}`], {
      serviceName: "CAD Protocol scan engine",
      stdio: "pipe",
    });
    host = child;
    const lines = (stream) => {
      if (!stream || typeof stream.on !== "function") return;
      stream.on("data", (chunk) => {
        for (const line of String(chunk).split(/\r?\n/)) if (line.trim()) log(line.trim());
      });
    };
    lines(child.stdout);
    lines(child.stderr);
    child.on("exit", (code) => {
      if (host === child) host = null;
      if (stopped) return;
      log(`scan engine: its process ended (${code})`);
      if (restarts >= MOST_RESTARTS) return;
      restarts += 1;
      later(() => {
        const again = ensure();
        // Its scans opened again (their indexes open at once).
        if (again) for (const [token, p] of tokens) again.postMessage({ kind: "open", token, path: p, cacheDir });
      }, 500 * restarts);
    });
    return child;
  }

  function ensure() {
    return host || start();
  }

  /** A scan file the page was given: its token (the same each time for the same file), or null. */
  async function register(filePath) {
    if (!available || !scanPathOk(filePath)) return null;
    const stat = await statFile(filePath);
    if (!stat || typeof stat.isFile !== "function" || !stat.isFile()) return null;
    const key = filePath.toLowerCase();
    let token = byPath.get(key);
    if (!token) {
      if (tokens.size >= MOST_SCANS) return null;
      token = crypto.randomBytes(16).toString("hex");
      tokens.set(token, filePath);
      byPath.set(key, token);
      const h = ensure();
      if (!h) return null;
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
    stop,
    tokens,
    get running() {
      return host !== null;
    },
  };
}

module.exports = { SCAN_EXTENSIONS, MOST_SCANS, addonPath, cacheDirFor, createScanEngine, scanPathOk, unpacked };
