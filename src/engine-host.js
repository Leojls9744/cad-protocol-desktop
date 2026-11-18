"use strict";
// The scan engine's own process (1.19): a background process of the app's,
// started by src/scan-engine.js, holding the engine (the Rust add-on,
// engine/), so its reading and searching never wait on the window, and the
// window never waits on them.
//
// From the app's main process it hears which scan files the page was given
// (each a token and its path: `open`), and gets a port for each page that
// asks (`page`). Over a page's port it says what's open (`hello`, `opened`,
// `progress`, `failed`) and answers what's asked (`snap`, `status`; 1.20
// `region`; 1.22 `align` and `check`, cloud to cloud on points the page
// gives). A page names scans only by their tokens: it never gives a path.

/** How often a scan being indexed says how far it's come. */
const PROGRESS_MS = 300;

/** The host, given the engine and the port to the main process. */
function createHost({ engine, parentPort, log = () => {}, every = setInterval, stop = clearInterval }) {
  /** token → { path, handle?, info?, error?, fraction } */
  const scans = new Map();
  const ports = new Set();

  const tell = (msg) => {
    for (const port of ports) {
      try {
        port.postMessage(msg);
      } catch {
        ports.delete(port);
      }
    }
  };

  const summary = (token, s) => ({
    token,
    state: s.handle != null ? "open" : s.error ? "failed" : "indexing",
    fraction: s.fraction,
    ...(s.info ? { info: s.info } : {}),
    ...(s.error ? { error: s.error } : {}),
  });

  function open({ token, path, cacheDir }) {
    if (typeof token !== "string" || typeof path !== "string" || typeof cacheDir !== "string") return;
    if (scans.has(token)) return;
    const s = { path, fraction: 0 };
    scans.set(token, s);
    const timer = every(() => {
      const f = engine.openProgress(path);
      if (f >= 0 && f !== s.fraction) {
        s.fraction = f;
        tell({ kind: "progress", token, fraction: f });
      }
    }, PROGRESS_MS);
    let opening;
    try {
      opening = Promise.resolve(engine.openScan(path, cacheDir));
    } catch (err) {
      opening = Promise.reject(err);
    }
    opening.then(
      (info) => {
        stop(timer);
        s.handle = info.handle;
        s.fraction = 1;
        s.info = { count: info.count, built: info.built, ms: info.ms, hasRgb: info.hasRgb, min: info.min, max: info.max };
        log(`scan engine: ${info.built ? "indexed" : "opened"} ${info.count} points in ${Math.round(info.ms)} ms`);
        tell({ kind: "opened", token, info: s.info });
      },
      (err) => {
        stop(timer);
        s.error = String((err && err.message) || err);
        log(`scan engine: couldn't open a scan: ${s.error}`);
        tell({ kind: "failed", token, error: s.error });
      },
    );
  }

  function close({ token }) {
    const s = scans.get(token);
    if (!s) return;
    scans.delete(token);
    if (s.handle != null) engine.closeScan(s.handle);
  }

  function answer(port, msg) {
    if (!msg || typeof msg !== "object") return;
    const id = msg.id;
    if (msg.kind === "status") {
      port.postMessage({ kind: "status", id, scans: [...scans].map(([t, s]) => summary(t, s)) });
      return;
    }
    // 1.20: a scan's points in a box of the drawing (a view's close-up, a command's region).
    if (msg.kind === "region") {
      const held = scans.get(msg.token);
      if (!held || held.handle == null) {
        port.postMessage({ kind: "region", id, missing: [msg.token] });
        return;
      }
      try {
        const got = engine.region(held.handle, msg.affine, msg.min, msg.max, msg.ring ?? undefined, msg.most);
        port.postMessage({
          kind: "region",
          id,
          origin: got.origin,
          count: got.count,
          xyz: got.xyz,
          intensity: got.intensity,
          classification: got.classification,
          flags: got.flags,
          returns: got.returns,
          rgb: got.rgb ?? null,
          complete: got.complete,
          tried: got.tried,
          ms: got.ms,
        });
      } catch (err) {
        port.postMessage({ kind: "region", id, error: String((err && err.message) || err) });
      }
      return;
    }
    // 1.22: cloud to cloud (ALIGNSCANS, SCANCHECK), the page's points given;
    // worked on the engine's own threads, so snaps are answered meanwhile.
    if (msg.kind === "align" || msg.kind === "check") {
      const kind = msg.kind;
      let work;
      try {
        work =
          kind === "align"
            ? engine.alignClouds(msg.fixed, msg.moving, msg.ask)
            : engine.checkClouds(Array.isArray(msg.scans) ? msg.scans : [], msg.foot, msg.tolerance);
      } catch (err) {
        work = Promise.reject(err);
      }
      Promise.resolve(work).then(
        (got) => {
          try {
            port.postMessage({ kind, id, ...got });
          } catch {
            ports.delete(port);
          }
        },
        (err) => {
          try {
            port.postMessage({ kind, id, error: String((err && err.message) || err) });
          } catch {
            ports.delete(port);
          }
        },
      );
      return;
    }
    if (msg.kind !== "snap") return;
    const list = Array.isArray(msg.scans) ? msg.scans : [];
    const asked = [];
    const missing = [];
    for (const s of list) {
      const held = s && scans.get(s.token);
      if (!held || held.handle == null) missing.push(s && s.token);
      else asked.push({ handle: held.handle, affine: s.affine, allow: s.allow ?? undefined });
    }
    if (missing.length || !asked.length) {
      port.postMessage({ kind: "snap", id, missing });
      return;
    }
    try {
      const got = engine.snap(asked, msg.ask);
      port.postMessage({ kind: "snap", id, front: got.front, near: got.near, tried: got.tried, cells: got.cells, ms: got.ms });
    } catch (err) {
      port.postMessage({ kind: "snap", id, error: String((err && err.message) || err) });
    }
  }

  function addPage(port) {
    ports.add(port);
    port.on("message", (event) => answer(port, event.data));
    port.on("close", () => ports.delete(port));
    port.start();
    port.postMessage({
      kind: "hello",
      version: engine.engineVersion(),
      scans: [...scans].map(([t, s]) => summary(t, s)),
    });
  }

  parentPort.on("message", (event) => {
    const msg = event.data;
    if (!msg || typeof msg !== "object") return;
    if (msg.kind === "open") open(msg);
    else if (msg.kind === "close") close(msg);
    else if (msg.kind === "page" && event.ports && event.ports[0]) addPage(event.ports[0]);
  });

  return { scans, ports, answer };
}

module.exports = { createHost, PROGRESS_MS };

// Started as the engine's process: the add-on named on its command line.
if (process.parentPort) {
  const arg = process.argv.find((a) => a.startsWith("--cadp-engine-addon="));
  const addon = arg ? arg.slice("--cadp-engine-addon=".length) : "";
  const log = (line) => process.stderr.write(`${line}\n`);
  try {
    // eslint-disable-next-line import/no-dynamic-require
    const engine = require(addon);
    createHost({ engine, parentPort: process.parentPort, log });
    log(`scan engine ${engine.engineVersion()} ready`);
  } catch (err) {
    log(`scan engine: couldn't start: ${err && err.message ? err.message : err}`);
    process.exit(1);
  }
}
