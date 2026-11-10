"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const http = require("node:http");
const { createBrowserDrive, drivePageUrl, readDriveAnswer, HANDOFF_INFO } = require("../src/browser-drive");

const CLIENT = "1234567890-abcdefghijklmnop.apps.googleusercontent.com";
const subtle = globalThis.crypto.subtle;
const b64 = (u8) => Buffer.from(u8).toString("base64url");

// The web page's sealForDesktop (lib/storage/desktop-drive.ts), step for step.
async function sealAsThePageDoes(desktopKey, state, payload) {
  const theirs = await subtle.importKey("raw", Buffer.from(desktopKey, "base64url"), { name: "ECDH", namedCurve: "P-256" }, false, []);
  const mine = await subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, ["deriveBits"]);
  const shared = await subtle.deriveBits({ name: "ECDH", public: theirs }, mine.privateKey, 256);
  const base = await subtle.importKey("raw", shared, "HKDF", false, ["deriveKey"]);
  const key = await subtle.deriveKey(
    { name: "HKDF", hash: "SHA-256", salt: new TextEncoder().encode(state), info: new TextEncoder().encode(HANDOFF_INFO) },
    base,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt"],
  );
  const iv = globalThis.crypto.getRandomValues(new Uint8Array(12));
  const sealed = await subtle.encrypt({ name: "AES-GCM", iv }, key, new TextEncoder().encode(JSON.stringify(payload)));
  return { e: b64(new Uint8Array(await subtle.exportKey("raw", mine.publicKey))), n: b64(iv), c: b64(new Uint8Array(sealed)) };
}

function get(url) {
  return new Promise((resolve, reject) => {
    http.get(url, (res) => {
      let body = "";
      res.on("data", (d) => (body += d));
      res.on("end", () => resolve({ status: res.statusCode, body }));
    }).on("error", reject);
  });
}

test("the page's address carries the key, port, state and client id after #", () => {
  const url = new URL(drivePageUrl("https://cad-dza.caffeine.xyz", "KEY", 51234, "ab".repeat(32), CLIENT));
  assert.equal(url.pathname, "/desktop-drive");
  assert.equal(url.search, "");
  const q = new URLSearchParams(url.hash.slice(1));
  assert.equal(q.get("p"), "51234");
  assert.equal(q.get("c"), CLIENT);
});

test("Google's token comes back sealed, and only this connection's key opens it", async () => {
  let opened = null;
  let focused = 0;
  const drive = createBrowserDrive({
    appUrl: "https://cad-dza.caffeine.xyz",
    openExternal: (url) => {
      opened = new URL(url);
    },
    focusApp: () => (focused += 1),
  });
  const result = drive.connect(CLIENT);
  while (!opened) await new Promise((r) => setTimeout(r, 10));
  const q = new URLSearchParams(opened.hash.slice(1));
  const port = Number(q.get("p"));
  const state = q.get("s");
  assert.match(state, /^[0-9a-f]{64}$/);

  // Another state, or a token sealed to another key: refused, still waiting.
  const wrongState = await get(`http://127.0.0.1:${port}/drive?s=${"0".repeat(64)}&e=a&n=b&c=c`);
  assert.equal(wrongState.status, 400);
  const otherKey = (await subtle.exportKey("raw", (await subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, ["deriveBits"])).publicKey));
  const forOther = await sealAsThePageDoes(b64(new Uint8Array(otherKey)), state, { accessToken: "x" });
  const refused = await get(`http://127.0.0.1:${port}/drive?${new URLSearchParams({ s: state, ...forOther })}`);
  assert.equal(refused.status, 400);

  const sealed = await sealAsThePageDoes(q.get("k"), state, { accessToken: "ya29.token", expiresIn: 3599, email: "leo@example.com" });
  const done = await get(`http://127.0.0.1:${port}/drive?${new URLSearchParams({ s: state, ...sealed })}`);
  assert.equal(done.status, 200);
  assert.match(done.body, /Google Drive is connected/);
  assert.deepEqual(await result, { accessToken: "ya29.token", expiresIn: 3599, email: "leo@example.com" });
  assert.equal(focused, 1);
  assert.equal(drive.serverForTests, null);
});

test("a bad client id isn't sent to the browser, and cancelling rejects", async () => {
  const drive = createBrowserDrive({ appUrl: "https://x.example", openExternal: () => {} });
  await assert.rejects(drive.connect("not-a-client"), /client id/);
  const waiting = drive.connect(CLIENT);
  drive.cancel();
  await assert.rejects(waiting, /cancelled/);
  assert.deepEqual(readDriveAnswer("/done", "GET", "s", null), { status: 404, message: "Nothing here." });
});
