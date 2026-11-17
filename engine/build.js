"use strict";
// Builds the scan engine's add-on (engine/node, Rust) and puts it where the
// app looks for it: engine/dist/cadp-engine.node (1.19). `npm run build:engine`.
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const root = __dirname;
execFileSync("cargo", ["build", "-p", "cadp-engine", "--release", "--manifest-path", path.join(root, "Cargo.toml")], {
  stdio: "inherit",
});
const built = {
  win32: "cadp_engine.dll",
  darwin: "libcadp_engine.dylib",
}[process.platform] || "libcadp_engine.so";
const from = path.join(root, "target", "release", built);
const to = path.join(root, "dist", "cadp-engine.node");
fs.mkdirSync(path.dirname(to), { recursive: true });
fs.copyFileSync(from, to);
console.log(`scan engine add-on: ${to} (${fs.statSync(to).size} bytes)`);
