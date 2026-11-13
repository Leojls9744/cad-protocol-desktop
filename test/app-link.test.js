"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { SCHEME, isAppLink, hasAppLink, registerAppLink } = require("../src/app-link");
const { launchedPaths } = require("../src/local-files");

const read = (file) => fs.readFileSync(path.join(__dirname, "..", file), "utf8");

test("cadprotocol:// links are known on the command line, however they're written", () => {
  assert.equal(SCHEME, "cadprotocol");
  for (const link of ["cadprotocol://open", "CADProtocol://Open", '"cadprotocol://open"', " cadprotocol://open/ ", "cadprotocol:open", "cadprotocol://open?from=unifyfield#x", "cadprotocol://anything/else"])
    assert.equal(isAppLink(link), true, link);
  for (const not of ["", "C:\\Jobs\\Lot 7.cadp", "--cadprotocol://open", "https://cad-dza.caffeine.xyz/", "cadprotocols://open", "xcadprotocol://open", null, 7])
    assert.equal(isAppLink(not), false, String(not));
  assert.equal(hasAppLink(["C:\\Program\\CAD Protocol.exe", "--allow-file-access-from-files", "cadprotocol://open"]), true);
  assert.equal(hasAppLink(["C:\\Program\\CAD Protocol.exe", "C:\\Jobs\\Lot 7.cadp"]), false);
  assert.equal(hasAppLink(undefined), false);
});

test("a link is never a drawing to open, and nothing beside one is either", () => {
  // Whatever the link ends with.
  assert.deepEqual(launchedPaths(["CAD Protocol.exe", "cadprotocol://open/C:/Jobs/Lot 7.cadp"]), []);
  assert.deepEqual(launchedPaths(["CAD Protocol.exe", "cadprotocol://open?file=C:\\Jobs\\Lot 7.cadp"]), []);
  assert.deepEqual(launchedPaths(["CAD Protocol.exe", "cadprotocol:C:\\Jobs\\Lot 7.cadp"]), []);
  // A path that rode in beside a link (a link made to break out of its quotes).
  assert.deepEqual(launchedPaths(["CAD Protocol.exe", "cadprotocol://open", "C:\\Jobs\\Lot 7.cadp"]), []);
  assert.deepEqual(launchedPaths(["CAD Protocol.exe", "C:\\Jobs\\Lot 7.cadp", '"cadprotocol://open"']), []);
  // A double-click, with no link, as before.
  assert.deepEqual(launchedPaths(["CAD Protocol.exe", "C:\\Jobs\\Lot 7.cadp"]), ["C:\\Jobs\\Lot 7.cadp"]);
});

test("the app says the links are its own: installed by itself, from its source with the program and its folder", () => {
  const calls = [];
  const app = {
    setAsDefaultProtocolClient: (...args) => {
      calls.push(args);
      return true;
    },
  };
  assert.equal(registerAppLink(app), true);
  assert.deepEqual(calls.pop(), ["cadprotocol"]);
  assert.equal(registerAppLink(app, { defaultApp: true, execPath: "C:\\el\\electron.exe", argv: ["C:\\el\\electron.exe", "."], resolve: (p) => `C:\\src\\${p}` }), true);
  assert.deepEqual(calls.pop(), ["cadprotocol", "C:\\el\\electron.exe", ["C:\\src\\."]]);
  // From its source with nothing to name: not said (Windows would start a bare Electron).
  assert.equal(registerAppLink(app, { defaultApp: true, execPath: "x", argv: ["x"] }), false);
  assert.equal(calls.length, 0);
  // Windows wouldn't take it: the app starts all the same.
  assert.equal(registerAppLink({ setAsDefaultProtocolClient: () => false }), false);
  assert.equal(
    registerAppLink({
      setAsDefaultProtocolClient: () => {
        throw new Error("no registry");
      },
    }),
    false,
  );
});

test("the installer registers the links, and the app says so before it checks for another copy of itself", () => {
  const pkg = JSON.parse(read("package.json"));
  assert.deepEqual(pkg.build.protocols, [{ name: "CAD Protocol", schemes: ["cadprotocol"] }]);
  // electron-builder's Windows installer doesn't act on build.protocols, so
  // the installer's own script writes the entry, and takes it out on uninstall.
  const nsh = read("build/installer.nsh");
  assert.match(nsh, /WriteRegStr SHELL_CONTEXT "Software\\Classes\\cadprotocol" "URL Protocol" ""/);
  assert.match(nsh, /WriteRegStr SHELL_CONTEXT "Software\\Classes\\cadprotocol\\shell\\open\\command" "" '"\$appExe" "%1"'/);
  assert.match(nsh, /!macro customUnInstall\s+\$\{ifNot\} \$\{isUpdated\}\s+DeleteRegKey SHELL_CONTEXT "Software\\Classes\\cadprotocol"/);
  // The .cadp icon refresh (1.11) is still in the same step.
  assert.match(nsh, /SHChangeNotify/);
  const main = read("src/main.js");
  const said = main.indexOf("registerAppLink(app, {");
  const lock = main.indexOf("app.requestSingleInstanceLock()");
  assert.ok(said > 0 && lock > 0 && said < lock, "the links are registered before the single-instance check");
  // A link while the app's open comes to the front as any second start does:
  // the same handler, the card or the start-up card first, nothing new read.
  const handler = main.slice(main.indexOf('app.on("second-instance"'), main.indexOf('app.on("web-contents-created"'));
  assert.match(handler, /if \(hasAppLink\(argv\)\) log\("brought to the front by a cadprotocol link"\);/);
  assert.match(handler, /signInCard\.focus\(\);[\s\S]*splash\.focus\(\);[\s\S]*mainWindow\.focus\(\);/);
  // The link itself is used for nothing but knowing it's one.
  assert.equal(/new URL\([^)]*argv/.test(main), false);
  assert.equal(read("src/app-link.js").includes("new URL("), false);
});
