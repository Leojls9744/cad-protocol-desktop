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

const crypto = require("node:crypto");
const http = require("node:http");

const TIMEOUT_MS = 10 * 60 * 1000;
const MAX_HEADER_BYTES = 64 * 1024;
const HANDOFF_INFO = "cadp-drive-handoff-v1";

function isClientId(value) {
  return typeof value === "string" && /^\d+-[a-z0-9]+\.apps\.googleusercontent\.com$/.test(value);
}

/** The page in the person's browser. Everything after # stays in the browser. */
function drivePageUrl(appUrl, key, port, state, clientId) {
  const url = new URL("/desktop-drive", appUrl);
  url.hash = new URLSearchParams({ k: key, p: String(port), s: state, c: clientId }).toString();
  return url.toString();
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
  return {
    accessToken: got.accessToken,
    expiresIn: Number.isFinite(expiresIn) && expiresIn > 0 ? Math.min(expiresIn, 3600) : 3600,
    email: typeof got.email === "string" ? got.email.slice(0, 300) : "",
  };
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
    /** Resolves with { accessToken, expiresIn, email }, or rejects when cancelled or after the time limit. */
    connect(clientId) {
      if (!isClientId(clientId)) return Promise.reject(new Error("That isn't a Google OAuth client id."));
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
          res.end(page("Google Drive is connected", "Go back to CAD Protocol for Windows. You can close this tab."));
          log("Google Drive connected through the browser");
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
          const url = drivePageUrl(appUrl, publicKey, port, state, clientId);
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

module.exports = { createBrowserDrive, drivePageUrl, readDriveAnswer, openSealed, isClientId, HANDOFF_INFO, TIMEOUT_MS };
