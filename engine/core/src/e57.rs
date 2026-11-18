//! E57 scans (ASTM E2807) for the engine (desktop 1.24), read as the page
//! reads them (web lib/editor/e57.ts), so the engine's points are the
//! page's:
//!
//! - the file is pages of `pageSize` bytes (1,024), each ending in a 4-byte
//!   checksum (not checked here, as the page doesn't); the header gives
//!   where its XML is, the XML describes each scan (data3D): its fields
//!   (Float, Integer or ScaledInteger), how many points, its pose, and
//!   where its compressed vector section is;
//! - the section's data packets hold one byte stream a field, each going on
//!   from packet to packet: floats as they are, integers bit-packed least
//!   significant bit first in as few bits as their range needs;
//! - each scan's points placed by its pose (a quaternion's turn, then a
//!   shift); points marked invalid left out, and those at the scanner
//!   itself (0, 0, 0: no return); spherical points turned into x, y, z;
//! - colour to 0–255 by the red limits, intensity to 0–65,535 by its
//!   limits (else its field's range); each point's setup (its scan's place).
//!
//! The file is read a window at a time, never whole. `write` makes E57
//! files for the engine's tests.

use roxmltree::{Document, Node};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// A point as the page has it: the file's frame (metres, its setup's pose
/// applied), intensity (16 bits), colour (8 bits) and setup.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct E57Point {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub intensity: u16,
    pub rgb: [u8; 3],
    pub setup: u16,
}

/// What the file holds, before its points.
#[derive(Clone, Debug, Default)]
pub struct E57Facts {
    pub scans: usize,
    /// Each scan's points, as its description says (valid or not).
    pub records: u64,
    /// Any scan has colour.
    pub colour: bool,
}

fn bad(why: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, why.into())
}

/// The file's pages, read by logical offset a window at a time.
struct Pages {
    file: File,
    len: u64,
    size: u64,
    data: u64,
    window: Vec<u8>,
    base: u64,
}

const WINDOW: u64 = 8 << 20;

impl Pages {
    fn physical(&self, at: u64) -> u64 {
        (at / self.data) * self.size + at % self.data
    }

    fn logical(&self, physical: u64) -> u64 {
        physical - (physical / self.size) * 4
    }

    fn read(&mut self, at: u64, out: &mut [u8]) -> std::io::Result<()> {
        let length = out.len() as u64;
        if length == 0 {
            return Ok(());
        }
        let from = self.physical(at);
        let to = self.physical(at + length - 1) + 1;
        if to > self.len {
            return Err(bad("The E57 file ends early: it may not have finished saving."));
        }
        if from < self.base || to > self.base + self.window.len() as u64 {
            let end = self.len.min(to.max(from + WINDOW));
            self.window.resize((end - from) as usize, 0);
            self.file.seek(SeekFrom::Start(from))?;
            self.file.read_exact(&mut self.window)?;
            self.base = from;
        }
        let mut done = 0u64;
        while done < length {
            let page = (at + done) / self.data;
            let in_page = (at + done) % self.data;
            let start = (page * self.size + in_page - self.base) as usize;
            let take = (self.data - in_page).min(length - done) as usize;
            out[done as usize..done as usize + take].copy_from_slice(&self.window[start..start + take]);
            done += take as u64;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum Field {
    Float { size: usize },
    Int { bits: u32, min: f64, scale: f64, offset: f64 },
}

/// One field's values as its packets come in.
struct Stream {
    field: Field,
    buf: Vec<u8>,
    bit: u64,
}

impl Stream {
    fn push(&mut self, bytes: &[u8]) {
        let used = (self.bit >> 3) as usize;
        self.buf.drain(..used.min(self.buf.len()));
        self.bit &= 7;
        self.buf.extend_from_slice(bytes);
    }

    fn available(&self) -> u64 {
        let bits = (self.buf.len() as u64 * 8).saturating_sub(self.bit);
        match self.field {
            Field::Float { size } => bits / (size as u64 * 8),
            Field::Int { bits: 0, .. } => u64::MAX,
            Field::Int { bits: b, .. } => bits / b as u64,
        }
    }

    fn read(&mut self, n: usize, out: &mut [f64]) {
        match self.field {
            Field::Float { size } => {
                let mut at = (self.bit >> 3) as usize;
                for v in out.iter_mut().take(n) {
                    *v = if size == 4 {
                        f32::from_le_bytes(self.buf[at..at + 4].try_into().unwrap()) as f64
                    } else {
                        f64::from_le_bytes(self.buf[at..at + 8].try_into().unwrap())
                    };
                    at += size;
                }
                self.bit += (n * size * 8) as u64;
            }
            Field::Int { bits, min, scale, offset } => {
                for v in out.iter_mut().take(n) {
                    let mut got = 0u32;
                    let mut value: u64 = 0;
                    let mut pos = self.bit;
                    while got < bits {
                        let off = (pos & 7) as u32;
                        let take = (8 - off).min(bits - got);
                        let byte = self.buf[(pos >> 3) as usize] as u64;
                        value |= ((byte >> off) & ((1u64 << take) - 1)) << got;
                        got += take;
                        pos += take as u64;
                    }
                    self.bit = pos;
                    *v = (value as f64 + min) * scale + offset;
                }
            }
        }
    }
}

fn child<'a, 'i>(el: Option<Node<'a, 'i>>, name: &str) -> Option<Node<'a, 'i>> {
    el?.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

/// Its number (as the page's `num`: empty is 0, unreadable the fallback).
fn num(el: Option<Node>, fallback: f64) -> f64 {
    let Some(el) = el else { return fallback };
    let t: String = el.text().unwrap_or("").trim().to_string();
    if t.is_empty() {
        return 0.0;
    }
    match t.parse::<f64>() {
        Ok(v) if v.is_finite() => v,
        _ => fallback,
    }
}

fn field_of(el: Node) -> std::io::Result<(String, Field)> {
    let name = el.tag_name().name().to_string();
    match el.attribute("type") {
        Some("Float") => Ok((name, Field::Float { size: if el.attribute("precision") == Some("single") { 4 } else { 8 } })),
        Some(t @ ("Integer" | "ScaledInteger")) => {
            let lo: i128 = el.attribute("minimum").and_then(|v| v.trim().parse().ok()).unwrap_or(i64::MIN as i128);
            let hi: i128 = el.attribute("maximum").and_then(|v| v.trim().parse().ok()).unwrap_or(i64::MAX as i128);
            let range = hi - lo;
            let bits = if range <= 0 { 0 } else { 128 - range.leading_zeros() };
            if bits > 64 {
                return Err(bad(format!("The E57 field {name} uses {bits}-bit integers.")));
            }
            let scaled = t == "ScaledInteger";
            let attr = |k: &str, d: f64| el.attribute(k).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(d);
            Ok((
                name,
                Field::Int {
                    bits,
                    min: lo as f64,
                    scale: if scaled { attr("scale", 1.0) } else { 1.0 },
                    offset: if scaled { attr("offset", 0.0) } else { 0.0 },
                },
            ))
        }
        other => Err(bad(format!("The E57 field {name} is a {} type, which the engine can't read.", other.unwrap_or("missing")))),
    }
}

/// A scan's turn (as a matrix) and shift; none without a pose.
fn pose_of(scan: Node) -> ([f64; 9], [f64; 3]) {
    let pose = child(Some(scan), "pose");
    let rot = child(pose, "rotation");
    let tr = child(pose, "translation");
    let (mut w, mut x, mut y, mut z) = if rot.is_some() {
        (num(child(rot, "w"), 1.0), num(child(rot, "x"), 0.0), num(child(rot, "y"), 0.0), num(child(rot, "z"), 0.0))
    } else {
        (1.0, 0.0, 0.0, 0.0)
    };
    let mut len = (w * w + x * x + y * y + z * z).sqrt();
    if len == 0.0 {
        len = 1.0;
    }
    w /= len;
    x /= len;
    y /= len;
    z /= len;
    let r = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y - w * z),
        2.0 * (x * z + w * y),
        2.0 * (x * y + w * z),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z - w * x),
        2.0 * (x * z - w * y),
        2.0 * (y * z + w * x),
        1.0 - 2.0 * (x * x + y * y),
    ];
    let t = if tr.is_some() { [num(child(tr, "x"), 0.0), num(child(tr, "y"), 0.0), num(child(tr, "z"), 0.0)] } else { [0.0; 3] };
    (r, t)
}

fn limits_of(scan: Node, boxed: &str, lo: &str, hi: &str) -> Option<(f64, f64)> {
    let b = child(Some(scan), boxed)?;
    Some((num(child(Some(b), lo), 0.0), num(child(Some(b), hi), 0.0)))
}

/// JavaScript's Math.round: halves up.
#[inline]
fn js_round(v: f64) -> f64 {
    let f = v.floor();
    if v - f >= 0.5 {
        f + 1.0
    } else {
        f
    }
}

fn open(path: &Path) -> std::io::Result<(Pages, String)> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut head = [0u8; 48];
    if len < 48 {
        return Err(bad("This isn't an E57 scan (it doesn't start with ASTM-E57)."));
    }
    file.read_exact(&mut head)?;
    if &head[0..8] != b"ASTM-E57" {
        return Err(bad("This isn't an E57 scan (it doesn't start with ASTM-E57)."));
    }
    let major = u32::from_le_bytes(head[8..12].try_into().unwrap());
    if major != 1 {
        return Err(bad(format!("This is E57 version {major}; the engine reads version 1.")));
    }
    let xml_at = u64::from_le_bytes(head[24..32].try_into().unwrap());
    let xml_len = u64::from_le_bytes(head[32..40].try_into().unwrap());
    let size = match u64::from_le_bytes(head[40..48].try_into().unwrap()) {
        0 => 1024,
        s => s,
    };
    if size <= 4 || xml_len > 1 << 31 {
        return Err(bad("The E57 file's header isn't usable."));
    }
    let mut pages = Pages { file, len, size, data: size - 4, window: Vec::new(), base: 0 };
    let mut xml = vec![0u8; xml_len as usize];
    let at = pages.logical(xml_at);
    pages.read(at, &mut xml)?;
    let text = String::from_utf8(xml).map_err(|_| bad("The E57 file's description can't be read."))?;
    Ok((pages, text))
}

fn scans_of<'a, 'i>(doc: &'a Document<'i>) -> Vec<Node<'a, 'i>> {
    child(Some(doc.root_element()), "data3D").map(|d| d.children().filter(|c| c.is_element()).collect()).unwrap_or_default()
}

fn has_colour(scan: Node) -> bool {
    let proto = child(child(Some(scan), "points"), "prototype");
    ["colorRed", "colorGreen", "colorBlue"].iter().all(|c| child(proto, c).is_some())
}

/// Every valid point of the file in turn, as the page reads them (`visit`
/// false: no more wanted); `progress` with records read, of all.
pub fn read(path: &Path, mut visit: impl FnMut(&E57Point) -> bool, mut progress: impl FnMut(u64, u64)) -> std::io::Result<E57Facts> {
    let (mut pages, text) = open(path)?;
    let doc = Document::parse(&text).map_err(|_| bad("The E57 file's description can't be read."))?;
    let scans = scans_of(&doc);
    if scans.is_empty() {
        return Err(bad("The E57 file has no scans in it."));
    }
    if scans.len() > 4096 {
        return Err(bad("The E57 file has more than 4,096 setups."));
    }
    let counts: Vec<u64> = scans
        .iter()
        .map(|s| child(Some(*s), "points").and_then(|p| p.attribute("recordCount")).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0))
        .collect();
    let total: u64 = counts.iter().sum();
    let any_colour = scans.iter().any(|s| has_colour(*s));
    let facts = E57Facts { scans: scans.len(), records: total, colour: any_colour };
    let mut index: u64 = 0;
    for (k, scan) in scans.iter().enumerate() {
        let points = child(Some(*scan), "points");
        let proto = child(points, "prototype");
        let (Some(points), Some(proto)) = (points, proto) else { continue };
        if child(Some(points), "codecs").map(|c| c.children().any(|x| x.is_element())).unwrap_or(false) {
            return Err(bad("This E57 scan uses a compression the engine can't read."));
        }
        let mut names = Vec::new();
        let mut fields = Vec::new();
        for el in proto.children().filter(|c| c.is_element()) {
            let (n, f) = field_of(el)?;
            names.push(n);
            fields.push(f);
        }
        // Points of no bytes (every field one value) would come without end (1.25).
        if fields.iter().all(|f| matches!(f, Field::Int { bits: 0, .. })) {
            return Err(bad("An E57 scan's points hold no values to read."));
        }
        let at = |n: &str| names.iter().position(|x| x == n);
        let cart: Vec<Option<usize>> = ["cartesianX", "cartesianY", "cartesianZ"].iter().map(|n| at(n)).collect();
        let sph: Vec<Option<usize>> = ["sphericalRange", "sphericalAzimuth", "sphericalElevation"].iter().map(|n| at(n)).collect();
        let use_cart = cart.iter().all(|i| i.is_some());
        if !use_cart && !sph.iter().all(|i| i.is_some()) {
            return Err(bad("An E57 scan has no x, y, z or range and angles for its points."));
        }
        let xyz_at: Vec<usize> = if use_cart { cart.iter().map(|i| i.unwrap()).collect() } else { sph.iter().map(|i| i.unwrap()).collect() };
        let invalid_at = at(if use_cart { "cartesianInvalidState" } else { "sphericalInvalidState" });
        let colour_at: Vec<Option<usize>> = ["colorRed", "colorGreen", "colorBlue"].iter().map(|n| at(n)).collect();
        let colour = colour_at.iter().all(|i| i.is_some());
        let intensity_at = at("intensity");
        let (r_lo, r_hi) = limits_of(*scan, "colorLimits", "colorRedMinimum", "colorRedMaximum").unwrap_or((0.0, 255.0));
        let field_range = |i: usize| -> (f64, f64) {
            match fields[i] {
                Field::Int { bits, min, scale, offset } => (min * scale + offset, (min + 2f64.powi(bits as i32) - 1.0) * scale + offset),
                Field::Float { .. } => (0.0, 1.0),
            }
        };
        let (i_lo, i_hi) = match intensity_at {
            Some(i) => limits_of(*scan, "intensityLimits", "intensityMinimum", "intensityMaximum").unwrap_or_else(|| field_range(i)),
            None => (0.0, 1.0),
        };
        let (r, t) = pose_of(*scan);
        let mut streams: Vec<Stream> = fields.iter().map(|f| Stream { field: f.clone(), buf: Vec::new(), bit: 0 }).collect();
        let mut block: Vec<Vec<f64>> = vec![vec![0.0; 4096]; fields.len()];
        let offset: u64 = points.attribute("fileOffset").and_then(|v| v.parse().ok()).unwrap_or(0);
        let section_at = pages.logical(offset);
        let mut sh = [0u8; 32];
        pages.read(section_at, &mut sh)?;
        if sh[0] != 1 {
            return Err(bad("An E57 scan's points can't be found where the file says."));
        }
        let section_end = section_at + u64::from_le_bytes(sh[8..16].try_into().unwrap());
        let mut p = pages.logical(u64::from_le_bytes(sh[16..24].try_into().unwrap()));
        let mut left = counts[k];
        let c8 = |v: f64| js_round(255.0 * (v - r_lo) / (r_hi - r_lo).max(1e-12)).clamp(0.0, 255.0) as u8;
        let i16 = |v: f64| js_round(65535.0 * (v - i_lo) / (i_hi - i_lo).max(1e-12)).clamp(0.0, 65535.0) as u16;
        let mut packet = vec![0u8; 65536];
        while p < section_end && left > 0 {
            progress(index, total);
            let mut ph = [0u8; 6];
            pages.read(p, &mut ph)?;
            let kind = ph[0];
            let length = u16::from_le_bytes([ph[2], ph[3]]) as u64 + 1;
            if kind == 1 {
                let count = u16::from_le_bytes([ph[4], ph[5]]) as usize;
                if 6 + 2 * count > length as usize {
                    return Err(bad("An E57 data packet is cut short."));
                }
                let pk = &mut packet[..length as usize];
                pages.read(p, pk)?;
                let mut at2 = 6 + 2 * count;
                for (s, stream) in streams.iter_mut().enumerate().take(count) {
                    let len = u16::from_le_bytes([pk[6 + 2 * s], pk[7 + 2 * s]]) as usize;
                    if at2 + len > pk.len() {
                        return Err(bad("An E57 data packet is cut short."));
                    }
                    stream.push(&pk[at2..at2 + len]);
                    at2 += len;
                }
                let mut ready = streams.iter().map(|s| s.available()).min().unwrap_or(0).min(left);
                while ready > 0 {
                    let n = ready.min(4096) as usize;
                    for (s, stream) in streams.iter_mut().enumerate() {
                        stream.read(n, &mut block[s]);
                    }
                    for j in 0..n {
                        index += 1;
                        if let Some(iv) = invalid_at {
                            if block[iv][j] != 0.0 {
                                continue;
                            }
                        }
                        let (px, py, pz) = if use_cart {
                            (block[xyz_at[0]][j], block[xyz_at[1]][j], block[xyz_at[2]][j])
                        } else {
                            let (range, az, el) = (block[xyz_at[0]][j], block[xyz_at[1]][j], block[xyz_at[2]][j]);
                            (range * el.cos() * az.cos(), range * el.cos() * az.sin(), range * el.sin())
                        };
                        // No return: at the scanner itself.
                        if px == 0.0 && py == 0.0 && pz == 0.0 {
                            continue;
                        }
                        let pt = E57Point {
                            x: r[0] * px + r[1] * py + r[2] * pz + t[0],
                            y: r[3] * px + r[4] * py + r[5] * pz + t[1],
                            z: r[6] * px + r[7] * py + r[8] * pz + t[2],
                            intensity: intensity_at.map(|i| i16(block[i][j])).unwrap_or(0),
                            rgb: if colour {
                                [c8(block[colour_at[0].unwrap()][j]), c8(block[colour_at[1].unwrap()][j]), c8(block[colour_at[2].unwrap()][j])]
                            } else {
                                [0, 0, 0]
                            },
                            setup: k as u16,
                        };
                        if !visit(&pt) {
                            return Ok(facts);
                        }
                    }
                    ready -= n as u64;
                    left -= n as u64;
                    progress(index, total);
                }
            }
            p += length;
        }
    }
    progress(total, total);
    Ok(facts)
}

// ── Writing (the engine's tests, the app's own check) ────────────────────

/// A point to write: its place (x, y, z; or range, azimuth, elevation), intensity, colour, and whether it's marked invalid.
#[derive(Clone, Copy, Debug)]
pub struct WritePoint {
    pub at: [f64; 3],
    pub intensity: f64,
    pub rgb: [u8; 3],
    pub invalid: bool,
}

/// How a scan's places are written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Places {
    /// Doubles.
    Float,
    /// Whole numbers of `scale` (metres), from `min` to `max` of them.
    Scaled { scale: f64, min: i64, max: i64 },
    /// Range, azimuth, elevation, doubles.
    Spherical,
}

#[derive(Clone, Debug)]
pub struct WriteScan {
    pub name: String,
    /// Its turn (w, x, y, z) and shift.
    pub pose: Option<([f64; 4], [f64; 3])>,
    pub places: Places,
    /// Intensity written as whole numbers from 0 to this (none: doubles from 0 to 1, limits given).
    pub intensity_whole: Option<i64>,
    pub colour: bool,
    pub invalid_field: bool,
    pub points: Vec<WritePoint>,
}

/// CRC-32C (Castagnoli) of a page's data.
fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc: u32 = !0;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0x82F6_3B78 } else { crc >> 1 };
        }
    }
    !crc
}

/// Values bit-packed least significant bit first, `bits` each.
fn pack(values: &[u64], bits: u32) -> Vec<u8> {
    let mut out = vec![0u8; (values.len() * bits as usize).div_ceil(8)];
    let mut pos = 0usize;
    for &v in values {
        for b in 0..bits {
            if v >> b & 1 == 1 {
                out[pos >> 3] |= 1 << (pos & 7);
            }
            pos += 1;
        }
    }
    out
}

fn bits_for(range: i64) -> u32 {
    if range <= 0 {
        0
    } else {
        64 - (range as u64).leading_zeros()
    }
}

/// An E57 file of the scans given, written to `path` (1,024-byte pages with their checksums).
pub fn write(path: &Path, scans: &[WriteScan]) -> std::io::Result<()> {
    const PAGE: usize = 1024;
    const DATA: usize = PAGE - 4;
    // The file's logical bytes: the header, each scan's section, then the XML.
    let mut logical: Vec<u8> = vec![0u8; 48];
    let phys = |logical_at: usize| -> u64 { ((logical_at / DATA) * PAGE + logical_at % DATA) as u64 };
    let mut xml_scans = String::new();
    for (k, s) in scans.iter().enumerate() {
        while logical.len() % 4 != 0 {
            logical.push(0);
        }
        let section_at = logical.len();
        // The fields, in order: places, invalid, intensity, colour.
        let mut proto = String::new();
        let mut streams: Vec<Vec<u8>> = Vec::new();
        let names: [&str; 3] = if s.places == Places::Spherical {
            ["sphericalRange", "sphericalAzimuth", "sphericalElevation"]
        } else {
            ["cartesianX", "cartesianY", "cartesianZ"]
        };
        for (a, name) in names.iter().enumerate() {
            match s.places {
                Places::Scaled { scale, min, max } => {
                    proto += &format!("<{name} type=\"ScaledInteger\" minimum=\"{min}\" maximum=\"{max}\" scale=\"{scale:e}\" offset=\"0\"/>");
                    let vals: Vec<u64> = s.points.iter().map(|p| (((p.at[a] / scale).round() as i64).clamp(min, max) - min) as u64).collect();
                    streams.push(pack(&vals, bits_for(max - min)));
                }
                _ => {
                    proto += &format!("<{name} type=\"Float\"/>");
                    streams.push(s.points.iter().flat_map(|p| p.at[a].to_le_bytes()).collect());
                }
            }
        }
        if s.invalid_field {
            let state = if s.places == Places::Spherical { "sphericalInvalidState" } else { "cartesianInvalidState" };
            proto += &format!("<{state} type=\"Integer\" minimum=\"0\" maximum=\"2\"/>");
            let vals: Vec<u64> = s.points.iter().map(|p| if p.invalid { 1 } else { 0 }).collect();
            streams.push(pack(&vals, 2));
        }
        match s.intensity_whole {
            Some(top) => {
                proto += &format!("<intensity type=\"Integer\" minimum=\"0\" maximum=\"{top}\"/>");
                let vals: Vec<u64> = s.points.iter().map(|p| (p.intensity.round() as i64).clamp(0, top) as u64).collect();
                streams.push(pack(&vals, bits_for(top)));
            }
            None => {
                proto += "<intensity type=\"Float\" precision=\"single\"/>";
                streams.push(s.points.iter().flat_map(|p| (p.intensity as f32).to_le_bytes()).collect());
            }
        }
        if s.colour {
            for (c, name) in ["colorRed", "colorGreen", "colorBlue"].iter().enumerate() {
                proto += &format!("<{name} type=\"Integer\" minimum=\"0\" maximum=\"255\"/>");
                let vals: Vec<u64> = s.points.iter().map(|p| p.rgb[c] as u64).collect();
                streams.push(pack(&vals, 8));
            }
        }
        // The section: its header, then data packets of at most ~60 KB, each stream's bytes in turn.
        let total: usize = streams.iter().map(|b| b.len()).sum();
        let packets = total.div_ceil(60_000).max(1);
        let mut body: Vec<u8> = Vec::new();
        let data_at = section_at + 32;
        for i in 0..packets {
            let parts: Vec<&[u8]> = streams
                .iter()
                .map(|b| {
                    let each = b.len().div_ceil(packets);
                    let from = (i * each).min(b.len());
                    let to = ((i + 1) * each).min(b.len());
                    &b[from..to]
                })
                .collect();
            let mut pk = vec![1u8, 0, 0, 0];
            pk.extend_from_slice(&(parts.len() as u16).to_le_bytes());
            for part in &parts {
                pk.extend_from_slice(&(part.len() as u16).to_le_bytes());
            }
            for part in &parts {
                pk.extend_from_slice(part);
            }
            while pk.len() % 4 != 0 {
                pk.push(0);
            }
            let len = (pk.len() - 1) as u16;
            pk[2..4].copy_from_slice(&len.to_le_bytes());
            body.extend_from_slice(&pk);
        }
        let mut head = vec![0u8; 32];
        head[0] = 1;
        head[8..16].copy_from_slice(&((32 + body.len()) as u64).to_le_bytes());
        head[16..24].copy_from_slice(&phys(data_at).to_le_bytes());
        logical.extend_from_slice(&head);
        logical.extend_from_slice(&body);
        let pose = match s.pose {
            Some((q, t)) => format!(
                "<pose type=\"Structure\"><rotation type=\"Structure\"><w type=\"Float\">{:e}</w><x type=\"Float\">{:e}</x><y type=\"Float\">{:e}</y><z type=\"Float\">{:e}</z></rotation><translation type=\"Structure\"><x type=\"Float\">{:e}</x><y type=\"Float\">{:e}</y><z type=\"Float\">{:e}</z></translation></pose>",
                q[0], q[1], q[2], q[3], t[0], t[1], t[2]
            ),
            None => String::new(),
        };
        let limits = match s.intensity_whole {
            Some(top) => format!("<intensityLimits type=\"Structure\"><intensityMinimum type=\"Integer\">0</intensityMinimum><intensityMaximum type=\"Integer\">{top}</intensityMaximum></intensityLimits>"),
            None => "<intensityLimits type=\"Structure\"><intensityMinimum type=\"Float\">0</intensityMinimum><intensityMaximum type=\"Float\">1</intensityMaximum></intensityLimits>".into(),
        };
        let colour_limits = if s.colour {
            "<colorLimits type=\"Structure\"><colorRedMinimum type=\"Integer\">0</colorRedMinimum><colorRedMaximum type=\"Integer\">255</colorRedMaximum><colorGreenMinimum type=\"Integer\">0</colorGreenMinimum><colorGreenMaximum type=\"Integer\">255</colorGreenMaximum><colorBlueMinimum type=\"Integer\">0</colorBlueMinimum><colorBlueMaximum type=\"Integer\">255</colorBlueMaximum></colorLimits>"
        } else {
            ""
        };
        xml_scans += &format!(
            "<vectorChild type=\"Structure\"><guid type=\"String\"><![CDATA[{{cadp-check-scan-{k}}}]]></guid><name type=\"String\"><![CDATA[{}]]></name>{pose}{limits}{colour_limits}<points type=\"CompressedVector\" fileOffset=\"{}\" recordCount=\"{}\"><prototype type=\"Structure\">{proto}</prototype><codecs type=\"Vector\" allowHeterogeneousChildren=\"1\"></codecs></points></vectorChild>",
            s.name,
            phys(section_at),
            s.points.len()
        );
    }
    while logical.len() % 4 != 0 {
        logical.push(0);
    }
    let xml_at = logical.len();
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<e57Root type=\"Structure\" xmlns=\"http://www.astm.org/COMMIT/E57/2010-e57-v1.0\"><formatName type=\"String\"><![CDATA[ASTM E57 3D Imaging Data File]]></formatName><guid type=\"String\"><![CDATA[{{cadp-check}}]]></guid><versionMajor type=\"Integer\">1</versionMajor><versionMinor type=\"Integer\">0</versionMinor><coordinateMetadata type=\"String\"><![CDATA[]]></coordinateMetadata><data3D type=\"Vector\" allowHeterogeneousChildren=\"1\">{xml_scans}</data3D><images2D type=\"Vector\" allowHeterogeneousChildren=\"1\"></images2D></e57Root>"
    );
    logical.extend_from_slice(xml.as_bytes());
    let pages = logical.len().div_ceil(DATA);
    let physical_len = (pages * PAGE) as u64;
    logical[0..8].copy_from_slice(b"ASTM-E57");
    logical[8..12].copy_from_slice(&1u32.to_le_bytes());
    logical[12..16].copy_from_slice(&0u32.to_le_bytes());
    logical[16..24].copy_from_slice(&physical_len.to_le_bytes());
    logical[24..32].copy_from_slice(&phys(xml_at).to_le_bytes());
    logical[32..40].copy_from_slice(&(xml.len() as u64).to_le_bytes());
    logical[40..48].copy_from_slice(&(PAGE as u64).to_le_bytes());
    logical.resize(pages * DATA, 0);
    let mut out = std::io::BufWriter::new(File::create(path)?);
    for page in logical.chunks_exact(DATA) {
        out.write_all(page)?;
        out.write_all(&crc32c(page).to_be_bytes())?;
    }
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::crc32c;

    #[test]
    fn crc32c_is_castagnolis() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }
}
