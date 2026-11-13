"use strict";
// The faster graphics card (1.16; Leo, 2026-11-13 15:38: "add the high
// performance graphics card").
//
// A computer with two graphics cards (a laptop with Intel or AMD graphics
// built in and an NVIDIA or AMD card of its own) starts an app like this
// one on the built-in ones, unless someone has said otherwise for it in
// Windows' Graphics settings; and the built-in ones are what a scan's
// millions of points are slowest on. Chromium takes a switch that asks for
// the faster card for everything the app draws (force_high_performance_gpu,
// in Electron's list of command-line switches: "Force using discrete GPU
// when there are multiple GPUs available"). It must be on the command line
// before the app is ready, which is when the graphics process starts.
//
// A computer with one graphics card draws as before: there's nothing to
// choose between. Started with --force_low_power_gpu (a shortcut's target,
// say), the app keeps the built-in ones, for the battery: Chromium looks at
// that switch first, and this leaves its own off.
//
// Which card is in use is written to the app's log as it starts
// (cardsWords), and the web app's Scan Stats names the card drawing the
// scans.

/** Chromium's switches: the faster card, and the one that saves power. */
const HIGH = "force_high_performance_gpu";
const LOW = "force_low_power_gpu";

/**
 * Asks for the faster graphics card on the app's command line, unless the
 * app was started asking for the power-saving one. Before the app's ready.
 * Says which is asked for: "high" or "low" ("none" if the command line
 * couldn't be read or added to).
 */
function chooseGraphics(commandLine) {
  try {
    if (commandLine.hasSwitch(LOW)) return "low";
    if (!commandLine.hasSwitch(HIGH)) commandLine.appendSwitch(HIGH);
    return "high";
  } catch {
    return "none";
  }
}

/** The makers the app's report gives by number. */
const MAKERS = { 0x10de: "NVIDIA", 0x1002: "AMD", 0x1022: "AMD", 0x8086: "Intel", 0x1414: "Microsoft", 0x5143: "Qualcomm", 0x106b: "Apple" };

const hex = (n) => (Number.isFinite(n) ? Number(n).toString(16).padStart(4, "0") : "?");

/**
 * The graphics cards in the app's own report (app.getGPUInfo("basic")), in
 * a line for the log: "NVIDIA 2860 (in use), Intel a7a0". "" with none
 * listed.
 */
function cardsWords(info) {
  const list = info && Array.isArray(info.gpuDevice) ? info.gpuDevice : [];
  return list
    .filter((d) => d && typeof d === "object")
    .map((d) => `${MAKERS[d.vendorId] || `maker ${hex(d.vendorId)}`} ${hex(d.deviceId)}${d.active ? " (in use)" : ""}`)
    .join(", ");
}

module.exports = { HIGH, LOW, chooseGraphics, cardsWords };
