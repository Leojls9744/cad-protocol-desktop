"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { EventEmitter } = require("node:events");
const { WIDTH, HEIGHT, NEXT_HEIGHT, overBounds, signInWindowOptions, createSignInWindows } = require("../src/signin-window");
const { createSignInCard } = require("../src/signin-card");
const config = require("../src/config");

const read = (file) => fs.readFileSync(path.join(__dirname, "..", "src", file), "utf8");

test("Internet Identity's window opens over the card, inside the screen", () => {
  const screen = { x: 0, y: 0, width: 1920, height: 1040 };
  const card = { x: 400, y: 220, width: 1120, height: 600 };
  assert.deepEqual(overBounds(card, screen), { x: 700, y: 140, width: WIDTH, height: HEIGHT });
  // A card at the screen's edge: the window stays on the screen.
  assert.deepEqual(overBounds({ x: 1500, y: 700, width: 1120, height: 600 }, screen), {
    x: 1920 - WIDTH,
    y: 1040 - HEIGHT,
    width: WIDTH,
    height: HEIGHT,
  });
  // A small screen: no bigger than it.
  assert.deepEqual(overBounds({ x: 0, y: 0, width: 1024, height: 728 }, { x: 0, y: 0, width: 1024, height: 728 }), {
    x: 252,
    y: 0,
    width: WIDTH,
    height: 728,
  });
  // A second screen to the left (its own origin).
  const left = { x: -1920, y: 0, width: 1920, height: 1040 };
  assert.equal(overBounds({ x: -1500, y: 200, width: 1120, height: 600 }, left).x, -1200);
});

test("it's a window of the app's own: titled, shown, in front of the card, with no way into the computer", () => {
  const card = { id: "card" };
  const options = signInWindowOptions({
    anchor: { x: 400, y: 220, width: 1120, height: 600 },
    workArea: { x: 0, y: 0, width: 1920, height: 1040 },
    parent: card,
    dark: true,
    webPreferences: { sandbox: true, contextIsolation: true },
  });
  assert.equal(options.title, "Internet Identity");
  assert.equal(options.parent, card);
  assert.equal(options.show, true);
  assert.equal(options.autoHideMenuBar, true);
  assert.equal(options.backgroundColor, "#121212");
  assert.deepEqual([options.x, options.y, options.width, options.height], [700, 140, WIDTH, HEIGHT]);
  assert.deepEqual(options.webPreferences, { sandbox: true, contextIsolation: true });
  // No preload: the sign-in pages get none of the desktop's features.
  assert.equal("preload" in options.webPreferences, false);
  // A provider's window (Google), opened from Internet Identity's: over it, light.
  const next = signInWindowOptions({
    anchor: { x: 700, y: 140, width: WIDTH, height: HEIGHT },
    workArea: { x: 0, y: 0, width: 1920, height: 1040 },
    next: true,
    dark: true,
  });
  assert.equal(next.height, NEXT_HEIGHT);
  assert.equal(next.backgroundColor, "#ffffff");
  assert.equal("parent" in next, false);
  assert.equal(next.y, 140 + (HEIGHT - NEXT_HEIGHT) / 2);
});

test("the sign-in windows that are open close together (Cancel on the card), and are forgotten as they close", () => {
  const made = () =>
    Object.assign(new EventEmitter(), {
      destroyed: false,
      isDestroyed() {
        return this.destroyed;
      },
      close() {
        this.destroyed = true;
        this.emit("closed");
      },
    });
  const windows = createSignInWindows();
  const identity = made();
  const google = made();
  windows.add(identity);
  windows.add(identity);
  windows.add(google);
  assert.equal(windows.count, 2);
  // One closes by itself (the provider's, once it's done).
  google.close();
  assert.equal(windows.count, 1);
  windows.closeAll();
  assert.ok(identity.destroyed);
  assert.equal(windows.count, 0);
  windows.closeAll();
});

test("the card has Sign in only when the app's page says it can sign in in the app's window", async () => {
  const made = [];
  class FakeWindow extends EventEmitter {
    constructor(options) {
      super();
      this.options = options;
      this.ran = [];
      this.destroyed = false;
      this.webContents = Object.assign(new EventEmitter(), {
        executeJavaScript: async (js) => {
          this.ran.push(js);
        },
      });
      made.push(this);
    }
    loadURL() {
      setImmediate(() => this.emit("ready-to-show"));
      return Promise.resolve();
    }
    show() {}
    focus() {}
    isMinimized() {
      return false;
    }
    getBounds() {
      return { x: 400, y: 220, width: 1120, height: 600 };
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
  // An older page of the app says nothing: the card's as it was.
  assert.equal(card.canWindow, false);
  assert.equal(card.window, null);
  card.can(undefined);
  assert.equal(card.canWindow, false);
  card.can({ window: "yes" });
  assert.equal(card.canWindow, false);
  card.can({ window: true });
  assert.equal(card.canWindow, true);
  card.open();
  await new Promise((r) => setImmediate(r));
  // The window Internet Identity's stays in front of, once the card shows.
  assert.equal(card.window, made[0]);
  assert.match(made[0].ran.at(-1), /"canWindow":true/);
  card.say({ phase: "waiting", text: "Internet Identity is open. Sign in there." });
  assert.match(made[0].ran.at(-1), /"phase":"waiting".*"canWindow":true/);
  card.close();
  assert.equal(card.window, null);
});

test("the card's page, its bridge and the app's page agree on the new button", () => {
  const html = read("signin-card.html");
  assert.match(html, /<button type="button" class="primary" id="window" hidden>Sign in<\/button>/);
  assert.match(html, /choose\("window"\)/);
  assert.match(html, /Sign in with your browser instead/);
  // Hidden means hidden, whatever the button's own display says.
  assert.match(html, /\[hidden\] \{ display: none !important; \}/);
  assert.match(read("card-preload.js"), /new Set\(\["window", "browser", "offline", "cancel", "quit"\]\)/);
  assert.match(read("preload.js"), /signIn: \(can\) => ipcRenderer\.invoke\("cadp:card:sign-in", can\)/);
  const main = read("main.js");
  // Only from a card that has the button.
  assert.match(main, /if \(kind === "window" && !signInCard\.canWindow\) return;/);
  assert.match(main, /\["window", "browser", "offline", "cancel"\]\.includes\(kind\)/);
  // Cancel closes the app's own sign-in windows as well as the browser's attempt.
  assert.match(main, /browserSignIn\.cancel\(\);\s+\/\/[^\n]*\n\s+signInWindows\.closeAll\(\);/);
  // Internet Identity, and what it opens, are still the only pages let through as sign-in windows.
  assert.equal(config.openKind("https://id.ai/#authorize"), "signin");
  assert.equal(config.openKind("https://accounts.google.com/o/oauth2/v2/auth", "signin"), "signin");
  assert.equal(config.openKind("https://accounts.google.com/o/oauth2/v2/auth"), "external");
  assert.equal(config.openKind("http://id.ai/"), "external");
});
