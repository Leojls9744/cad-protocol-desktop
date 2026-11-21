"use strict";
// The scan engine's own process (1.19): a background process of the app's,
// started by src/scan-engine.js, holding the engine (the Rust add-on,
// engine/), so its reading and searching never wait on the window, and the
// window never waits on them.
//
// From the app's main process it hears which scan files the page was given
// (each a token and its path: `open`), and gets a port for each page that
// asks (`page`); it tells the main process each scan opened or turned down
// (1.25), so after a crash the scans it was still reading aren't tried again. Over a page's port it says what's open (`hello`, `opened`,
// `progress`, `failed`) and answers what's asked (`snap`, `status`; 1.20
// `region`; 1.22 `align` and `check`, cloud to cloud on points the page
// gives; 1.23 `classify`, Geo W. on points the page gives; 1.28 a big
// `region` answered in parts, for a scan's overview). 1.28: the main
// process asks for a scan cut to the limit boxes (`crop`), into a file the
// person chose; a page asks for a `render` (the ray tracer on every core),
// told how far it's come, and can stop it (`render-stop`). 1.29: the rules
// classifier's middle (`classify-stage`). A page names scans only by their tokens: it never gives a path.

/** How often a scan being indexed says how far it's come. */
const PROGRESS_MS = 300;

/** The most one message carries (1.28): a bigger list goes ahead in parts, as the page sends its own (1.23). */
const PART_BYTES = 16 * 1024 * 1024;
const LISTS = ["Float64Array", "Float32Array", "Uint8Array", "Uint16Array", "Uint32Array", "Int8Array", "Int32Array"];
const listOf = (v) => (ArrayBuffer.isView(v) ? LISTS.find((n) => Object.prototype.toString.call(v) === `[object ${n}]`) : undefined);

/**
 * An answer sent with its big lists in parts first (1.28): each over
 * PART_BYTES named in its place ({ __part, type, length }) and sent ahead a
 * part at a time ({ kind: "part", id, part, at, total, bytes }), put back
 * together by the page before it reads the answer. How many lists went in parts.
 */
function postInParts(port, msg) {
  let parts = 0;
  // 1.29: lists inside a list too (each scan's, say).
  const walk = (v) => {
    const type = listOf(v);
    if (type) {
      if (v.byteLength <= PART_BYTES) return v;
      const part = parts;
      parts += 1;
      const bytes = new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
      for (let at = 0; at < bytes.length; at += PART_BYTES)
        port.postMessage({ kind: "part", id: msg.id, part, at, total: bytes.length, bytes: bytes.slice(at, Math.min(bytes.length, at + PART_BYTES)) });
      return { __part: part, type, length: v.length };
    }
    if (Array.isArray(v)) return v.map(walk);
    return v;
  };
  const out = {};
  for (const [k, v] of Object.entries(msg)) out[k] = walk(v);
  port.postMessage(out);
  return parts;
}

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

  /** The main process told a scan is settled (1.25). */
  const settle = (kind, token) => {
    try {
      if (typeof parentPort.postMessage === "function") parentPort.postMessage({ kind, token });
    } catch {
      // The main process is going.
    }
  };

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
    s.ready = opening.then(
      (info) => {
        stop(timer);
        s.handle = info.handle;
        s.fraction = 1;
        s.info = { count: info.count, built: info.built, ms: info.ms, hasRgb: info.hasRgb, setups: info.setups === true, e57: info.e57 === true, min: info.min, max: info.max };
        log(`scan engine: ${info.built ? "indexed" : "opened"} ${info.count} points in ${Math.round(info.ms)} ms`);
        settle("opened", token);
        tell({ kind: "opened", token, info: s.info });
      },
      (err) => {
        stop(timer);
        s.error = String((err && err.message) || err);
        log(`scan engine: couldn't open a scan: ${s.error}`);
        settle("failed", token);
        tell({ kind: "failed", token, error: s.error });
      },
    );
  }

  /** Told to the main process (1.28: a cut's progress and its end). */
  const toMain = (msg) => {
    try {
      if (typeof parentPort.postMessage === "function") parentPort.postMessage(msg);
    } catch {
      // The main process is going.
    }
  };

  /**
   * 1.28: a scan cut to the drawing's limit boxes from its index, every
   * point, written to `out` (the main process chose it with the person, in
   * Windows' Save window); how far it's come said now and then, then what
   * was cut, or why not. A scan still being indexed is waited for.
   */
  async function crop({ id, token, out, ask }) {
    const held = scans.get(token);
    if (!held) {
      toMain({ kind: "crop-done", id, error: "The scan isn't open in the scan engine." });
      return;
    }
    if (held.handle == null && held.ready) await held.ready.catch(() => {});
    if (held.handle == null) {
      toMain({ kind: "crop-done", id, error: held.error || "The scan engine couldn't read the scan." });
      return;
    }
    let last = -1;
    const timer = every(() => {
      const done = engine.cropProgress();
      if (done !== last) {
        last = done;
        toMain({ kind: "crop-progress", id, done });
      }
    }, PROGRESS_MS);
    try {
      const got = await engine.cropScan(held.handle, ask, out);
      toMain({ kind: "crop-done", id, kept: got.kept, processed: got.processed, min: got.min, max: got.max, bytes: got.bytes, ms: got.ms });
    } catch (err) {
      toMain({ kind: "crop-done", id, error: String((err && err.message) || err) });
    } finally {
      stop(timer);
    }
  }

  /** A question's big lists as they come in parts (1.23), by port, then id. */
  const partsOf = new WeakMap();

  /** A part kept; a question's parts put back in their places. */
  function gathered(port, msg) {
    let held = partsOf.get(port);
    if (!held) {
      held = new Map();
      partsOf.set(port, held);
    }
    if (msg.kind === "part") {
      if (!(msg.bytes instanceof Uint8Array) || !(msg.total > 0)) return null;
      let mine = held.get(msg.id);
      if (!mine) {
        mine = new Map();
        held.set(msg.id, mine);
      }
      let buf = mine.get(msg.part);
      if (!buf) {
        buf = new Uint8Array(msg.total);
        mine.set(msg.part, buf);
      }
      if (msg.at >= 0 && msg.at + msg.bytes.length <= buf.length) buf.set(msg.bytes, msg.at);
      return null;
    }
    const mine = held.get(msg.id);
    if (!mine) return msg;
    held.delete(msg.id);
    const KINDS = { Float64Array, Float32Array, Uint8Array, Uint16Array, Uint32Array, Int8Array, Int32Array };
    const put = (v) => {
      if (v && typeof v === "object" && !ArrayBuffer.isView(v)) {
        if (typeof v.__part === "number" && KINDS[v.type]) {
          const buf = mine.get(v.__part);
          return buf ? new KINDS[v.type](buf.buffer, 0, v.length) : v;
        }
        if (Array.isArray(v)) return v.map(put);
        return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, put(x)]));
      }
      return v;
    };
    return put(msg);
  }

  /**
   * 1.35: the engine's own window (cadp-engine-view), in the app's frame.
   * `started`: the frame it's in, as the app gave it; an engine without it
   * (an add-on from before) says so.
   */
  let viewIn = null;
  /** Why it didn't start, the last time it was tried. */
  let viewError = null;
  const canView = () => typeof engine.viewStart === "function";

  function viewStart(msg) {
    if (!canView()) return { error: "This scan engine can't draw in a window of its own." };
    try {
      engine.viewStart(String(msg.hwnd));
      viewIn = String(msg.hwnd);
      viewError = null;
      log(`scan engine: its own window started in the app's frame`);
      return { started: true };
    } catch (err) {
      viewIn = null;
      const error = String((err && err.message) || err);
      viewError = error;
      log(`scan engine: its own window didn't start: ${error}`);
      return { error };
    }
  }

  function viewStop() {
    if (!canView()) return;
    try {
      engine.viewStop();
    } catch {
      // Gone already.
    }
    viewIn = null;
  }

  const nums = (v, n) => (Array.isArray(v) && v.length === n && v.every((x) => typeof x === "number") ? v.map(Number) : undefined);
  /** 256 whole numbers (a class table), as a list; else nothing. */
  const table = (v) => {
    if (!v || v.length !== 256) return undefined;
    const out = Array.from(v, (x) => x >>> 0);
    return out.every(Number.isFinite) ? out : undefined;
  };
  /** A scan's look as the page gave it (1.36; 1.37: its shading from the shape of its surface), only what the engine reads. */
  function lookOf(l) {
    if (!l || typeof l !== "object") return undefined;
    return {
      ...(Number.isInteger(l.by) && l.by >= 0 ? { by: l.by } : {}),
      ...(typeof l.px === "number" && l.px > 0 ? { px: l.px } : {}),
      round: l.round === true,
      ...(nums(l.one, 3) ? { one: nums(l.one, 3) } : {}),
      ...(nums(l.heights, 2) ? { heights: nums(l.heights, 2) } : {}),
      ...(nums(l.intensity, 2) ? { intensity: nums(l.intensity, 2) } : {}),
      ...(nums(l.mix, 3) ? { mix: nums(l.mix, 3) } : {}),
      ...(table(l.classes) ? { classes: table(l.classes) } : {}),
      ...(nums(l.shade, 3) ? { shade: nums(l.shade, 3) } : {}),
    };
  }

  /**
   * 1.36: the page's own points of a scan it has classified or edited
   * (where they are, their classes, which are taken out), for the engine's
   * window to draw the scan by. `needPoints`: where they are must come too
   * (it had only been told their classes).
   */
  function viewClasses(msg) {
    if (!canView() || typeof engine.viewBorrow !== "function") return { error: "This scan engine can't draw a scan with the page's own classes." };
    const held = scans.get(msg.token);
    if (!held || held.handle == null) return { missing: [msg.token] };
    const u8 = (v) => (v instanceof Uint8Array ? v : null);
    try {
      if (!u8(msg.classification) || !nums(msg.origin, 3) || !(msg.cell > 0)) return { error: "The page's classes weren't as the engine reads them." };
      const took = engine.viewBorrow(
        held.handle,
        String(msg.key),
        String(msg.pointsKey),
        Number(msg.cell),
        nums(msg.origin, 3),
        msg.xyz instanceof Float32Array ? msg.xyz : null,
        msg.classification,
        u8(msg.removed),
      );
      return took ? { ok: true } : { needPoints: true };
    } catch (err) {
      return { error: String((err && err.message) || err) };
    }
  }

  /** What the window's told to show (a "view"), or just how it's going ("view-stats"). */
  function viewAnswer(msg) {
    if (!canView()) return { error: "This scan engine can't draw in a window of its own." };
    if (viewIn === null)
      return { error: viewError ? `The engine's window didn't start: ${viewError}` : "The engine's window isn't started.", stats: engine.viewStats() };
    try {
      let shown;
      if (msg.kind === "view") {
        const list = [];
        for (const s of Array.isArray(msg.scans) ? msg.scans : []) {
          const held = s && scans.get(s.token);
          if (!held || held.handle == null || !Array.isArray(s.affine)) continue;
          // 1.35: which of its points: the classes shown (eight numbers, as a snap's), an E57's one setup, its setups turned off.
          // 1.36: the class each of the file's codes is read as, the page's own classes for it (by their key), and how it looks.
          const whole = (v) => Number.isInteger(v) && v >= 0;
          list.push({
            handle: held.handle,
            affine: s.affine.map(Number),
            ...(s.most > 0 ? { most: Number(s.most) } : {}),
            ...(Array.isArray(s.allow) && s.allow.length === 8 ? { allow: s.allow.map((v) => v >>> 0) } : {}),
            ...(whole(s.setup) ? { setup: s.setup } : {}),
            ...(Array.isArray(s.off) && s.off.every(whole) && s.off.length ? { off: s.off } : {}),
            ...(table(s.map) ? { map: table(s.map) } : {}),
            ...(typeof s.borrow === "string" && s.borrow ? { borrow: s.borrow } : {}),
            ...(lookOf(s.look) ? { look: lookOf(s.look) } : {}),
          });
        }
        const a = msg.ask && typeof msg.ask === "object" ? msg.ask : {};
        shown = engine.viewShow({
          target: nums(a.target, 3),
          right: nums(a.right, 3),
          up: nums(a.up, 3),
          toEye: nums(a.toEye, 3),
          unitsPerPixel: typeof a.unitsPerPixel === "number" ? a.unitsPerPixel : undefined,
          rect: nums(a.rect, 4),
          px: typeof a.px === "number" ? a.px : undefined,
          test: a.test === true,
          scans: list,
          // The page's background, its scans kept and not drawn, and its own number for this ask.
          clear: nums(a.clear, 3),
          hide: a.hide === true,
          serial: typeof a.serial === "number" ? a.serial : undefined,
          // 1.36: eye-dome lighting as the page has it, and how long the ask is held so the page's own picture lands with it.
          edl: nums(a.edl, 4),
          holdMs: typeof a.holdMs === "number" && a.holdMs > 0 ? a.holdMs : undefined,
          // 1.37: depth shading, relief and face shading, each as the page's own factor.
          shade: nums(a.shade, 3),
        });
      }
      return { ...(shown !== undefined ? { shown } : {}), stats: engine.viewStats() };
    } catch (err) {
      return { error: String((err && err.message) || err) };
    }
  }

  /** Renders under way (1.28), by port: stopped when asked, or when the page goes. */
  const rendersOf = new WeakMap();

  /**
   * 1.28: a render, the page's ray tracer on every core: its scene put in a
   * BVH, its passes traced a batch at a time (about a second each), the
   * light summed so far sent after each (in parts when it's big), until
   * every pass is done or it's stopped.
   */
  async function render(port, msg) {
    const id = msg.id;
    let mine = rendersOf.get(port);
    if (!mine) {
      mine = new Map();
      rendersOf.set(port, mine);
    }
    const state = { stopped: false };
    mine.set(id, state);
    const say = (m) => {
      try {
        postInParts(port, m);
      } catch {
        state.stopped = true;
      }
    };
    let scene;
    let job;
    try {
      scene = engine.renderOpen(Array.isArray(msg.meshes) ? msg.meshes : []);
      job = engine.renderStart(scene, msg.ask);
      const total = Math.max(1, Math.min(10000, Math.floor(Number(msg.passes) || 1)));
      let done = 0;
      let batch = 1;
      while (done < total && !state.stopped) {
        const n = Math.min(batch, total - done);
        const t0 = Date.now();
        const got = await engine.renderStep(job, n);
        done = got.passes;
        batch = Math.max(1, Math.min(64, Math.round((n * 1000) / Math.max(1, Date.now() - t0))));
        if (!state.stopped) say({ kind: "render-progress", id, sum: got.sum, passes: got.passes, done: done >= total });
      }
      say({ kind: "render", id, passes: done, stopped: state.stopped });
    } catch (err) {
      say({ kind: "render", id, error: String((err && err.message) || err) });
    } finally {
      mine.delete(id);
      try {
        engine.renderClose(job ?? undefined, scene ?? undefined);
      } catch {
        // Gone already.
      }
    }
  }

  function answer(port, raw) {
    if (!raw || typeof raw !== "object") return;
    const msg = gathered(port, raw);
    if (!msg) return;
    const id = msg.id;
    if (msg.kind === "render") {
      void render(port, msg);
      return;
    }
    // 1.35 (the layer trial): what the engine's own window shows (the
    // page's view and scans, or the check's ring), and how it's going.
    if (msg.kind === "view" || msg.kind === "view-stats") {
      port.postMessage({ kind: msg.kind, id, ...viewAnswer(msg) });
      return;
    }
    // 1.36: the page's own classes for a scan, for the engine's window.
    if (msg.kind === "view-classes") {
      port.postMessage({ kind: "view-classes", id, key: msg.key, ...viewClasses(msg) });
      return;
    }
    // 1.29: the rules classifier's middle, on the engine's threads (the
    // page's scan-classify.ts voxelStage, as it is), answered in parts.
    if (msg.kind === "classify-stage") {
      let work;
      try {
        work = engine.classifyStage(msg.ask);
      } catch (err) {
        work = Promise.reject(err);
      }
      Promise.resolve(work).then(
        (got) => {
          log(`scan engine: classified ${got.n} voxels in ${Math.round(got.ms)} ms`);
          try {
            postInParts(port, { kind: "classify-stage", id, ...got });
          } catch {
            ports.delete(port);
          }
        },
        (err) => {
          try {
            port.postMessage({ kind: "classify-stage", id, error: String((err && err.message) || err) });
          } catch {
            ports.delete(port);
          }
        },
      );
      return;
    }
    if (msg.kind === "render-stop") {
      const state = rendersOf.get(port)?.get(id);
      if (state) state.stopped = true;
      return;
    }
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
        const setup = Number.isInteger(msg.setup) && msg.setup >= 0 ? msg.setup : undefined;
        const got = engine.region(held.handle, msg.affine, msg.min, msg.max, msg.ring ?? undefined, msg.most, setup);
        // 1.28: a big one (a scan's overview) in parts, when the page puts them together.
        const send = msg.parts === true ? (m) => postInParts(port, m) : (m) => port.postMessage(m);
        send({
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
          setupOf: got.setupOf ?? null,
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
    // 1.23: Geo W. over the page's points, on the engine's own threads; how
    // far it's come said now and then (classify-progress).
    if (msg.kind === "classify") {
      let work;
      try {
        work = engine.netClassify(msg.model, msg.blob, msg.points);
      } catch (err) {
        work = Promise.reject(err);
      }
      const say = (m) => {
        try {
          port.postMessage(m);
        } catch {
          ports.delete(port);
        }
      };
      let last = -1;
      const timer = every(() => {
        const done = engine.netProgress();
        if (done !== last) {
          last = done;
          say({ kind: "classify-progress", id, done });
        }
      }, PROGRESS_MS);
      Promise.resolve(work).then(
        (got) => {
          stop(timer);
          say({ kind: "classify", id, cls: got.cls, sure: got.sure, cubes: got.cubes, ms: got.ms });
        },
        (err) => {
          stop(timer);
          say({ kind: "classify", id, error: String((err && err.message) || err) });
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
      else
        asked.push({
          handle: held.handle,
          affine: s.affine,
          allow: s.allow ?? undefined,
          // 1.24: an E57 setup attached alone.
          setup: Number.isInteger(s.setup) && s.setup >= 0 ? s.setup : undefined,
        });
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
    port.on("close", () => {
      ports.delete(port);
      // Its renders stop with it (1.28).
      for (const state of rendersOf.get(port)?.values() ?? []) state.stopped = true;
    });
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
    else if (msg.kind === "crop") void crop(msg);
    else if (msg.kind === "page" && event.ports && event.ports[0]) addPage(event.ports[0]);
    // 1.35: the app's frame for the engine's own window (the layer trial), or no more of it.
    else if (msg.kind === "view-start") parentPort.postMessage({ kind: "view-started", ...viewStart(msg) });
    else if (msg.kind === "view-stop") viewStop();
  });

  return { scans, ports, answer };
}

module.exports = { createHost, postInParts, PART_BYTES };

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
