"use strict";
// What the desktop app loads, and which pages may open inside its windows.
// Everything here is plain data and pure functions so it can be tested
// without Electron.

/** The live CAD Protocol app on the Internet Computer. */
const APP_URL = "https://cad-dza.caffeine.xyz/";
const APP_ORIGIN = new URL(APP_URL).origin;

/** Internet Identity, the sign-in the app uses. */
const SIGN_IN_HOSTS = ["id.ai", "identity.ic0.app", "identity.internetcomputer.org"];

function parse(url) {
  try {
    return new URL(url);
  } catch {
    return null;
  }
}

function hostIs(host, name) {
  return host === name || host.endsWith(`.${name}`);
}

/** True for a page of the app itself. */
function isAppUrl(url) {
  const u = parse(url);
  return Boolean(u && u.origin === APP_ORIGIN);
}

/** True for Internet Identity's sign-in pages. */
function isSignInUrl(url) {
  const u = parse(url);
  return Boolean(u && u.protocol === "https:" && SIGN_IN_HOSTS.some((h) => hostIs(u.hostname, h)));
}

/**
 * What a window.open from the app becomes:
 * - "signin": Internet Identity, in its own small window that keeps its
 *   link to the app (the sign-in hands its answer back through it);
 * - "report": a page the app made itself (a printed report, a map check),
 *   which the app opens as a blob: address of its own origin;
 * - "external": any other web or mail address, opened in the person's
 *   own browser;
 * - "deny": anything else (file:, javascript:, data: and so on).
 * A sign-in window may itself open another https sign-in window (Internet
 * Identity handing over to a sign-in provider).
 */
function openKind(url, openerKind) {
  if (isSignInUrl(url)) return "signin";
  if (openerKind === "signin") {
    const u = parse(url);
    return u && u.protocol === "https:" ? "signin" : "deny";
  }
  const u = parse(url);
  if (!u) return "deny";
  if (u.protocol === "blob:" && isAppUrl(u.pathname)) return "report";
  if (url === "about:blank") return "report";
  if (isAppUrl(url)) return "external";
  if (isExternalSafe(url)) return "external";
  return "deny";
}

/** Addresses safe to hand to the person's browser or mail program. */
function isExternalSafe(url) {
  const u = parse(url);
  return Boolean(u && (u.protocol === "https:" || u.protocol === "http:" || u.protocol === "mailto:"));
}

/**
 * Whether a window of this kind may move to the address.
 * - The main window stays on the app (or the local "can't reach" page).
 * - A sign-in window may go to any https page: Internet Identity can hand
 *   over to a sign-in provider. It has no access to the desktop features.
 * - A report window stays on the app's own blob: pages.
 */
function canNavigate(kind, url, offlineUrl) {
  if (kind === "main") return isAppUrl(url) || (offlineUrl != null && url.split(/[?#]/)[0] === offlineUrl);
  if (kind === "signin") {
    const u = parse(url);
    return Boolean(u && u.protocol === "https:");
  }
  if (kind === "report") {
    const u = parse(url);
    return url === "about:blank" || Boolean(u && u.protocol === "blob:" && isAppUrl(u.pathname));
  }
  return false;
}

/** Browser permissions the app may use. Everything else is refused. */
const APP_PERMISSIONS = new Set([
  "clipboard-read",
  "clipboard-sanitized-write",
  "fullscreen",
  "pointerLock",
  "keyboardLock",
  "notifications",
  "fileSystem",
  "storage-access",
  "top-level-storage-access",
]);
const SIGN_IN_PERMISSIONS = new Set(["clipboard-sanitized-write", "storage-access", "top-level-storage-access"]);

function permissionAllowed(permission, requestingUrl) {
  if (isAppUrl(requestingUrl)) return APP_PERMISSIONS.has(permission);
  if (isSignInUrl(requestingUrl)) return SIGN_IN_PERMISSIONS.has(permission);
  return false;
}

/**
 * Electron's user agent with its own name and the Electron token removed, so
 * sites see the plain Chrome they already support. The app finds the
 * desktop app through window.cadpDesktop instead.
 */
function plainUserAgent(ua) {
  return ua
    .replace(/\(KHTML, like Gecko\) .*? Chrome\//, "(KHTML, like Gecko) Chrome/")
    .replace(/ Electron\/\S+/, "")
    .trim();
}

module.exports = {
  APP_URL,
  APP_ORIGIN,
  SIGN_IN_HOSTS,
  APP_PERMISSIONS,
  isAppUrl,
  isSignInUrl,
  openKind,
  isExternalSafe,
  canNavigate,
  permissionAllowed,
  plainUserAgent,
};
