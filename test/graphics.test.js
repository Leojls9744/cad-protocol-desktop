"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { HIGH, LOW, chooseGraphics, cardsWords } = require("../src/graphics");

const read = (file) => fs.readFileSync(path.join(__dirname, "..", file), "utf8");

/** A command line that knows its switches, as Electron's app.commandLine does. */
function commandLine(...had) {
  const switches = new Set(had);
  const added = [];
  return {
    added,
    hasSwitch: (name) => switches.has(name),
    appendSwitch: (name) => {
      switches.add(name);
      added.push(name);
    },
  };
}

test("the faster graphics card is asked for as the app starts", () => {
  assert.equal(HIGH, "force_high_performance_gpu");
  assert.equal(LOW, "force_low_power_gpu");
  const line = commandLine();
  assert.equal(chooseGraphics(line), "high");
  assert.deepEqual(line.added, [HIGH]);
  assert.equal(line.hasSwitch(HIGH), true);
});

test("asked for once: started with the switch already, nothing's added", () => {
  const line = commandLine(HIGH);
  assert.equal(chooseGraphics(line), "high");
  assert.deepEqual(line.added, []);
});

test("started asking for the power-saving card, the app keeps it", () => {
  const line = commandLine(LOW);
  assert.equal(chooseGraphics(line), "low");
  assert.deepEqual(line.added, []);
  assert.equal(line.hasSwitch(HIGH), false);
  // Both given: the power-saving one, as Chromium itself has it.
  const both = commandLine(LOW, HIGH);
  assert.equal(chooseGraphics(both), "low");
  assert.deepEqual(both.added, []);
});

test("a command line that can't be read or added to never stops the app", () => {
  assert.equal(chooseGraphics(null), "none");
  assert.equal(chooseGraphics({ hasSwitch: () => false, appendSwitch: () => { throw new Error("no"); } }), "none");
});

test("the cards are named for the log, the one in use marked", () => {
  assert.equal(
    cardsWords({ gpuDevice: [{ active: true, vendorId: 0x10de, deviceId: 0x2860 }, { active: false, vendorId: 0x8086, deviceId: 0xa7a0 }] }),
    "NVIDIA 2860 (in use), Intel a7a0",
  );
  assert.equal(cardsWords({ gpuDevice: [{ active: true, vendorId: 0x1414, deviceId: 0x8c }] }), "Microsoft 008c (in use)");
  assert.equal(cardsWords({ gpuDevice: [{ active: false, vendorId: 0x1002, deviceId: 0x73df }, { active: true, vendorId: 0x1234, deviceId: 1 }] }), "AMD 73df, maker 1234 0001 (in use)");
  assert.equal(cardsWords({ gpuDevice: [null, "x", { vendorId: Number.NaN }] }), "maker ? ?");
  assert.equal(cardsWords({}), "");
  assert.equal(cardsWords(null), "");
});

test("the app asks before it's ready, says so in its log, and the build checks it on Windows", () => {
  const main = read("src/main.js");
  const asked = main.indexOf("const graphicsAsked = chooseGraphics(app.commandLine);");
  assert.ok(asked > 0, "asked for");
  // Before the links, the smoke check and anything waiting on the app being ready.
  assert.ok(asked < main.indexOf("const linksAreOurs = registerAppLink("));
  assert.ok(asked < main.indexOf("app.whenReady()"));
  assert.match(main, /log\(`graphics: asked for the \$\{graphicsAsked === "low" \? "power-saving" : "faster"\} card; \$\{cardsWords\(info\) \|\| "none listed"\}`\)/);
  assert.match(main, /result\.graphics = \{\s+asked: graphicsAsked,\s+onCommandLine: app\.commandLine\.hasSwitch\(FASTER_CARD\),/);
  const build = read(".github/workflows/build.yml");
  assert.match(build, /\$r\.graphics\.asked -ne "high"/);
  assert.match(build, /\$r\.graphics\.onCommandLine/);
  assert.equal(JSON.parse(read("package.json")).version, "1.16.0");
  assert.match(read("README.md"), /force_low_power_gpu/);
});
