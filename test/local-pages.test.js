"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");
const { pageUrl, fileFor, registerScheme, serveLocalPages } = require("../src/local-pages");

const DIR = path.join("C:", "app.asar", "src");

test("the card and offline page have cadp://app/ addresses, with their query", () => {
  assert.equal(pageUrl("update.html", { splash: "1" }), "cadp://app/update.html?splash=1");
  assert.equal(pageUrl("offline.html"), "cadp://app/offline.html");
  assert.equal(
    pageUrl("offline.html", { reason: "ERR_INTERNET_DISCONNECTED (-106)" }),
    "cadp://app/offline.html?reason=ERR_INTERNET_DISCONNECTED+%28-106%29",
  );
});

test("only the listed files are served, from the app's folder", () => {
  assert.deepEqual(fileFor("cadp://app/update.html?splash=1", DIR), {
    file: path.join(DIR, "update.html"),
    type: "text/html; charset=utf-8",
  });
  assert.deepEqual(fileFor("cadp://app/splash.jpg", DIR), { file: path.join(DIR, "splash.jpg"), type: "image/jpeg" });
  for (const no of [
    "cadp://app/main.js",
    "cadp://app/../package.json",
    "cadp://app/%2e%2e/package.json",
    "cadp://app/..%2fpackage.json",
    "cadp://other/update.html",
    "file:///C:/app.asar/src/update.html",
    "https://cad-dza.caffeine.xyz/update.html",
    "not a url",
  ])
    assert.equal(fileFor(no, DIR), null, no);
});

test("cadp is registered as a standard, secure scheme, and answered from the files", async () => {
  const registered = [];
  registerScheme({ registerSchemesAsPrivileged: (list) => registered.push(...list) });
  assert.deepEqual(registered, [{ scheme: "cadp", privileges: { standard: true, secure: true } }]);

  let handler = null;
  const read = [];
  serveLocalPages({
    protocol: { handle: (scheme, fn) => scheme === "cadp" && (handler = fn) },
    dir: DIR,
    readFile: async (file) => {
      read.push(file);
      if (file.endsWith("splash.jpg")) throw new Error("gone");
      return Buffer.from("<p>card</p>");
    },
  });
  const page = await handler({ url: "cadp://app/update.html?splash=1" });
  assert.equal(page.status, 200);
  assert.equal(page.headers.get("content-type"), "text/html; charset=utf-8");
  assert.equal(await page.text(), "<p>card</p>");
  assert.equal((await handler({ url: "cadp://app/main.js" })).status, 404);
  assert.equal((await handler({ url: "cadp://app/splash.jpg" })).status, 404);
  assert.deepEqual(read, [path.join(DIR, "update.html"), path.join(DIR, "splash.jpg")]);
});
