"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const { createSignInCard, cardSize, cleanState, WIDTH, HEIGHT } = require("../src/signin-card");
const { createLaunch } = require("../src/launch");
const { fileFor } = require("../src/local-pages");

test("the sign-in card is four times the start-up card, never over 90% of the screen", () => {
  assert.equal(WIDTH * HEIGHT, 4 * 560 * 300);
  assert.deepEqual(cardSize({ width: 1920, height: 1040 }), { width: 1120, height: 600 });
  assert.deepEqual(cardSize({ width: 1000, height: 600 }), { width: 900, height: 540 });
});

test("it takes only a phase, words and a note from the app's page", () => {
  assert.deepEqual(cleanState({ phase: "waiting", text: "Sign in in your browser", extra: 1 }), {
    phase: "waiting",
    text: "Sign in in your browser",
    note: "",
  });
  assert.equal(cleanState({ phase: "<script>" }).phase, "ready");
  assert.equal(cleanState({ text: "x".repeat(900) }).text.length, 300);
  assert.equal(cleanState(null).phase, "ready");
});

test("its page is served from cadp://app/", () => {
  assert.ok(fileFor("cadp://app/signin-card.html", "/x"));
});

test("it opens once it's drawn, frameless, with its own preload, says what the page says, and closes", async () => {
  const made = [];
  class FakeWindow extends EventEmitter {
    constructor(options) {
      super();
      this.options = options;
      this.shown = false;
      this.ran = [];
      this.destroyed = false;
      this.webContents = Object.assign(new EventEmitter(), {
        executeJavaScript: async (js) => {
          this.ran.push(js);
        },
      });
      made.push(this);
    }
    loadURL(url) {
      this.url = url;
      setImmediate(() => this.emit("ready-to-show"));
      return Promise.resolve();
    }
    show() {
      this.shown = true;
    }
    focus() {}
    isMinimized() {
      return false;
    }
    isDestroyed() {
      return this.destroyed;
    }
    close() {
      this.destroyed = true;
      this.emit("closed");
    }
  }
  const card = createSignInCard({
    BrowserWindow: FakeWindow,
    workArea: () => ({ width: 1920, height: 1040 }),
    url: "cadp://app/signin-card.html",
    preload: "/app/card-preload.js",
    webPreferences: { sandbox: true },
  });
  let then = 0;
  card.open(() => (then += 1));
  await new Promise((r) => setImmediate(r));
  const win = made[0];
  assert.equal(win.url, "cadp://app/signin-card.html");
  assert.equal(win.options.frame, false);
  assert.equal(win.options.width, 1120);
  assert.equal(win.options.webPreferences.preload, "/app/card-preload.js");
  assert.equal(win.options.webPreferences.sandbox, true);
  assert.ok(win.shown && card.isShown);
  assert.equal(then, 1);
  card.say({ phase: "finishing", text: "Signing you in…" });
  assert.match(win.ran.at(-1), /"phase":"finishing"/);
  card.close();
  assert.equal(card.isShown, false);
});

test("signed out, the window waits for the card, and shows once signed in", () => {
  const shown = [];
  const timers = [];
  const launch = createLaunch({
    showMain: (why) => shown.push(why),
    closeSplash: () => {},
    setTimer: (fn) => {
      timers.push(fn);
      return timers.length;
    },
    clearTimer: () => {},
  });
  launch.loading();
  launch.loaded();
  launch.gate();
  for (const fn of timers) fn();
  launch.ready();
  assert.deepEqual(shown, []);
  launch.ungate();
  assert.deepEqual(shown, ["slow"]);
});
