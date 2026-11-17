"use strict";
// A small LAS 1.2 file (point format 0) of given points, for the build's
// check of the scan engine on Windows (1.19) and for the tests: a scan
// written here, so the check never needs one of the person's.

/**
 * The bytes of a LAS 1.2 file of `points` ([x, y, z, class] each), its
 * coordinates on a grid of `scale` from `offset`.
 */
function lasBytes(points, { scale = 0.001, offset = [0, 0, 0] } = {}) {
  const HEAD = 227;
  const RECORD = 20;
  const buf = Buffer.alloc(HEAD + points.length * RECORD);
  buf.write("LASF", 0, "ascii");
  buf.writeUInt8(1, 24); // version 1.2
  buf.writeUInt8(2, 25);
  buf.write("CAD Protocol", 26, "ascii");
  buf.write("CAD Protocol check", 58, "ascii");
  buf.writeUInt16LE(1, 90); // day
  buf.writeUInt16LE(2026, 92); // year
  buf.writeUInt16LE(HEAD, 94);
  buf.writeUInt32LE(HEAD, 96); // where the points start
  buf.writeUInt32LE(0, 100); // no VLRs
  buf.writeUInt8(0, 104); // point format 0
  buf.writeUInt16LE(RECORD, 105);
  buf.writeUInt32LE(points.length, 107);
  buf.writeUInt32LE(points.length, 111); // all first returns
  for (let a = 0; a < 3; a += 1) {
    buf.writeDoubleLE(scale, 131 + a * 8);
    buf.writeDoubleLE(offset[a], 155 + a * 8);
  }
  const lo = [Infinity, Infinity, Infinity];
  const hi = [-Infinity, -Infinity, -Infinity];
  points.forEach(([x, y, z, cls], i) => {
    const at = HEAD + i * RECORD;
    [x, y, z].forEach((v, a) => {
      buf.writeInt32LE(Math.round((v - offset[a]) / scale), at + a * 4);
      lo[a] = Math.min(lo[a], v);
      hi[a] = Math.max(hi[a], v);
    });
    buf.writeUInt16LE(10, at + 12); // intensity
    buf.writeUInt8(0b00001001, at + 14); // return 1 of 1
    buf.writeUInt8(cls & 31, at + 15);
  });
  if (!points.length) lo.fill(0), hi.fill(0);
  // Max x, min x, max y, min y, max z, min z.
  for (let a = 0; a < 3; a += 1) {
    buf.writeDoubleLE(hi[a], 179 + a * 16);
    buf.writeDoubleLE(lo[a], 187 + a * 16);
  }
  return buf;
}

/**
 * A floor at z 100 (class 2), 0.1 apart over 10 × 10, and a post at (5, 5)
 * to z 104 (class 5), all moved by `at`.
 */
function samplePoints(at = [0, 0, 0]) {
  const pts = [];
  for (let i = 0; i < 100; i += 1) for (let j = 0; j < 100; j += 1) pts.push([at[0] + i * 0.1, at[1] + j * 0.1, at[2] + 100, 2]);
  for (let k = 0; k < 40; k += 1) pts.push([at[0] + 5, at[1] + 5, at[2] + 100.1 + k * 0.1, 5]);
  return pts;
}

module.exports = { lasBytes, samplePoints };
