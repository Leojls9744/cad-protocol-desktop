"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const http = require("node:http");
const { createBrowserDrive, drivePageUrl, readDriveAnswer, pickRequest, pickedFrom, HANDOFF_INFO } = require("../src/browser-drive");

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

const API_KEY = "AIzaSyA1234567890abcdefghijklmnopqrstu";

test("1.10.2: picking in the browser: the request rides after #, and what's chosen comes back sealed", async () => {
  let opened = null;
  const drive = createBrowserDrive({
    appUrl: "https://cad-dza.caffeine.xyz",
    openExternal: (url) => {
      opened = new URL(url);
    },
  });
  const result = drive.connect(CLIENT, { pick: { kind: "folder", apiKey: API_KEY, title: "Choose a Drive folder for 26273", startIn: "abc_123" } });
  while (!opened) await new Promise((r) => setTimeout(r, 10));
  assert.equal(opened.search, "");
  const q = new URLSearchParams(opened.hash.slice(1));
  assert.equal(q.get("m"), "folder");
  assert.equal(q.get("a"), API_KEY);
  assert.equal(q.get("t"), "Choose a Drive folder for 26273");
  assert.equal(q.get("f"), "abc_123");
  const state = q.get("s");
  const picked = [
    { id: "folder_1", name: "Boundary", mimeType: "application/vnd.google-apps.folder", parentId: "abc_123", sizeBytes: null },
    { id: "../nope", name: "x" },
  ];
  const sealed = await sealAsThePageDoes(q.get("k"), state, { accessToken: "ya29.t", expiresIn: 3000, email: "leo@example.com", picked });
  const done = await get(`http://127.0.0.1:${Number(q.get("p"))}/drive?${new URLSearchParams({ s: state, ...sealed })}`);
  assert.equal(done.status, 200);
  assert.match(done.body, /Sent to CAD Protocol/);
  assert.deepEqual((await result).picked, [picked[0]]);
});

test("1.10.2: a pick that isn't one is turned down before the browser opens", async () => {
  let opens = 0;
  const drive = createBrowserDrive({ appUrl: "https://x.example", openExternal: () => (opens += 1) });
  await assert.rejects(drive.connect(CLIENT, { pick: { kind: "everything", apiKey: API_KEY } }), /Pick files or a folder/);
  await assert.rejects(drive.connect(CLIENT, { pick: { kind: "file", apiKey: "nope" } }), /API key/);
  assert.equal(opens, 0);
  assert.equal(pickRequest(undefined), null);
  assert.deepEqual(pickRequest({ kind: "file", apiKey: API_KEY, startIn: "bad/id" }), { kind: "file", apiKey: API_KEY, title: "", startIn: "" });
  assert.equal(pickedFrom(undefined), undefined);
  assert.deepEqual(pickedFrom([]), []);
  assert.equal(pickedFrom(Array.from({ length: 80 }, (_, i) => ({ id: `f${i}` }))).length, 50);
});
