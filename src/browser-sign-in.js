"use strict";
// Sign in with your browser.
//
// Google refuses to sign anyone in inside an app's own window, so the desktop
// app signs in through the person's own browser, the way native apps are
// meant to (RFC 8252, "OAuth 2.0 for Native Apps", with a loopback address):
//
// 1. The app's page makes a session key that never leaves it and asks for
//    a browser sign-in for its public half.
// 2. This opens a one-use listener on 127.0.0.1 (this computer only) at a
//    free port, and opens the app's /desktop-sign-in page in the person's
//    browser with the key, the port and a random state.
// 3. Signed in there as usual, they press Continue. The page delegates a
//    session for their account to the key and goes to
//    http://127.0.0.1:<port>/done with it and the state.
// 4. If the state matches, the listener hands the result to the app's page,
//    shows "you can close this tab", closes and brings the app to the front.
//
// The result is no use without the desktop app's key, which never leaves
// it; the state stops any other page's answer being taken for this one.

const crypto = require("node:crypto");
const http = require("node:http");

const TIMEOUT_MS = 10 * 60 * 1000;
/** Big enough for a delegation chain in the address. */
const MAX_HEADER_BYTES = 256 * 1024;

function isPublicKeyHex(value) {
  return typeof value === "string" && value.length % 2 === 0 && /^[0-9a-f]{40,400}$/.test(value);
}

/** The page in the person's browser. Everything after # stays in the browser. */
function signInPageUrl(appUrl, key, port, state) {
  const url = new URL("/desktop-sign-in", appUrl);
  url.hash = new URLSearchParams({ k: key, p: String(port), s: state }).toString();
  return url.toString();
}

function sameText(a, b) {
  const x = Buffer.from(String(a));
  const y = Buffer.from(String(b));
  return x.length === y.length && crypto.timingSafeEqual(x, y);
}

/**
 * Reads the browser's answer. Returns { result } for a good one, or
 * { status, message } to answer with.
 */
function readAnswer(requestUrl, method, state) {
  let url;
  try {
    url = new URL(requestUrl, "http://127.0.0.1");
  } catch {
    return { status: 400, message: "That address can't be read." };
  }
  if (method !== "GET" || url.pathname !== "/done") return { status: 404, message: "Nothing here." };
  if (!sameText(url.searchParams.get("s") || "", state)) {
    return { status: 400, message: "This sign-in isn't the one the desktop app is waiting for. Start again from the desktop app." };
  }
  const principal = url.searchParams.get("a") || "";
  if (!/^[a-z0-9]{5}(-[a-z0-9]{1,5}){1,12}$/.test(principal)) {
    return { status: 400, message: "The account in this sign-in can't be read." };
  }
  let chain;
  try {
    chain = Buffer.from(url.searchParams.get("d") || "", "base64url").toString("utf8");
    const parsed = JSON.parse(chain);
    if (!parsed || typeof parsed.publicKey !== "string" || !Array.isArray(parsed.delegations) || parsed.delegations.length === 0) {
      throw new Error("not a chain");
    }
  } catch {
    return { status: 400, message: "The sign-in in this address can't be read." };
  }
  return { result: { chain, principal } };
}

function page(title, message) {
  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${esc(title)}</title><style>:root{color-scheme:dark}body{margin:0;min-height:100vh;display:grid;place-items:center;background:#000;color:#e8e8ea;font:15px/1.5 "Segoe UI",system-ui,sans-serif}main{max-width:28rem;padding:0 1.5rem}h1{font-size:1.25rem;font-weight:600;margin:0 0 .5rem}p{margin:0;color:#a9a9b2}</style></head><body><main><h1>${esc(title)}</h1><p>${esc(message)}</p></main></body></html>`;
}

/**
 * One sign-in at a time. `openExternal(url)` opens the person's browser;
 * `focusApp()` brings the desktop app to the front.
 */
function createBrowserSignIn({ appUrl, openExternal, focusApp = () => {}, log = () => {}, timeoutMs = TIMEOUT_MS }) {
  let pending = null;

  function finish(error, result) {
    const current = pending;
    if (!current) return;
    pending = null;
    clearTimeout(current.timer);
    current.server.close();
    if (typeof current.server.closeAllConnections === "function") {
      // After the answer page has gone out.
      setTimeout(() => current.server.closeAllConnections(), 200).unref();
    }
    if (error) current.reject(error);
    else current.resolve(result);
  }

  return {
    /** Resolves with { chain, principal } from the browser, or rejects when cancelled or after the time limit. */
    start(publicKeyHex) {
      if (!isPublicKeyHex(publicKeyHex)) return Promise.reject(new Error("That key can't be used."));
      if (pending) finish(new Error("cancelled: a new sign-in started"));
      const state = crypto.randomBytes(32).toString("hex");
      return new Promise((resolve, reject) => {
        const server = http.createServer({ maxHeaderSize: MAX_HEADER_BYTES }, (req, res) => {
          res.setHeader("Content-Type", "text/html; charset=utf-8");
          res.setHeader("Cache-Control", "no-store");
          res.setHeader("Referrer-Policy", "no-referrer");
          res.setHeader("Connection", "close");
          if (!pending || pending.server !== server) {
            res.statusCode = 410;
            res.end(page("CAD Protocol", "This sign-in has already finished."));
            return;
          }
          const answer = readAnswer(req.url || "", req.method, state);
          if (!answer.result) {
            res.statusCode = answer.status;
            res.end(page("CAD Protocol", answer.message));
            return;
          }
          res.statusCode = 200;
          res.end(page("You're signed in", "Go back to CAD Protocol for Windows. You can close this tab."));
          log("signed in through the browser");
          focusApp();
          finish(null, answer.result);
        });
        const timer = setTimeout(() => finish(new Error("timed out")), timeoutMs);
        timer.unref();
        pending = { server, timer, resolve, reject };
        server.on("error", (err) => {
          log(`sign-in listener: ${err.message}`);
          finish(new Error("The sign-in listener couldn't start."));
        });
        server.listen(0, "127.0.0.1", () => {
          if (!pending || pending.server !== server) return;
          const { port } = server.address();
          const url = signInPageUrl(appUrl, publicKeyHex, port, state);
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
    /** For tests: the server of the sign-in in progress. */
    get serverForTests() {
      return pending ? pending.server : null;
    },
  };
}

module.exports = { createBrowserSignIn, isPublicKeyHex, signInPageUrl, readAnswer, TIMEOUT_MS };
