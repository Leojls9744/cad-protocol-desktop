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
 * its index (not indexed twice).
 */
async function checkEngine({ utilityProcess, MessageChannelMain, dir: srcDir, log = () => {} }) {
  const os = require("node:os");
  const { lasBytes, samplePoints } = require("./las-sample");
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "cadp-engine-check-"));
  const scan = path.join(dir, "Check scan.las");
  fs.writeFileSync(scan, lasBytes(samplePoints([6000000, 2000000, 0]), { offset: [6000000, 2000000, 0] }));
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
    // This process plays the page: the port's other end kept here.
    const port = await new Promise((resolve, reject) => {
      const fake = { postMessage: (_channel, _msg, ports) => resolve(ports[0]) };
      if (!engine.connect(fake)) reject(new Error("No port to the scan engine."));
    });
    const heard = [];
    const waitFor = (test, ms) =>
      new Promise((resolve, reject) => {
        const t = setTimeout(() => reject(new Error(`The scan engine didn't answer (${round}): ${JSON.stringify(heard.slice(-3))}`)), ms);
        const look = () => {
          const m = heard.find(test);
          if (m) {
            clearTimeout(t);
            resolve(m);
          } else setTimeout(look, 20);
        };
        look();
      });
    port.on("message", (e) => heard.push(e.data));
    port.start();
    const hello = await waitFor((m) => m.kind === "hello", 20000);
    // Opened before the port came (its index made before), the hello says so.
    const said = (m) =>
      m.kind === "hello" ? (m.scans || []).find((x) => x.token === reg.token && x.state !== "indexing") : null;
    const heardOpen = await waitFor((m) => ((m.kind === "opened" || m.kind === "failed") && m.token === reg.token) || said(m), 60000);
    const early = said(heardOpen);
    const opened = early ? { kind: early.state === "open" ? "opened" : "failed", info: early.info, error: early.error } : heardOpen;
    if (opened.kind === "failed") throw new Error(`The scan engine couldn't open the check's scan: ${opened.error}`);
    const t0 = Date.now();
    port.postMessage({
      kind: "snap",
      id: 1,
      scans: [{ token: reg.token, affine: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0] }],
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
    });
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

module.exports = { checkEngine, c2cPair };
