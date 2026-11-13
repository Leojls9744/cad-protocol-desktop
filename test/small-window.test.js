"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const {
  WIDTH,
  HEIGHT,
  WINDOW_CLASS,
  isChromium,
  overRect,
  sameRect,
  overlaps,
  regValue,
  exeFromCommand,
  registryBrowser,
  windowsBackend,
  createSmallWindow,
} = require("../src/small-window");

const CHROME = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
const EDGE = "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe";
const FIREFOX = "C:\\Program Files\\Mozilla Firefox\\firefox.exe";
const URL_SMALL = "https://cad-dza.caffeine.xyz/desktop-sign-in#k=ab12&p=51234&s=ff00&w=1";
const RECT = { x: 690, y: 140, width: 540, height: 760 };
const TITLES = ["CAD Protocol", "cad-dza.caffeine.xyz", "desktop-sign-in"];
/** Quick, so the tests are. */
const QUICK = { pollMs: 5, fastForMs: 100, slowPollMs: 5, findForMs: 150, settleMs: [5, 5], watchMs: 5 };
const OWN_PID = 4242;
const BROWSER_PID = 500;
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// The small window's own timers never keep the app running; something has to keep the tests running.
const running = setInterval(() => {}, 1000);
test.after(() => clearInterval(running));

/** Windows' windows, as far as the small window asks about them. */
function fakeWindows({ browser = { exe: CHROME, how: "windows" }, supported = true } = {}) {
  const wins = new Map();
  const exes = new Map([[BROWSER_PID, CHROME]]);
  const calls = [];
  let next = 1000;
  const backend = {
    supported,
    defaultBrowser: () => browser,
    handles: () => [...wins.keys()],
    info: (h) => {
      const w = wins.get(h);
      return w
        ? { visible: w.visible, maximized: Boolean(w.maximized), cls: w.cls, title: w.title, pid: w.pid }
        : { visible: false, maximized: false, cls: "", title: "", pid: 0 };
    },
    exeOf: (pid) => exes.get(pid) ?? null,
    exists: (h) => wins.has(h),
    rect: (h) => (wins.has(h) ? { ...wins.get(h).rect } : null),
    place: (h, r) => {
      calls.push(["place", h, { ...r }]);
      wins.get(h).rect = { ...r };
      return true;
    },
    raise: (h) => calls.push(["raise", h]),
    close: (h) => {
      calls.push(["close", h]);
      wins.delete(h);
      return true;
    },
  };
  return {
    backend,
    wins,
    exes,
    calls,
    /** A window appears; as the browser opens one unless said otherwise. */
    add(win = {}) {
      const h = next;
      next += 2;
      wins.set(h, { visible: true, cls: WINDOW_CLASS, title: "CAD Protocol", pid: BROWSER_PID, rect: { x: 10, y: 10, width: 945, height: 1060 }, ...win });
      return h;
    },
    touched: () => [...new Set(calls.map((c) => c[1]))],
  };
}

/** A browser that starts (or doesn't), and what it was started with. */
function fakeSpawn({ fails = false, onStart = () => {} } = {}) {
  const started = [];
  const spawn = (exe, args, options) => {
    const child = new EventEmitter();
    child.unref = () => {
      child.unrefed = true;
    };
    started.push({ exe, args, options, child });
    setImmediate(() => {
      if (fails) child.emit("error", new Error("spawn ENOENT"));
      else {
        child.emit("spawn");
        onStart();
      }
    });
    return child;
  };
  return { spawn, started };
}

function small(windows, spawn) {
  return createSmallWindow({ backend: windows.backend, spawn, ownPid: OWN_PID, timing: QUICK });
}

test("Chrome, Edge and Brave have a small window; other browsers don't", () => {
  assert.equal(isChromium(CHROME), true);
  assert.equal(isChromium(EDGE), true);
  assert.equal(isChromium("C:\\Users\\leo\\AppData\\Local\\BraveSoftware\\Brave-Browser\\Application\\BRAVE.EXE"), true);
  assert.equal(isChromium(FIREFOX), false);
  assert.equal(isChromium("C:\\evil\\notchrome.exe"), false);
  assert.equal(isChromium(""), false);
  assert.equal(isChromium(null), false);
});

test("it goes over the middle of the card, inside the screen", () => {
  assert.deepEqual([WIDTH, HEIGHT], [540, 760]);
  const screen = { x: 0, y: 0, width: 1920, height: 1040 };
  // The card, 1120 x 600, centred.
  assert.deepEqual(overRect({ x: 400, y: 220, width: 1120, height: 600 }, screen), { x: 690, y: 140, width: WIDTH, height: HEIGHT });
  // A card near the top: kept on the screen.
  assert.deepEqual(overRect({ x: 400, y: 0, width: 1120, height: 600 }, screen), { x: 690, y: 0, width: WIDTH, height: HEIGHT });
  // A second screen to the left.
  assert.deepEqual(overRect({ x: -1500, y: 200, width: 1120, height: 600 }, { x: -1920, y: 0, width: 1920, height: 1040 }), {
    x: -1210,
    y: 120,
    width: WIDTH,
    height: HEIGHT,
  });
  // A small screen: no bigger than it.
  assert.deepEqual(overRect({ x: 0, y: 0, width: 1024, height: 560 }, { x: 0, y: 0, width: 1024, height: 560 }), {
    x: 242,
    y: 0,
    width: WIDTH,
    height: 560,
  });
});

test("windows overlap when they share any of the screen", () => {
  assert.equal(overlaps(RECT, { x: 710, y: 240, width: 500, height: 600 }), true);
  assert.equal(overlaps(RECT, { x: 0, y: 0, width: 690, height: 900 }), false); // side by side
  assert.equal(overlaps(RECT, null), false);
});

test("rectangles a pixel or two apart are the same place", () => {
  assert.equal(sameRect(RECT, { ...RECT, x: RECT.x + 2 }), true);
  assert.equal(sameRect(RECT, { ...RECT, width: 945 }), false);
  assert.equal(sameRect(RECT, null), false);
});

test("the default browser is read from the registry's answers", () => {
  const choice = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\https\\UserChoice\r\n    ProgId    REG_SZ    ChromeHTML\r\n\r\n";
  assert.equal(regValue(choice), "ChromeHTML");
  const command = `\r\nHKEY_CLASSES_ROOT\\ChromeHTML\\shell\\open\\command\r\n    (Default)    REG_SZ    "${CHROME}" --single-argument %1\r\n\r\n`;
  assert.equal(regValue(command), `"${CHROME}" --single-argument %1`);
  // The value's name is in the person's language.
  assert.equal(regValue(command.replace("(Default)", "(Standard)")), `"${CHROME}" --single-argument %1`);
  assert.equal(regValue("ERROR: The system was unable to find the specified registry key or value."), null);
  assert.equal(regValue(""), null);

  assert.equal(exeFromCommand(`"${CHROME}" --single-argument %1`), CHROME);
  assert.equal(exeFromCommand(`${FIREFOX} -osint -url "%1"`), FIREFOX);
  assert.equal(exeFromCommand('"%ProgramFiles%\\Google\\Chrome\\Application\\chrome.exe" %1', { ProgramFiles: "C:\\Program Files" }), CHROME);
  assert.equal(exeFromCommand("rundll32 something"), null);

  const asked = [];
  const answers = (table) => (args) => {
    asked.push(args[1]);
    return table[args[1]] || "";
  };
  const base = "HKCU\\Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\https\\";
  assert.equal(registryBrowser(answers({ [`${base}UserChoice`]: choice, "HKCR\\ChromeHTML\\shell\\open\\command": command })), CHROME);
  // Windows' newer place for the choice is read first.
  assert.equal(asked[0], `${base}UserChoiceLatest\\ProgId`);
  assert.equal(
    registryBrowser(
      answers({
        [`${base}UserChoiceLatest\\ProgId`]: choice.replace("ChromeHTML", "MSEdgeHTM"),
        [`${base}UserChoice`]: choice,
        "HKCR\\MSEdgeHTM\\shell\\open\\command": command.replace(CHROME, EDGE),
      }),
    ),
    EDGE,
  );
  // Nothing chosen, or a name that isn't one: no browser.
  assert.equal(registryBrowser(answers({})), null);
  assert.equal(registryBrowser(answers({ [`${base}UserChoice`]: choice.replace("ChromeHTML", "a\\..\\b") })), null);
});

test("away from Windows there's no small window", async () => {
  if (process.platform !== "win32") assert.equal(windowsBackend().supported, false);
  const windows = fakeWindows({ supported: false });
  const { spawn, started } = fakeSpawn();
  assert.equal(await small(windows, spawn).open(URL_SMALL, { rect: RECT, titles: TITLES }), null);
  assert.equal(started.length, 0);
});

test("with Firefox, or no browser Windows will name, the page opens in a tab instead", async () => {
  for (const browser of [{ exe: FIREFOX, how: "windows" }, null]) {
    const windows = fakeWindows({ browser });
    const { spawn, started } = fakeSpawn();
    const window = small(windows, spawn);
    assert.equal(window.browser(), null);
    assert.equal(await window.open(URL_SMALL, { rect: RECT, titles: TITLES }), null);
    assert.equal(started.length, 0);
  }
});

test("only web pages open in it", async () => {
  const windows = fakeWindows();
  const { spawn, started } = fakeSpawn();
  assert.equal(await small(windows, spawn).open("file:///C:/Windows/win.ini", { rect: RECT, titles: TITLES }), null);
  assert.equal(await small(windows, spawn).open("--user-data-dir=C:\\x", { rect: RECT, titles: TITLES }), null);
  assert.equal(started.length, 0);
});

test("the browser starts with the page as an app window and no size of its own; its new window goes over the card", async () => {
  const windows = fakeWindows();
  // Already open: the person's browser with CAD Protocol in a tab, and this app's own card.
  const theirs = windows.add({ title: "CAD Protocol - Google Chrome" });
  const card = windows.add({ title: "Sign in to CAD Protocol", pid: OWN_PID });
  let opened = null;
  const { spawn, started } = fakeSpawn({
    onStart: () => {
      // Its window shows a moment later, where the browser last had it.
      setTimeout(() => {
        opened = windows.add({ title: "cad-dza.caffeine.xyz/desktop-sign-in" });
      }, 12);
    },
  });
  const handle = await small(windows, spawn).open(URL_SMALL, { rect: RECT, titles: TITLES });
  assert.ok(handle);
  assert.equal(started.length, 1);
  assert.equal(started[0].exe, CHROME);
  // Nothing but the page: size and place switches would size the person's other windows too.
  assert.deepEqual(started[0].args, [`--app=${URL_SMALL}`]);
  assert.equal(started[0].options.detached, true);
  assert.equal(started[0].options.shell, undefined);
  assert.equal(started[0].child.unrefed, true);

  const placed = await handle.placed;
  assert.deepEqual(placed.rect, RECT);
  assert.equal(handle.isOpen, true);
  assert.deepEqual(windows.wins.get(opened).rect, RECT);
  assert.deepEqual(windows.calls[0], ["place", opened, RECT]);
  assert.deepEqual(windows.calls[1], ["raise", opened]);
  // The person's own browser window and the app's card were never touched.
  assert.deepEqual(windows.touched(), [opened]);
  assert.deepEqual(windows.wins.get(theirs).rect, { x: 10, y: 10, width: 945, height: 1060 });
  assert.ok(windows.wins.has(card));
  handle.close();
});

test("a new window of this app, of another program, or with another title is never taken for it", async () => {
  const windows = fakeWindows();
  windows.exes.set(700, "C:\\Program Files\\Slack\\slack.exe");
  const { spawn } = fakeSpawn({
    onStart: () => {
      windows.add({ title: "CAD Protocol", pid: OWN_PID }); // the app's own window showing
      windows.add({ title: "CAD Protocol | Slack", pid: 700 });
      windows.add({ title: "New Tab - Google Chrome" });
      windows.add({ title: "CAD Protocol", visible: false });
      windows.add({ title: "CAD Protocol", cls: "MozillaWindowClass" });
    },
  });
  const handle = await small(windows, spawn).open(URL_SMALL, { rect: RECT, titles: TITLES });
  assert.equal(await handle.placed, null); // given up on
  assert.equal(handle.isOpen, false);
  assert.deepEqual(windows.calls, []);
  handle.close();
  assert.deepEqual(windows.calls, []);
});

test("a window whose program Windows won't name is taken by its title", async () => {
  const windows = fakeWindows();
  let opened = null;
  const { spawn } = fakeSpawn({
    onStart: () => {
      opened = windows.add({ pid: 900 }); // no program known for 900
    },
  });
  const handle = await small(windows, spawn).open(URL_SMALL, { rect: RECT, titles: TITLES });
  assert.ok(await handle.placed);
  assert.deepEqual(windows.wins.get(opened).rect, RECT);
  handle.close();
});

test("moved by the browser as it opens, it's put back over the card", async () => {
  const windows = fakeWindows();
  let opened = null;
  const { spawn } = fakeSpawn({
    onStart: () => {
      opened = windows.add();
      // The browser restores its own place just after showing it.
      setTimeout(() => {
        windows.wins.get(opened).rect = { x: 10, y: 10, width: 945, height: 1060 };
      }, 3);
    },
  });
  const handle = await small(windows, spawn).open(URL_SMALL, { rect: RECT, titles: TITLES });
  const placed = await handle.placed;
  assert.deepEqual(placed.rect, RECT);
  assert.equal(windows.calls.filter((c) => c[0] === "place").length, 2);
  handle.close();
});

test("closing it asks only that window to close, once", async () => {
  const windows = fakeWindows();
  const theirs = windows.add({ title: "CAD Protocol - Google Chrome" });
  let opened = null;
  let closedByPerson = 0;
  const { spawn } = fakeSpawn({ onStart: () => (opened = windows.add()) });
  const handle = await small(windows, spawn).open(URL_SMALL, { rect: RECT, titles: TITLES, onClosed: () => (closedByPerson += 1) });
  await handle.placed;
  handle.close();
  handle.close();
  assert.deepEqual(
    windows.calls.filter((c) => c[0] === "close"),
    [["close", opened]],
  );
  assert.ok(windows.wins.has(theirs));
  assert.equal(handle.isOpen, false);
  await wait(30);
  // Closed by the app: not the person's doing.
  assert.equal(closedByPerson, 0);
});

test("closed before its window showed, it's closed as soon as it does", async () => {
  const windows = fakeWindows();
  let opened = null;
  const { spawn } = fakeSpawn({
    onStart: () => {
      setTimeout(() => (opened = windows.add()), 25);
    },
  });
  const handle = await small(windows, spawn).open(URL_SMALL, { rect: RECT, titles: TITLES });
  handle.close(); // cancelled at once
  assert.deepEqual(windows.calls, []);
  assert.equal(await handle.placed, null);
  assert.deepEqual(windows.calls, [["close", opened]]);
  assert.equal(windows.wins.has(opened), false);
});

test("closed by the person, it says so once; clicking the card brings it forward till then", async () => {
  const windows = fakeWindows();
  let opened = null;
  let closedByPerson = 0;
  const { spawn } = fakeSpawn({ onStart: () => (opened = windows.add()) });
  const handle = await small(windows, spawn).open(URL_SMALL, { rect: RECT, titles: TITLES, onClosed: () => (closedByPerson += 1) });
  await handle.placed;
  handle.raise();
  assert.deepEqual(windows.calls[windows.calls.length - 1], ["raise", opened]);
  const before = windows.calls.length;
  windows.wins.delete(opened); // its × pressed
  await wait(30);
  assert.equal(closedByPerson, 1);
  assert.equal(handle.isOpen, false);
  handle.raise();
  handle.close();
  assert.equal(windows.calls.length, before);
});

test("another window taking its place in Windows' list isn't closed for it", async () => {
  const windows = fakeWindows();
  let opened = null;
  let closedByPerson = 0;
  const { spawn } = fakeSpawn({ onStart: () => (opened = windows.add()) });
  const handle = await small(windows, spawn).open(URL_SMALL, { rect: RECT, titles: TITLES, onClosed: () => (closedByPerson += 1) });
  await handle.placed;
  // Closed, and Windows gave its number to another program's window.
  windows.wins.set(opened, { visible: true, cls: "Notepad", title: "notes.txt", pid: 800, rect: { x: 0, y: 0, width: 300, height: 300 } });
  await wait(30);
  assert.equal(closedByPerson, 1);
  handle.close();
  assert.equal(windows.calls.filter((c) => c[0] === "close").length, 0);
  assert.ok(windows.wins.has(opened));
});

test("a browser that won't start means a tab instead", async () => {
  const windows = fakeWindows();
  const { spawn } = fakeSpawn({ fails: true });
  assert.equal(await small(windows, spawn).open(URL_SMALL, { rect: RECT, titles: TITLES }), null);
  const throws = () => {
    throw new Error("EACCES");
  };
  assert.equal(await small(windows, throws).open(URL_SMALL, { rect: RECT, titles: TITLES }), null);
});

test("Windows failing part-way never throws out of a timer", async () => {
  const windows = fakeWindows();
  let opened = null;
  const { spawn } = fakeSpawn({ onStart: () => (opened = windows.add()) });
  windows.backend.place = () => {
    throw new Error("access denied");
  };
  const logged = [];
  const window = createSmallWindow({ backend: windows.backend, spawn, ownPid: OWN_PID, timing: QUICK, log: (line) => logged.push(line) });
  const handle = await window.open(URL_SMALL, { rect: RECT, titles: TITLES });
  assert.equal(await handle.placed, null);
  assert.match(logged.join("\n"), /access denied/);
  // It can still be closed.
  handle.close();
  assert.deepEqual(windows.calls, [["close", opened]]);
});

test("clicking the card brings Internet Identity's window forward too, and nothing else of the browser's", async () => {
  const windows = fakeWindows();
  const theirs = windows.add({ title: "Inbox - Google Chrome" });
  let opened = null;
  const { spawn } = fakeSpawn({ onStart: () => (opened = windows.add()) });
  const handle = await small(windows, spawn).open(URL_SMALL, { rect: RECT, titles: TITLES });
  await handle.placed;
  // Its page opens Internet Identity over it; Internet Identity opens Google's, its own size, across it.
  const identity = windows.add({ title: "Internet Identity", rect: { ...RECT } });
  const google = windows.add({ title: "Sign in - Google Accounts", rect: { x: 710, y: 240, width: 500, height: 600 } });
  // Meanwhile: windows of their own (elsewhere, and maximised), another program's in the same place, and a tooltip.
  const own = windows.add({ title: "New Tab - Google Chrome", rect: { x: 0, y: 0, width: 600, height: 120 } });
  const full = windows.add({ title: "Mail - Google Chrome", maximized: true, rect: { x: -8, y: -8, width: 1936, height: 1056 } });
  windows.exes.set(700, "C:\\Program Files\\Slack\\slack.exe");
  const other = windows.add({ title: "Slack", pid: 700, rect: { ...RECT } });
  const tip = windows.add({ title: "", rect: { ...RECT } });
  // Windows lists windows front to back: the newest first here.
  windows.backend.handles = () => [...windows.wins.keys()].reverse();
  windows.calls.length = 0;
  handle.raise();
  // The small window, then the ones over it, the front one last.
  assert.deepEqual(windows.calls, [
    ["raise", opened],
    ["raise", identity],
    ["raise", google],
  ]);
  for (const untouched of [theirs, own, full, other, tip]) assert.ok(!windows.touched().includes(untouched));
  handle.close();
  assert.deepEqual(windows.calls.at(-1), ["close", opened]);
  assert.ok(windows.wins.has(identity));
});
