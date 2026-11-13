"use strict";
// The small sign-in window (1.13; Leo, 2026-11-13 07:43: "can we open a
// small chrome window ... still using chrome to open the browser desktop
// sign in page, we make it a small browser that open right over the cad
// protocol sign in card, and it open internet identity, you sign in it
// guides you back to the app and then the little chrome window closes").
//
// Signing in still happens in the person's own browser (Google won't sign
// anyone in inside an app's window; browser-sign-in.js). Instead of a tab in
// their big browser window, the page opens in a small window of that
// browser, over the sign-in card, and closes by itself once they're in:
//
// 1. Windows is asked for the default browser. Only Chrome, Edge and Brave
//    have such a window (--app); with any other, or wherever any of this
//    can't be done, the page opens in a tab as before.
// 2. The browser is started with --app=<the page> and nothing else. Its own
//    size and place switches aren't used: a browser that's already running
//    doesn't read them, and one that starts with them sizes every window it
//    opens afterwards by them, the person's own too.
// 3. So the new window is looked for (a window of that browser that wasn't
//    there before, with the page's title) and put over the card with
//    Windows' own calls. No other window is ever touched.
// 4. It's closed when the sign-in is done or cancelled. Closed by the
//    person first, the sign-in is cancelled.
// 5. Clicking the card behind it brings it forward again, with the window
//    its page opened over it (Internet Identity), so neither is lost
//    behind the card.

const path = require("node:path");

/**
 * Its size over the card, in the screen's own units (the card is 1120 x
 * 600): room for the sign-in page, and for Internet Identity, which opens
 * over it at the same size and asks for about 525 x 705 inside.
 */
const WIDTH = 540;
const HEIGHT = 760;

/** The window class of Chrome, Edge and Brave windows (and of this app's own, which are left out by process). */
const WINDOW_CLASS = "Chrome_WidgetWin_1";
/** The browsers with an --app window. */
const BROWSERS = new Set(["chrome.exe", "msedge.exe", "brave.exe"]);

const SW_RESTORE = 9;
const WM_CLOSE = 0x0010;
const SWP_NOSIZE = 0x0001;
const SWP_NOMOVE = 0x0002;
const SWP_NOZORDER = 0x0004;
const SWP_NOACTIVATE = 0x0010;
const SWP_SHOWWINDOW = 0x0040;
const HWND_TOP = 0;
const PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;
/** AssocQueryString: "https" is a protocol, and the program it opens with. */
const ASSOCF_IS_PROTOCOL = 0x1000;
const ASSOCSTR_EXECUTABLE = 2;

const TIMING = Object.freeze({
  /** Looked for this often at first, */
  pollMs: 50,
  /** for this long, */
  fastForMs: 5000,
  /** then this often, */
  slowPollMs: 250,
  /** and given up on after this. */
  findForMs: 30000,
  /** Put in place again if the browser moved it as it opened: after these waits. */
  settleMs: [150, 300, 500],
  /** Then checked this often for the person closing it. */
  watchMs: 500,
});

/** Whether a program is a browser with an --app window. */
function isChromium(exe) {
  return typeof exe === "string" && BROWSERS.has(path.win32.basename(exe).toLowerCase());
}

/** A rectangle of `size` centred on `anchor`, kept inside `workArea` (made smaller only if the screen is). */
function overRect(anchor, workArea, size = { width: WIDTH, height: HEIGHT }) {
  const width = Math.min(size.width, workArea.width);
  const height = Math.min(size.height, workArea.height);
  const within = (value, low, high) => Math.max(low, Math.min(value, high));
  return {
    x: within(Math.round(anchor.x + (anchor.width - width) / 2), workArea.x, workArea.x + workArea.width - width),
    y: within(Math.round(anchor.y + (anchor.height - height) / 2), workArea.y, workArea.y + workArea.height - height),
    width,
    height,
  };
}

/** Whether two windows share any of the screen. */
function overlaps(a, b) {
  return Boolean(a && b) && a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

/** Within a few pixels: a browser may round, or keep an edge of its own. */
function sameRect(a, b, slack = 2) {
  return Boolean(a && b) && ["x", "y", "width", "height"].every((k) => Math.abs(a[k] - b[k]) <= slack);
}

// ── The default browser, from the registry (if Windows' own call won't say) ──

/** A value in reg.exe's answer: the text after REG_SZ (the value's own name is in the person's language). */
function regValue(output) {
  const m = /^\s+.*?\s+REG_(?:EXPAND_)?SZ\s+(.*?)\s*$/m.exec(String(output || ""));
  return m && m[1] ? m[1] : null;
}

/** The program in an "open" command: the quoted path, or up to .exe. */
function exeFromCommand(command, env = process.env) {
  const text = String(command || "").replace(/%([^%]+)%/g, (all, name) => env[name] || all);
  const quoted = /^\s*"([^"]+)"/.exec(text);
  if (quoted) return quoted[1];
  const bare = /^\s*(.+?\.exe)(?:\s|$)/i.exec(text);
  return bare ? bare[1] : null;
}

/** The https program by the registry: the person's choice, then its open command. `query(args)` runs reg.exe. */
function registryBrowser(query, env = process.env) {
  const base = "HKCU\\Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\https\\";
  const progId =
    regValue(query(["query", `${base}UserChoiceLatest\\ProgId`, "/v", "ProgId"])) ||
    regValue(query(["query", `${base}UserChoice`, "/v", "ProgId"]));
  if (!progId || !/^[\w.-]{1,120}$/.test(progId)) return null;
  const command = regValue(query(["query", `HKCR\\${progId}\\shell\\open\\command`, "/ve"]));
  return command ? exeFromCommand(command, env) : null;
}

// ── Windows' calls ───────────────────────────────────────────────────────────

const unsupported = Object.freeze({ supported: false });
let made = null;

/** The Windows calls, through koffi (made once). Anywhere else, or if koffi can't load, there's no small window. */
function windowsBackend(log = () => {}) {
  if (process.platform !== "win32") return unsupported;
  if (made) return made;
  try {
    const koffi = require("koffi");
    const { execFileSync } = require("node:child_process");
    const user32 = koffi.load("user32.dll");
    const kernel32 = koffi.load("kernel32.dll");
    const shlwapi = koffi.load("shlwapi.dll");
    const Rect = koffi.struct("CadpSmallRect", { left: "int32", top: "int32", right: "int32", bottom: "int32" });
    const EnumProc = koffi.proto("__stdcall", "CadpSmallEnumProc", "int", ["uintptr_t", "intptr_t"]);
    const call = (lib, name, result, args) => lib.func("__stdcall", name, result, args);
    const enumWindows = call(user32, "EnumWindows", "int", [koffi.pointer(EnumProc), "intptr_t"]);
    const isWindow = call(user32, "IsWindow", "int", ["uintptr_t"]);
    const isWindowVisible = call(user32, "IsWindowVisible", "int", ["uintptr_t"]);
    const isIconic = call(user32, "IsIconic", "int", ["uintptr_t"]);
    const isZoomed = call(user32, "IsZoomed", "int", ["uintptr_t"]);
    const showWindow = call(user32, "ShowWindow", "int", ["uintptr_t", "int"]);
    const getClassName = call(user32, "GetClassNameW", "int", ["uintptr_t", "uint16_t *", "int"]);
    const getWindowText = call(user32, "GetWindowTextW", "int", ["uintptr_t", "uint16_t *", "int"]);
    const getProcessId = call(user32, "GetWindowThreadProcessId", "uint32", ["uintptr_t", koffi.out(koffi.pointer("uint32"))]);
    const getWindowRect = call(user32, "GetWindowRect", "int", ["uintptr_t", koffi.out(koffi.pointer(Rect))]);
    const setWindowPos = call(user32, "SetWindowPos", "int", ["uintptr_t", "uintptr_t", "int", "int", "int", "int", "uint32"]);
    const setForeground = call(user32, "SetForegroundWindow", "int", ["uintptr_t"]);
    const postMessage = call(user32, "PostMessageW", "int", ["uintptr_t", "uint32", "uintptr_t", "intptr_t"]);
    const openProcess = call(kernel32, "OpenProcess", "uintptr_t", ["uint32", "int", "uint32"]);
    const closeHandle = call(kernel32, "CloseHandle", "int", ["uintptr_t"]);
    const imageName = call(kernel32, "QueryFullProcessImageNameW", "int", [
      "uintptr_t",
      "uint32",
      "uint16_t *",
      koffi.inout(koffi.pointer("uint32")),
    ]);
    const assocQuery = call(shlwapi, "AssocQueryStringW", "int32", [
      "uint32",
      "uint32",
      "str16",
      "str16",
      "uint16_t *",
      koffi.inout(koffi.pointer("uint32")),
    ]);

    /** The text in a buffer, up to its end mark. */
    const textOf = (buffer, length) => {
      let end = Math.max(0, Math.min(length, buffer.length));
      const zero = buffer.indexOf(0);
      if (zero >= 0 && zero < end) end = zero;
      return Buffer.from(buffer.buffer, buffer.byteOffset, end * 2).toString("utf16le");
    };
    const reg = path.win32.join(process.env.SystemRoot || "C:\\Windows", "System32", "reg.exe");
    const query = (args) => {
      try {
        return execFileSync(reg, args, { encoding: "utf8", windowsHide: true, timeout: 4000, stdio: ["ignore", "pipe", "ignore"] });
      } catch {
        return "";
      }
    };

    made = {
      supported: true,
      /** The program https links open with, and how it was found; null if Windows won't say. */
      defaultBrowser() {
        try {
          const buffer = new Uint16Array(1024);
          const size = [buffer.length];
          if (assocQuery(ASSOCF_IS_PROTOCOL, ASSOCSTR_EXECUTABLE, "https", null, buffer, size) === 0) {
            const exe = textOf(buffer, buffer.length);
            if (exe) return { exe, how: "windows" };
          }
        } catch (err) {
          log(`small window: asking Windows for the browser failed: ${err && err.message ? err.message : err}`);
        }
        const exe = registryBrowser(query);
        return exe ? { exe, how: "registry" } : null;
      },
      /** Every top-level window. */
      handles() {
        const list = [];
        enumWindows((hwnd) => {
          list.push(hwnd);
          return 1;
        }, 0);
        return list;
      },
      info(hwnd) {
        const buffer = new Uint16Array(512);
        const cls = textOf(buffer, getClassName(hwnd, buffer, buffer.length));
        buffer.fill(0);
        const title = textOf(buffer, getWindowText(hwnd, buffer, buffer.length));
        const pid = [0];
        getProcessId(hwnd, pid);
        return { visible: Boolean(isWindowVisible(hwnd)), maximized: Boolean(isZoomed(hwnd)), cls, title, pid: pid[0] };
      },
      /** The program a process runs, or null if Windows won't say. */
      exeOf(pid) {
        const handle = openProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if (!handle) return null;
        try {
          const buffer = new Uint16Array(1024);
          const size = [buffer.length];
          return imageName(handle, 0, buffer, size) ? textOf(buffer, size[0]) || null : null;
        } finally {
          closeHandle(handle);
        }
      },
      exists(hwnd) {
        return Boolean(isWindow(hwnd));
      },
      rect(hwnd) {
        const r = {};
        if (!getWindowRect(hwnd, r)) return null;
        return { x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top };
      },
      /** Its size and place only; maximised or minimised, it's put back to a window first. */
      place(hwnd, r) {
        if (isIconic(hwnd) || isZoomed(hwnd)) showWindow(hwnd, SW_RESTORE);
        return Boolean(setWindowPos(hwnd, HWND_TOP, r.x, r.y, r.width, r.height, SWP_NOZORDER | SWP_NOACTIVATE));
      },
      /** In front of the card, as far as Windows allows (minimised, it's a window again first). */
      raise(hwnd) {
        if (isIconic(hwnd)) showWindow(hwnd, SW_RESTORE);
        setWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        setForeground(hwnd);
      },
      /** Asked to close, as its own × does. */
      close(hwnd) {
        return Boolean(postMessage(hwnd, WM_CLOSE, 0, 0));
      },
    };
    return made;
  } catch (err) {
    log(`small window unavailable: ${err && err.message ? err.message : err}`);
    return unsupported;
  }
}

// ── The small window ─────────────────────────────────────────────────────────

/**
 * Opens pages in a small window of the person's browser. `backend` is
 * Windows' calls (windowsBackend), `spawn` is node:child_process's, and
 * `ownPid` is this app's process, whose own windows are never taken for the
 * browser's.
 */
function createSmallWindow({ backend, spawn, ownPid = process.pid, log = () => {}, timing = {} }) {
  const t = { ...TIMING, ...timing };
  const told = (what, err) => log(`small window: ${what}: ${err && err.message ? err.message : err}`);

  /** Starts the browser; true once it's running. */
  function launch(exe, args) {
    return new Promise((resolve) => {
      let child;
      try {
        child = spawn(exe, args, { detached: true, stdio: "ignore" });
      } catch (err) {
        told("the browser couldn't start", err);
        resolve(false);
        return;
      }
      child.on("error", (err) => {
        told("the browser couldn't start", err);
        resolve(false);
      });
      child.once("spawn", () => {
        if (typeof child.unref === "function") child.unref();
        resolve(true);
      });
    });
  }

  /** The browser a small window opens in, or null when the person's has none. */
  function browserExe() {
    if (!backend.supported) return null;
    try {
      const found = backend.defaultBrowser();
      return found && isChromium(found.exe) ? found.exe : null;
    } catch (err) {
      told("the browser couldn't be found", err);
      return null;
    }
  }

  return {
    browser: browserExe,

    /**
     * Opens `url` in a small window at `rect` (screen pixels). `titles`:
     * what the window's title has in it (any of them). `onClosed` when the
     * person closes it. Resolves with the window's handle ({ placed, close,
     * raise, isOpen }), or null when there's no small window to be had (the
     * caller opens a tab instead).
     */
    async open(url, { rect, titles, onClosed = () => {}, exe = null, extraArgs = [] }) {
      if (!backend.supported || !/^https?:\/\//.test(String(url))) return null;
      const browser = exe || browserExe();
      if (!browser) return null;
      let before;
      try {
        before = new Set(backend.handles().map(String));
      } catch (err) {
        told("the open windows couldn't be listed", err);
        return null;
      }
      if (!(await launch(browser, [`--app=${url}`, ...extraArgs]))) return null;

      const wants = titles.map((title) => String(title).toLowerCase());
      const program = browser.toLowerCase();
      const startedAt = Date.now();
      /** "looking", "open" (hwnd set) or "done". */
      let state = "looking";
      let hwnd = null;
      let pid = 0;
      let title = "";
      let closing = false;
      let timer = null;
      let tellPlaced;
      const placed = new Promise((resolve) => {
        tellPlaced = resolve;
      });

      /** Never throws: a timer's error would stop the app. */
      const later = (ms, step) => {
        timer = setTimeout(() => {
          timer = null;
          try {
            step();
          } catch (err) {
            told("stopped", err);
            finish();
          }
        }, ms);
        if (typeof timer.unref === "function") timer.unref();
      };
      const finish = () => {
        state = "done";
        if (timer) clearTimeout(timer);
        timer = null;
        tellPlaced(null);
      };
      /** The window found, if it's still that browser's. */
      const stillThere = () => hwnd !== null && backend.exists(hwnd) && backend.info(hwnd).pid === pid;
      /** A new window of the browser with the page's title; never one of this app's. */
      const theWindow = () => {
        for (const handle of backend.handles()) {
          if (before.has(String(handle))) continue;
          const info = backend.info(handle);
          if (!info.visible || info.cls !== WINDOW_CLASS || info.pid === ownPid) continue;
          const text = info.title.toLowerCase();
          if (!wants.some((want) => text.includes(want))) continue;
          const from = backend.exeOf(info.pid);
          if (from && from.toLowerCase() !== program) continue;
          return { handle, info };
        }
        return null;
      };

      const look = () => {
        if (state !== "looking") return;
        const found = theWindow();
        if (found) {
          hwnd = found.handle;
          pid = found.info.pid;
          title = found.info.title;
          state = "open";
          if (closing) {
            shut();
            return;
          }
          backend.place(hwnd, rect);
          backend.raise(hwnd);
          settle(0);
          return;
        }
        const waited = Date.now() - startedAt;
        if (waited > t.findForMs) {
          log("small window: the browser's window wasn't found; it's left as it opened");
          finish();
          return;
        }
        later(waited < t.fastForMs ? t.pollMs : t.slowPollMs, look);
      };
      /** A browser may put its window where it last was just after it shows: put back, a few times at most. */
      const settle = (step) => {
        if (state !== "open") return;
        if (!stillThere()) {
          gone();
          return;
        }
        if (step >= t.settleMs.length) {
          tellPlaced({ rect: backend.rect(hwnd), title });
          watch();
          return;
        }
        later(t.settleMs[step], () => {
          if (state !== "open") return;
          if (stillThere() && !sameRect(backend.rect(hwnd), rect)) backend.place(hwnd, rect);
          settle(step + 1);
        });
      };
      const watch = () => {
        if (state !== "open") return;
        if (!stillThere()) {
          gone();
          return;
        }
        later(t.watchMs, watch);
      };
      /** Closed by the person. */
      const gone = () => {
        hwnd = null;
        finish();
        if (closing) return;
        try {
          onClosed();
        } catch (err) {
          told("after it closed", err);
        }
      };
      const shut = () => {
        try {
          if (stillThere()) backend.close(hwnd);
        } catch (err) {
          told("it couldn't be closed", err);
        }
        hwnd = null;
        finish();
      };

      later(0, look);
      return {
        /** Resolves with { rect, title } once it's in place, or null if it never was. */
        placed,
        /** Closes it; one that hasn't shown yet is closed when it does. */
        close() {
          closing = true;
          if (hwnd !== null) shut();
        },
        /**
         * In front of the card again (the card was clicked), and then the
         * windows opened over it since (Internet Identity's, and Google's
         * from there: newer windows of the same browser that overlap it,
         * not maximised), which would otherwise be left behind it. Those
         * are only ever brought forward, in the order they were in.
         */
        raise() {
          if (state !== "open" || closing) return;
          try {
            if (!stillThere()) return;
            backend.raise(hwnd);
            const at = backend.rect(hwnd);
            // Windows lists them front to back: the backmost first, so they keep their order.
            for (const handle of backend.handles().reverse()) {
              if (handle === hwnd || before.has(String(handle))) continue;
              const info = backend.info(handle);
              if (!info.visible || info.maximized || info.cls !== WINDOW_CLASS || info.pid !== pid || !info.title) continue;
              if (overlaps(backend.rect(handle), at)) backend.raise(handle);
            }
          } catch (err) {
            told("it couldn't be brought forward", err);
          }
        },
        get isOpen() {
          return state === "open";
        },
      };
    },
  };
}

module.exports = {
  WIDTH,
  HEIGHT,
  WINDOW_CLASS,
  TIMING,
  isChromium,
  overRect,
  sameRect,
  overlaps,
  regValue,
  exeFromCommand,
  registryBrowser,
  windowsBackend,
  createSmallWindow,
};
