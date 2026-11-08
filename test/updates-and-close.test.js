"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { parseVersion, isNewer, createUpdateCheck, RELEASES_URL } = require("../src/updates");
const { createCloseGuard } = require("../src/close-guard");

test("versions compare as numbers, with or without a v", () => {
  assert.deepEqual(parseVersion("v1.2.0"), [1, 2, 0]);
  assert.equal(parseVersion("1.2"), null);
  assert.equal(isNewer("1.10.0", "1.9.3"), true);
  assert.equal(isNewer("v1.2.0", "1.2.0"), false);
  assert.equal(isNewer("1.1.9", "1.2.0"), false);
  assert.equal(isNewer("junk", "1.2.0"), false);
});

test("the update check reads GitHub's newest release and keeps it for 6 hours", async () => {
  let calls = 0;
  let clock = 0;
  const check = createUpdateCheck({
    current: "1.2.0",
    now: () => clock,
    fetchJson: async () => {
      calls += 1;
      return { tag_name: "v1.3.0", html_url: "https://github.com/leojls9744/cad-protocol-desktop/releases/tag/v1.3.0" };
    },
  });
  assert.deepEqual(await check.get(), {
    current: "1.2.0",
    latest: "1.3.0",
    newer: true,
    url: "https://github.com/leojls9744/cad-protocol-desktop/releases/tag/v1.3.0",
  });
  clock = 5 * 3600 * 1000;
  await check.get();
  assert.equal(calls, 1);
  clock = 7 * 3600 * 1000;
  await check.get();
  assert.equal(calls, 2);
});

test("a failed check says nothing is known; a link elsewhere is replaced", async () => {
  const failing = createUpdateCheck({ current: "1.2.0", fetchJson: async () => { throw new Error("offline"); } });
  assert.deepEqual(await failing.get(), { current: "1.2.0", latest: null, newer: false, url: RELEASES_URL });
  const odd = createUpdateCheck({ current: "1.2.0", fetchJson: async () => ({ tag_name: "v1.2.0", html_url: "https://evil.example/x" }) });
  assert.deepEqual(await odd.get(), { current: "1.2.0", latest: "1.2.0", newer: false, url: RELEASES_URL });
});

function guard(options = {}) {
  const asked = [];
  let closed = 0;
  const g = createCloseGuard({ ask: (id) => asked.push(id), close: () => (closed += 1), ackMs: 30, maxMs: 120, ...options });
  return { g, asked, closed: () => closed };
}
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

test("closing asks the app, waits while it works, and closes when it's done", async () => {
  const { g, asked, closed } = guard();
  assert.equal(g.request(), true);
  assert.equal(g.request(), true); // a second close while asking: still held
  assert.deepEqual(asked, [1]);
  g.started(1);
  await wait(60); // past the 30 ms answer time: it started, so it waits
  assert.equal(closed(), 0);
  g.done(1);
  assert.equal(closed(), 1);
  assert.equal(g.closing, true);
  assert.equal(g.request(), false); // the real close goes through
});

test("an app that doesn't answer closes after 1.5 s (here 30 ms); one that hangs, after the limit", async () => {
  const silent = guard();
  silent.g.request();
  await wait(50);
  assert.equal(silent.closed(), 1);

  const hung = guard();
  hung.g.request();
  hung.g.started(1);
  await wait(60);
  assert.equal(hung.closed(), 0);
  await wait(100);
  assert.equal(hung.closed(), 1);
});

test("answers for an earlier question are ignored, and Stay asks again next time", async () => {
  const { g, asked, closed } = guard();
  g.request();
  g.started(1);
  g.reset();
  g.request();
  assert.deepEqual(asked, [1, 2]);
  g.done(1);
  assert.equal(closed(), 0);
  g.done(2);
  assert.equal(closed(), 1);
});
