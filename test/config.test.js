"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const config = require("../src/config");

const APP = config.APP_ORIGIN;
const OFFLINE = "file:///C:/Program%20Files/CAD%20Protocol/resources/app.asar/src/offline.html";

test("the app loads from the live address", () => {
  assert.equal(config.APP_URL, "https://cad-dza.caffeine.xyz/");
  assert.equal(APP, "https://cad-dza.caffeine.xyz");
});

test("window.open: sign-in, the app's own reports, and everything else outside", () => {
  assert.equal(config.openKind("https://id.ai/authorize"), "signin");
  assert.equal(config.openKind("https://beta.id.ai/authorize#x"), "signin");
  assert.equal(config.openKind("https://identity.ic0.app/#authorize"), "signin");
  assert.equal(config.openKind("https://identity.internetcomputer.org/"), "signin");
  assert.equal(config.openKind(`blob:${APP}/6f1c-4e`), "report");
  assert.equal(config.openKind("about:blank"), "report");
  assert.equal(config.openKind("https://github.com/leojls9744"), "external");
  assert.equal(config.openKind("mailto:someone@example.com"), "external");
  assert.equal(config.openKind(`${APP}/drawings`), "external");
  assert.equal(config.openKind("blob:https://elsewhere.example/1"), "deny");
  assert.equal(config.openKind("file:///C:/Windows/win.ini"), "deny");
  assert.equal(config.openKind("javascript:alert(1)"), "deny");
  assert.equal(config.openKind("not a url"), "deny");
  // A look-alike host is not Internet Identity.
  assert.equal(config.openKind("https://id.ai.example.com/authorize"), "external");
  assert.equal(config.openKind("https://evilid.ai/authorize"), "external");
});

test("a sign-in window may hand over to another https sign-in window", () => {
  assert.equal(config.openKind("https://accounts.google.com/o/oauth2", "signin"), "signin");
  assert.equal(config.openKind("http://accounts.google.com/", "signin"), "deny");
  assert.equal(config.openKind("https://accounts.google.com/", "main"), "external");
});

test("the main window stays on the app", () => {
  assert.equal(config.canNavigate("main", `${APP}/editor?id=4`, OFFLINE), true);
  assert.equal(config.canNavigate("main", `${OFFLINE}?reason=offline`, OFFLINE), true);
  assert.equal(config.canNavigate("main", "https://github.com/", OFFLINE), false);
  assert.equal(config.canNavigate("main", "https://cad-dza.caffeine.xyz.example.com/", OFFLINE), false);
  assert.equal(config.canNavigate("main", "file:///C:/Users/leo/scan.las", OFFLINE), false);
  assert.equal(config.canNavigate("main", "http://cad-dza.caffeine.xyz/", OFFLINE), false);
});

test("sign-in windows go to https pages only; report windows stay on the app's blobs", () => {
  assert.equal(config.canNavigate("signin", "https://accounts.google.com/signin", OFFLINE), true);
  assert.equal(config.canNavigate("signin", "http://example.com/", OFFLINE), false);
  assert.equal(config.canNavigate("signin", "file:///C:/", OFFLINE), false);
  assert.equal(config.canNavigate("report", `blob:${APP}/9`, OFFLINE), true);
  assert.equal(config.canNavigate("report", "https://github.com/", OFFLINE), false);
  assert.equal(config.canNavigate("unknown", `${APP}/`, OFFLINE), false);
});

test("permissions: a short list for the app, less for sign-in, none for anyone else", () => {
  assert.equal(config.permissionAllowed("clipboard-read", `${APP}/editor`), true);
  assert.equal(config.permissionAllowed("clipboard-read", APP), true);
  assert.equal(config.permissionAllowed("fullscreen", APP), true);
  assert.equal(config.permissionAllowed("media", APP), false);
  // The microphone for push-to-talk (1.8), never the camera.
  assert.equal(config.permissionAllowed("media", APP, { mediaTypes: ["audio"] }), true);
  assert.equal(config.permissionAllowed("media", APP, { mediaType: "audio" }), true);
  assert.equal(config.permissionAllowed("media", APP, { mediaTypes: ["audio", "video"] }), false);
  assert.equal(config.permissionAllowed("media", APP, { mediaType: "video" }), false);
  assert.equal(config.permissionAllowed("media", "https://id.ai", { mediaTypes: ["audio"] }), false);
  assert.equal(config.permissionAllowed("geolocation", APP), false);
  assert.equal(config.permissionAllowed("clipboard-read", "https://id.ai"), false);
  assert.equal(config.permissionAllowed("clipboard-sanitized-write", "https://id.ai"), true);
  assert.equal(config.permissionAllowed("notifications", "https://example.com"), false);
  assert.equal(config.permissionAllowed("clipboard-read", ""), false);
});

test("the user agent reads as plain Chrome", () => {
  const ua =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) CAD Protocol/1.0.0 Chrome/142.0.7444.59 Electron/45.2.3 Safari/537.36";
  assert.equal(
    config.plainUserAgent(ua),
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.7444.59 Safari/537.36",
  );
});
