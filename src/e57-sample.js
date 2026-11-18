"use strict";
// A small E57 file (1.24) for the build's check of the scan engine on
// Windows: setups of points as doubles, each moved by its pose, with
// intensity and colour, written as the format has it (1,024-byte pages,
// each ending in its CRC-32C).

const PAGE = 1024;
const DATA = PAGE - 4;

/** CRC-32C (Castagnoli). */
function crc32c(bytes) {
  let crc = ~0 >>> 0;
  for (const b of bytes) {
    crc ^= b;
    for (let k = 0; k < 8; k += 1) crc = crc & 1 ? (crc >>> 1) ^ 0x82f63b78 : crc >>> 1;
  }
  return ~crc >>> 0;
}

const physical = (logical) => Math.floor(logical / DATA) * PAGE + (logical % DATA);

/**
 * The file's bytes for `setups`: each { name, pose: { t: [x, y, z] } (no
 * turn), points: [[x, y, z, intensity 0–2047, r, g, b], …] }.
 */
function e57Bytes(setups) {
  const parts = [Buffer.alloc(48)];
  let at = 48;
  const push = (b) => {
    parts.push(b);
    at += b.length;
  };
  const pad = () => {
    if (at % 4) push(Buffer.alloc(4 - (at % 4)));
  };
  let scansXml = "";
  setups.forEach((s, k) => {
    pad();
    const sectionAt = at;
    const n = s.points.length;
    const xs = Buffer.alloc(n * 8);
    const ys = Buffer.alloc(n * 8);
    const zs = Buffer.alloc(n * 8);
    const is = Buffer.alloc(n * 2);
    const rs = Buffer.alloc(n);
    const gs = Buffer.alloc(n);
    const bs = Buffer.alloc(n);
    s.points.forEach(([x, y, z, i, r, g, b], j) => {
      xs.writeDoubleLE(x, j * 8);
      ys.writeDoubleLE(y, j * 8);
      zs.writeDoubleLE(z, j * 8);
      is.writeUInt16LE(Math.max(0, Math.min(2047, Math.round(i))), j * 2);
      rs[j] = r;
      gs[j] = g;
      bs[j] = b;
    });
    // Intensity bit-packed in 11 bits, least significant first.
    const packed = Buffer.alloc(Math.ceil((n * 11) / 8));
    let bit = 0;
    for (let j = 0; j < n; j += 1) {
      const v = is.readUInt16LE(j * 2);
      for (let q = 0; q < 11; q += 1, bit += 1) if ((v >> q) & 1) packed[bit >> 3] |= 1 << (bit & 7);
    }
    const streams = [xs, ys, zs, packed, rs, gs, bs];
    const packets = Math.max(1, Math.ceil(streams.reduce((t, b) => t + b.length, 0) / 60000));
    const body = [];
    for (let p = 0; p < packets; p += 1) {
      const pieces = streams.map((b) => {
        const each = Math.ceil(b.length / packets);
        return b.subarray(Math.min(b.length, p * each), Math.min(b.length, (p + 1) * each));
      });
      const head = Buffer.alloc(6 + 2 * pieces.length);
      head[0] = 1;
      head.writeUInt16LE(pieces.length, 4);
      pieces.forEach((piece, q) => head.writeUInt16LE(piece.length, 6 + 2 * q));
      let packet = Buffer.concat([head, ...pieces]);
      if (packet.length % 4) packet = Buffer.concat([packet, Buffer.alloc(4 - (packet.length % 4))]);
      packet.writeUInt16LE(packet.length - 1, 2);
      body.push(packet);
    }
    const bodyBytes = Buffer.concat(body);
    const sh = Buffer.alloc(32);
    sh[0] = 1;
    sh.writeBigUInt64LE(BigInt(32 + bodyBytes.length), 8);
    sh.writeBigUInt64LE(BigInt(physical(sectionAt + 32)), 16);
    push(sh);
    push(bodyBytes);
    const [tx, ty, tz] = s.pose?.t ?? [0, 0, 0];
    scansXml +=
      `<vectorChild type="Structure"><guid type="String"><![CDATA[{cadp-check-${k}}]]></guid><name type="String"><![CDATA[${s.name}]]></name>` +
      `<pose type="Structure"><rotation type="Structure"><w type="Float">1</w><x type="Float">0</x><y type="Float">0</y><z type="Float">0</z></rotation>` +
      `<translation type="Structure"><x type="Float">${tx}</x><y type="Float">${ty}</y><z type="Float">${tz}</z></translation></pose>` +
      `<intensityLimits type="Structure"><intensityMinimum type="Integer">0</intensityMinimum><intensityMaximum type="Integer">2047</intensityMaximum></intensityLimits>` +
      `<colorLimits type="Structure"><colorRedMinimum type="Integer">0</colorRedMinimum><colorRedMaximum type="Integer">255</colorRedMaximum></colorLimits>` +
      `<points type="CompressedVector" fileOffset="${physical(sectionAt)}" recordCount="${n}"><prototype type="Structure">` +
      `<cartesianX type="Float"/><cartesianY type="Float"/><cartesianZ type="Float"/>` +
      `<intensity type="Integer" minimum="0" maximum="2047"/>` +
      `<colorRed type="Integer" minimum="0" maximum="255"/><colorGreen type="Integer" minimum="0" maximum="255"/><colorBlue type="Integer" minimum="0" maximum="255"/>` +
      `</prototype><codecs type="Vector" allowHeterogeneousChildren="1"></codecs></points></vectorChild>`;
  });
  pad();
  const xmlAt = at;
  const xml = Buffer.from(
    `<?xml version="1.0" encoding="UTF-8"?>\n<e57Root type="Structure" xmlns="http://www.astm.org/COMMIT/E57/2010-e57-v1.0">` +
      `<formatName type="String"><![CDATA[ASTM E57 3D Imaging Data File]]></formatName><guid type="String"><![CDATA[{cadp-check}]]></guid>` +
      `<versionMajor type="Integer">1</versionMajor><versionMinor type="Integer">0</versionMinor>` +
      `<data3D type="Vector" allowHeterogeneousChildren="1">${scansXml}</data3D><images2D type="Vector" allowHeterogeneousChildren="1"></images2D></e57Root>`,
    "utf8",
  );
  push(xml);
  const logical = Buffer.concat(parts);
  const pages = Math.ceil(logical.length / DATA);
  logical.write("ASTM-E57", 0, "latin1");
  logical.writeUInt32LE(1, 8);
  logical.writeUInt32LE(0, 12);
  logical.writeBigUInt64LE(BigInt(pages * PAGE), 16);
  logical.writeBigUInt64LE(BigInt(physical(xmlAt)), 24);
  logical.writeBigUInt64LE(BigInt(xml.length), 32);
  logical.writeBigUInt64LE(BigInt(PAGE), 40);
  const out = Buffer.alloc(pages * PAGE);
  for (let p = 0; p < pages; p += 1) {
    const data = Buffer.alloc(DATA);
    logical.copy(data, 0, p * DATA, Math.min(logical.length, (p + 1) * DATA));
    data.copy(out, p * PAGE);
    out.writeUInt32BE(crc32c(data), p * PAGE + DATA);
  }
  return out;
}

/**
 * The check's two setups: the las-sample's floor and post (0.1 apart), the
 * second 30 m east, z on the first at 100, the second at 200.
 */
function sampleSetups() {
  const floor = (z) => {
    const out = [];
    for (let i = 0; i <= 50; i += 1) for (let j = 0; j <= 50; j += 1) out.push([i * 0.1, j * 0.1, z, (i * 37) % 2048, 10, 20, 30]);
    return out;
  };
  return [
    { name: "Station 1", pose: { t: [1916000, 640000, 0] }, points: floor(100) },
    { name: "Station 2", pose: { t: [1916030, 640000, 0] }, points: floor(200) },
  ];
}

module.exports = { e57Bytes, sampleSetups };
