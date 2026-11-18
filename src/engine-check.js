"use strict";
// The build's check of the scan engine (1.19), run by the packaged app on
// Windows (--smoke-test, src/main.js) and by hand anywhere Electron runs.

const fs = require("node:fs");
const path = require("node:path");
const { addonPath, createScanEngine } = require("./scan-engine");

/**
 * The build's check of the scan engine (1.19) on real Windows: its add-on
 * in the packaged app, its own process started, a small scan written here
 * indexed, a point under a cursor found, then the scan opened again from
 * its index (not indexed twice). 1.25: the engine's process stopped, the
 * page told, the engine started again with its scans, and asked again.
 */
async function checkEngine({ utilityProcess, MessageChannelMain, dir: srcDir, log = () => {} }) {
  const os = require("node:os");
  const { lasBytes, samplePoints } = require("./las-sample");
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "cadp-engine-check-"));
  const scan = path.join(dir, "Check scan.las");
  fs.writeFileSync(scan, lasBytes(samplePoints([6000000, 2000000, 0]), { offset: [6000000, 2000000, 0] }));
  // 1.24: an E57 of two setups.
  const { e57Bytes, sampleSetups } = require("./e57-sample");
  const e57 = path.join(dir, "Check scan.e57");
  fs.writeFileSync(e57, e57Bytes(sampleSetups()));
  const result = { addon: addonPath(srcDir) };
  const once = async (round) => {
    const engine = createScanEngine({
      fork: (modulePath, args, options) => utilityProcess.fork(modulePath, args, options),
      newChannel: () => new MessageChannelMain(),
      statFile: (p) => fs.promises.stat(p).catch(() => null),
      exists: (p) => fs.existsSync(p),
      cacheDir: path.join(dir, "cache"),
      addon: addonPath(srcDir),
      hostPath: path.join(srcDir, "engine-host.js"),
      log,
    });
    if (!engine.available) throw new Error(`The scan engine's add-on isn't in the app: ${result.addon}`);
    const reg = await engine.register(scan);
    if (!reg) throw new Error("The scan engine wouldn't take the check's scan.");
    const regE57 = await engine.register(e57);
    if (!regE57) throw new Error("The scan engine wouldn't take the check's E57 scan.");
    // This process plays the page: the port's other end kept here, and what the page is told.
    const told = [];
    let portCame = null;
    const fake = {
      postMessage: (channel, msg, ports) => {
        if (channel === "cadp:engine:port") portCame?.(ports[0]);
        else told.push({ channel, msg });
      },
    };
    const portNow = () =>
      new Promise((resolve, reject) => {
        portCame = resolve;
        if (!engine.connect(fake)) reject(new Error("No port to the scan engine."));
      });
    let heard = [];
    /** Settles when `test` holds of something heard (1.25: no more looking once given up). */
    const until = (look, ms, what) =>
      new Promise((resolve, reject) => {
        let over = false;
        const t = setTimeout(() => {
          over = true;
          reject(new Error(`The scan engine didn't answer (${round}): ${what()}`));
        }, ms);
        const again = () => {
          if (over) return;
          const m = look();
          if (m) {
            clearTimeout(t);
            resolve(m);
          } else setTimeout(again, 20);
        };
        again();
      });
    const waitFor = (test, ms) => until(() => heard.find(test), ms, () => JSON.stringify(heard.slice(-3)));
    const listen = (p) => {
      p.on("message", (e) => heard.push(e.data));
      p.start();
    };
    let port = await portNow();
    listen(port);
    const hello = await waitFor((m) => m.kind === "hello", 20000);
    // Opened before the port came (its index made before), the hello says so.
    const said = (m) =>
      m.kind === "hello" ? (m.scans || []).find((x) => x.token === reg.token && x.state !== "indexing") : null;
    const heardOpen = await waitFor((m) => ((m.kind === "opened" || m.kind === "failed") && m.token === reg.token) || said(m), 60000);
    const early = said(heardOpen);
    const opened = early ? { kind: early.state === "open" ? "opened" : "failed", info: early.info, error: early.error } : heardOpen;
    const saidE57 = (m) =>
      m.kind === "hello" ? (m.scans || []).find((x) => x.token === regE57.token && x.state !== "indexing") : null;
    const heardE57 = await waitFor((m) => ((m.kind === "opened" || m.kind === "failed") && m.token === regE57.token) || saidE57(m), 60000);
    const earlyE57 = saidE57(heardE57);
    const openedE57 = earlyE57 ? { kind: earlyE57.state === "open" ? "opened" : "failed", info: earlyE57.info, error: earlyE57.error } : heardE57;
    if (openedE57.kind === "failed") throw new Error(`The scan engine couldn't open the check's E57 scan: ${openedE57.error}`);
    if (opened.kind === "failed") throw new Error(`The scan engine couldn't open the check's scan: ${opened.error}`);
    const t0 = Date.now();
    port.postMessage({ ...snapAsk(reg.token), id: 1 });
    const answer = await waitFor((m) => m.kind === "snap" && m.id === 1, 20000);
    // 1.20: every point in a box, and a tenth of them.
    const all = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0];
    port.postMessage({ kind: "region", id: 2, token: reg.token, affine: all, min: [5999999, 1999999, 0], max: [6000011, 2000011, 200], most: 1000000 });
    port.postMessage({ kind: "region", id: 3, token: reg.token, affine: all, min: [5999999, 1999999, 0], max: [6000011, 2000011, 200], most: 1000 });
    const every = await waitFor((m) => m.kind === "region" && m.id === 2, 20000);
    const tenth = await waitFor((m) => m.kind === "region" && m.id === 3, 20000);
    // 1.22: cloud to cloud on the page's points: a floor and two walls seen
    // twice, the second 0.2 off east, lined up; then the two measured.
    const pair = c2cPair();
    port.postMessage({
      kind: "align",
      id: 4,
      fixed: pair.fixed,
      moving: pair.moving,
      ask: { fit: [1, 0, 0, 0.17, 0, 1, 0, 0, 0, 0, 1, 0], fitRms: 0.03, level: true, refine: true, foot: 1, tolerance: 0.05 },
    });
    port.postMessage({ kind: "check", id: 5, scans: [pair.fixed, pair.fixed], foot: 1, tolerance: 0.05 });
    const lined = await waitFor((m) => m.kind === "align" && m.id === 4, 60000);
    const measured = await waitFor((m) => m.kind === "check" && m.id === 5, 60000);
    // 1.23: Geo W.: a small model over a floor's points.
    const net = netCheck();
    port.postMessage({ kind: "classify", id: 6, model: net.model, blob: net.blob, points: net.points });
    const classed = await waitFor((m) => m.kind === "classify" && m.id === 6, 60000);
    // 1.24: the E57's second setup snapped to from above; the first setup's floor alone in a box.
    port.postMessage({
      kind: "snap",
      id: 7,
      scans: [{ token: regE57.token, affine: all }],
      ask: { target: [1916032.5, 640002.5, 0], right: [1, 0, 0], up: [0, 1, 0], toEye: [0, 0, 1], unitsPerPixel: 0.01, halfW: 200, halfH: 150, cursor: [200, 150], reach: 8, frontPx: 2, window: 0, most: 32, frontMost: 8 },
    });
    port.postMessage({ kind: "region", id: 8, token: regE57.token, affine: all, min: [1915990, 639990, -1000], max: [1916050, 640010, 1000], most: 1000000, setup: 0 });
    const e57Snap = await waitFor((m) => m.kind === "snap" && m.id === 7, 20000);
    const e57Box = await waitFor((m) => m.kind === "region" && m.id === 8, 20000);
    // 1.25: the engine's process stopped; the page told, then told it's back; a new port, the scan open again, a snap.
    let restart = null;
    if (round === "opened again") {
      const pid = engine.pid;
      if (!pid) throw new Error("The scan engine's process has no id.");
      process.kill(pid);
      const lost = await until(() => told.find((t) => t.channel === "cadp:engine:lost"), 20000, () => JSON.stringify(told));
      await until(() => told.find((t) => t.channel === "cadp:engine:back"), 20000, () => JSON.stringify(told));
      port.close();
      heard = [];
      port = await portNow();
      listen(port);
      const back = (m) =>
        (m.kind === "opened" && m.token === reg.token) || (m.kind === "hello" && (m.scans || []).some((x) => x.token === reg.token && x.state === "open"));
      await waitFor(back, 60000);
      port.postMessage({ ...snapAsk(reg.token), id: 9 });
      const after = await waitFor((m) => m.kind === "snap" && m.id === 9, 20000);
      restart = {
        again: lost.msg.again === true,
        dropped: Array.isArray(lost.msg.dropped) ? lost.msg.dropped.length : -1,
        running: engine.running,
        error: after.error || null,
        near: after.near ? after.near.length / 14 : 0,
        frontZ: after.front && after.front.length >= 5 ? after.front[4] : null,
      };
    }
    port.close();
    engine.stop();
    const front = answer.front || [];
    return {
      version: hello.version,
      count: opened.info.count,
      built: opened.info.built,
      ms: Date.now() - t0,
      error: answer.error || null,
      near: answer.near ? answer.near.length / 14 : 0,
      // The post's top, 104, nearest the eye from above.
      frontZ: front.length >= 5 ? front[4] : null,
      frontClass: front.length >= 8 ? front[7] : null,
      region: { count: every.count, complete: every.complete, error: every.error || null },
      thinned: { count: tenth.count, complete: tenth.complete },
      align: { error: lined.error || null, east: lined.align ? lined.align[3] : null, refined: lined.refined === true },
      check: { error: measured.error || null, pairs: measured.pairs ? measured.pairs.length : 0, gaps: measured.gaps ? measured.gaps[0].length : 0 },
      classify: { error: classed.error || null, points: classed.cls ? classed.cls.length : 0, cubes: classed.cubes || 0 },
      restart,
      e57: {
        count: openedE57.info ? openedE57.info.count : 0,
        setups: openedE57.info ? openedE57.info.setups === true : false,
        snapError: e57Snap.error || null,
        frontZ: e57Snap.front && e57Snap.front.length >= 5 ? e57Snap.front[4] : null,
        boxError: e57Box.error || null,
        firstSetup: e57Box.count || 0,
        setupOf: e57Box.setupOf ? [...new Set(e57Box.setupOf)] : null,
      },
    };
  };
  try {
    result.first = await once("indexed");
    result.again = await once("opened again");
  } finally {
    try {
      fs.rmSync(dir, { recursive: true, force: true });
    } catch {
      // Left in the temp folder.
    }
  }
  return result;
}

/** The check's snap: straight down on the post at (6000005, 2000005). */
function snapAsk(token) {
  return {
    kind: "snap",
    scans: [{ token, affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0] }],
    ask: {
      target: [6000005, 2000005, 0],
      right: [1, 0, 0],
      up: [0, 1, 0],
      toEye: [0, 0, 1],
      unitsPerPixel: 0.01,
      halfW: 200,
      halfH: 150,
      cursor: [200, 150],
      reach: 8,
      frontPx: 2,
      window: 0,
      most: 32,
      frontMost: 8,
    },
  };
}

/** Two scans of a floor and two walls (1.22's check), the second 0.2 off east. */
function c2cPair(n = 20000) {
  let seed = 9;
  const rnd = () => (seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0) / 4294967296;
  const scene = (east) => {
    const out = new Float64Array(n * 3);
    for (let i = 0; i < n; i += 1) {
      const k = rnd();
      const p = k < 0.5 ? [rnd() * 30, rnd() * 30, 100] : k < 0.75 ? [0, rnd() * 30, 100 + rnd() * 8] : [rnd() * 30, 0, 100 + rnd() * 8];
      out[i * 3] = p[0] + (rnd() - 0.5) * 0.01 - east;
      out[i * 3 + 1] = p[1] + (rnd() - 0.5) * 0.01;
      out[i * 3 + 2] = p[2] + (rnd() - 0.5) * 0.01;
    }
    return out;
  };
  return { fixed: scene(0), moving: scene(0.2) };
}

/** A small Geo W. model (version 1, nine classes) and a floor's 5000 points (1.23's check). */
function netCheck(n = 5000) {
  let floats = 0;
  const layer = (i, o) => {
    const L = { inp: i, out: o, w: floats, b: floats + i * o };
    floats += i * o + o;
    return L;
  };
  const branches = [8, 4, 4].map((c) => [layer(c, 8), layer(8, 8)]);
  const head = [layer(8 * 3 + 2, 8), layer(8, 9)];
  const blob = new Float32Array(floats).map((_, i) => Math.sin(i) * 0.3);
  const px = new Float64Array(n).map((_, i) => (i % 70) * 0.3);
  const py = new Float64Array(n).map((_, i) => Math.floor(i / 70) * 0.3);
  const pz = new Float64Array(n).map((_, i) => 100 + (i % 7) * 0.05);
  const intensity = new Float64Array(n).fill(0.5);
  return { model: { branches, head, classes: 9, inputs: [] }, blob, points: { px, py, pz, intensity } };
}

module.exports = { checkEngine, c2cPair, netCheck };
