"use strict";
// This computer's memory and graphics cards (1.32; Leo, 2026-11-19 17:31:
// "when setting the scan vram and ram, can the app read the device specs and
// auto set it to the correct settings?"). The page sets the scans' memory
// from them (web v913, scan-caps.ts autoCaps). The computer's memory from
// Node; each graphics card's own memory from Windows' record of its driver
// (HardwareInformation.qwMemorySize, the full size: the older MemorySize and
// WMI's AdapterRAM stop at 4 GB). Read once a run, a few seconds at most;
// nothing kept and nothing sent anywhere.

const os = require("node:os");

/** The display adapters' driver keys, their name and memory, as JSON. */
const SCRIPT = [
  "$k='HKLM:\\SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}\\0*';",
  "Get-ItemProperty -Path $k -ErrorAction SilentlyContinue | ForEach-Object {",
  "  $q=$_.'HardwareInformation.qwMemorySize'; $m=$_.'HardwareInformation.MemorySize';",
  "  if ($m -is [byte[]]) { $m=[BitConverter]::ToUInt32($m,0) }",
  "  [pscustomobject]@{ name=[string]$_.DriverDesc; q=$q; m=$m }",
  "} | ConvertTo-Json -Compress",
].join(" ");

const MB = 1024 * 1024;

/** PowerShell's answer as cards: each with a name and its memory in MB (0 unknown). */
function cardsFrom(text) {
  let raw;
  try {
    raw = JSON.parse(String(text || "").trim() || "[]");
  } catch {
    return [];
  }
  const list = Array.isArray(raw) ? raw : [raw];
  const out = [];
  for (const c of list) {
    if (!c || typeof c !== "object") continue;
    const name = typeof c.name === "string" ? c.name.trim().slice(0, 120) : "";
    if (!name) continue;
    const q = Number(c.q);
    const m = Number(c.m);
    const bytes = q > 0 ? q : m > 0 ? m : 0;
    out.push({ name, vramMB: Math.round(bytes / MB) });
  }
  // The same card listed twice (a driver key per output): the bigger once.
  const best = new Map();
  for (const c of out) if (!best.has(c.name) || best.get(c.name).vramMB < c.vramMB) best.set(c.name, c);
  return [...best.values()].sort((a, b) => b.vramMB - a.vramMB);
}

/**
 * The specs: { ramMB, gpus: [{ name, vramMB }] }. `run(file, args, opts,
 * done)` is child_process.execFile (given, for tests); off Windows, or if
 * it fails, no cards.
 */
function createDeviceSpecs({ run, platform = process.platform, totalmem = os.totalmem, timeoutMs = 8000 }) {
  let asked = null;
  return {
    read() {
      if (asked) return asked;
      asked = new Promise((resolve) => {
        const ramMB = Math.round(totalmem() / MB);
        if (platform !== "win32" || !run) {
          resolve({ ramMB, gpus: [] });
          return;
        }
        try {
          run(
            "powershell.exe",
            ["-NoProfile", "-NonInteractive", "-Command", SCRIPT],
            { timeout: timeoutMs, windowsHide: true },
            (err, stdout) => resolve({ ramMB, gpus: err ? [] : cardsFrom(stdout) }),
          );
        } catch {
          resolve({ ramMB, gpus: [] });
        }
      });
      return asked;
    },
  };
}

module.exports = { createDeviceSpecs, cardsFrom, SCRIPT };
