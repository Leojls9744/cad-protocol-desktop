"use strict";
// Google Drive through the person's own browser (1.9; web app v782).
//
// Google won't sign in inside the app's own window, so, as for signing in
// (browser-sign-in.js, RFC 8252 with a loopback address):
//
// 1. The app's page asks for Drive with its OAuth client id.
// 2. This makes a one-use ECDH P-256 key pair (the private half never
//    leaves this process), opens a one-use listener on 127.0.0.1 at a free
//    port, and opens the app's /desktop-drive page in the person's browser
//    with the public half, the port, a random state and the client id.
// 3. There Google is asked for Drive access; the page seals the token to
//    this key (ECDH, HKDF-SHA-256 salted with the state, AES-256-GCM) and
//    goes to http://127.0.0.1:<port>/drive with it.
// 4. If the state matches and it opens with this key, the listener hands
//    { accessToken, expiresIn, email } to the app's page, says "you can
//    close this tab", closes and brings the app to the front.
//
// What passes through the browser's address bar is no use without this
// key; the state stops any other page's answer being taken for this one.
//
// 1.10.2 (Leo, 2026-11-10 16:50: "when i click on a folder and hit select
// nothing happens"): Google's file picker can't show in the app's window
// either (it asks to sign in to Google there, and opened in a tab of its
// own it has no way back), so picking goes the same way. connect() with
// `pick` ({ kind: "file" | "folder", apiKey, title, startIn }) has the
// page show the picker after the token, and the files or folder chosen
// come back sealed with it as `picked` (an empty list: closed).

const crypto = require("node:crypto");
const http = require("node:http");

const TIMEOUT_MS = 10 * 60 * 1000;
const MAX_HEADER_BYTES = 64 * 1024;
const HANDOFF_INFO = "cadp-drive-handoff-v1";

const MAX_PICKED = 50;
const DRIVE_ID = /^[A-Za-z0-9_-]{1,200}$/;

function isClientId(value) {
  return typeof value === "string" && /^\d+-[a-z0-9]+\.apps\.googleusercontent\.com$/.test(value);
}

/** What to pick (1.10.2), checked; null for none, or throws. */
function pickRequest(pick) {
  if (pick === undefined || pick === null) return null;
  if (typeof pick !== "object") throw new Error("That isn't something to pick.");
  if (pick.kind !== "file" && pick.kind !== "folder") throw new Error("Pick files or a folder.");
  if (typeof pick.apiKey !== "string" || !/^AIza[0-9A-Za-z_-]{20,80}$/.test(pick.apiKey)) throw new Error("That isn't a Google API key.");
  const title = typeof pick.title === "string" ? pick.title.slice(0, 120) : "";
  const startIn = typeof pick.startIn === "string" && DRIVE_ID.test(pick.startIn) ? pick.startIn : "";
  return { kind: pick.kind, apiKey: pick.apiKey, title, startIn };
}

/** The page in the person's browser. Everything after # stays in the browser. */
function drivePageUrl(appUrl, key, port, state, clientId, pick = null) {
  const url = new URL("/desktop-drive", appUrl);
  const q = new URLSearchParams({ k: key, p: String(port), s: state, c: clientId });
  if (pick) {
    q.set("m", pick.kind);
    q.set("a", pick.apiKey);
    if (pick.title) q.set("t", pick.title);
    if (pick.startIn) q.set("f", pick.startIn);
  }
  url.hash = q.toString();
  return url.toString();
}

/** The files or folder chosen, as far as they can be kept; undefined when none was asked. */
function pickedFrom(list) {
  if (!Array.isArray(list)) return undefined;
  const short = (v, n) => (typeof v === "string" ? v.slice(0, n) : "");
  return list
    .filter((item) => item && typeof item.id === "string" && DRIVE_ID.test(item.id))
    .slice(0, MAX_PICKED)
    .map((item) => ({
      id: item.id,
      name: short(item.name, 300),
      mimeType: short(item.mimeType, 200),
      parentId: typeof item.parentId === "string" && DRIVE_ID.test(item.parentId) ? item.parentId : null,
      sizeBytes: Number.isFinite(item.sizeBytes) && item.sizeBytes >= 0 ? item.sizeBytes : null,
    }));
}

function sameText(a, b) {
  const x = Buffer.from(String(a));
  const y = Buffer.from(String(b));
  return x.length === y.length && crypto.timingSafeEqual(x, y);
}

/** Opens what the page sealed: { accessToken, expiresIn, email }, or throws. */
function openSealed(ecdh, state, sealed) {
  const theirs = Buffer.from(sealed.e, "base64url");
  const shared = ecdh.computeSecret(theirs);
  const key = Buffer.from(crypto.hkdfSync("sha256", shared, Buffer.from(state), Buffer.from(HANDOFF_INFO), 32));
  const iv = Buffer.from(sealed.n, "base64url");
  const all = Buffer.from(sealed.c, "base64url");
  if (iv.length !== 12 || all.length < 17) throw new Error("too short");
  const decipher = crypto.createDecipheriv("aes-256-gcm", key, iv);
  decipher.setAuthTag(all.subarray(all.length - 16));
  const plain = Buffer.concat([decipher.update(all.subarray(0, all.length - 16)), decipher.final()]);
  const got = JSON.parse(plain.toString("utf8"));
  if (!got || typeof got.accessToken !== "string" || !got.accessToken || got.accessToken.length > 4096) throw new Error("no token");
  const expiresIn = Number(got.expiresIn);
  const out = {
    accessToken: got.accessToken,
    expiresIn: Number.isFinite(expiresIn) && expiresIn > 0 ? Math.min(expiresIn, 3600) : 3600,
    email: typeof got.email === "string" ? got.email.slice(0, 300) : "",
  };
  const picked = pickedFrom(got.picked);
  if (picked) out.picked = picked;
  return out;
}

/** Reads the browser's answer: { result } for a good one, or { status, message }. */
function readDriveAnswer(requestUrl, method, state, ecdh) {
  let url;
  try {
    url = new URL(requestUrl, "http://127.0.0.1");
  } catch {
    return { status: 400, message: "That address can't be read." };
  }
  if (method !== "GET" || url.pathname !== "/drive") return { status: 404, message: "Nothing here." };
  if (!sameText(url.searchParams.get("s") || "", state)) {
    return { status: 400, message: "This isn't the Google Drive connection the desktop app is waiting for. Start again from the desktop app." };
  }
  const sealed = { e: url.searchParams.get("e") || "", n: url.searchParams.get("n") || "", c: url.searchParams.get("c") || "" };
  try {
    return { result: openSealed(ecdh, state, sealed) };
  } catch {
    return { status: 400, message: "The Google Drive connection in this address can't be read." };
  }
}

function page(title, message) {
  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${esc(title)}</title><style>:root{color-scheme:dark}body{margin:0;min-height:100vh;display:grid;place-items:center;background:#000;color:#e8e8ea;font:15px/1.5 "Segoe UI",system-ui,sans-serif}main{max-width:28rem;padding:0 1.5rem}h1{font-size:1.25rem;font-weight:600;margin:0 0 .5rem}p{margin:0;color:#a9a9b2}</style></head><body><main><h1>${esc(title)}</h1><p>${esc(message)}</p></main></body></html>`;
}

/** One connection at a time. */
function createBrowserDrive({ appUrl, openExternal, focusApp = () => {}, log = () => {}, timeoutMs = TIMEOUT_MS }) {
  let pending = null;

  function finish(error, result) {
    const current = pending;
    if (!current) return;
    pending = null;
    clearTimeout(current.timer);
    current.server.close();
    if (typeof current.server.closeAllConnections === "function") {
      setTimeout(() => current.server.closeAllConnections(), 200).unref();
    }
    if (error) current.reject(error);
    else current.resolve(result);
  }

  return {
    /**
     * Resolves with { accessToken, expiresIn, email } (and `picked` when
     * `options.pick` asked for it), or rejects when cancelled or after the
     * time limit.
     */
    connect(clientId, options = {}) {
      if (!isClientId(clientId)) return Promise.reject(new Error("That isn't a Google OAuth client id."));
      let pick;
      try {
        pick = pickRequest(options && options.pick);
      } catch (err) {
        return Promise.reject(err);
      }
      if (pending) finish(new Error("cancelled: a new connection started"));
      const state = crypto.randomBytes(32).toString("hex");
      const ecdh = crypto.createECDH("prime256v1");
      const publicKey = ecdh.generateKeys().toString("base64url");
      return new Promise((resolve, reject) => {
        const server = http.createServer({ maxHeaderSize: MAX_HEADER_BYTES }, (req, res) => {
          res.setHeader("Content-Type", "text/html; charset=utf-8");
          res.setHeader("Cache-Control", "no-store");
          res.setHeader("Referrer-Policy", "no-referrer");
          res.setHeader("Connection", "close");
          if (!pending || pending.server !== server) {
            res.statusCode = 410;
            res.end(page("CAD Protocol", "This connection has already finished."));
            return;
          }
          const answer = readDriveAnswer(req.url || "", req.method, state, ecdh);
          if (!answer.result) {
            res.statusCode = answer.status;
            res.end(page("CAD Protocol", answer.message));
            return;
          }
          res.statusCode = 200;
          if (pick) {
            const chose = answer.result.picked && answer.result.picked.length;
            res.end(
              page(
                chose ? "Sent to CAD Protocol" : "Nothing chosen",
                chose
                  ? "Go back to CAD Protocol for Windows: what you chose is there. You can close this tab."
                  : "Nothing was chosen on Google Drive. Go back to CAD Protocol for Windows. You can close this tab.",
              ),
            );
            log("Google Drive picked through the browser");
          } else {
            res.end(page("Google Drive is connected", "Go back to CAD Protocol for Windows. You can close this tab."));
            log("Google Drive connected through the browser");
          }
          focusApp();
          finish(null, answer.result);
        });
        const timer = setTimeout(() => finish(new Error("timed out")), timeoutMs);
        timer.unref();
        pending = { server, timer, resolve, reject };
        server.on("error", (err) => {
          log(`drive listener: ${err.message}`);
          finish(new Error("The Google Drive listener couldn't start."));
        });
        server.listen(0, "127.0.0.1", () => {
          if (!pending || pending.server !== server) return;
          const { port } = server.address();
          const url = drivePageUrl(appUrl, publicKey, port, state, clientId, pick);
          Promise.resolve(openExternal(url)).catch((err) => {
            log(`couldn't open the browser: ${err && err.message ? err.message : err}`);
            finish(new Error("Your browser couldn't be opened."));
          });
        });
      });
    },
    cancel() {
      finish(new Error("cancelled"));
    },
    get serverForTests() {
      return pending ? pending.server : null;
    },
  };
}

module.exports = { createBrowserDrive, drivePageUrl, readDriveAnswer, openSealed, isClientId, pickRequest, pickedFrom, HANDOFF_INFO, TIMEOUT_MS, MAX_PICKED };
