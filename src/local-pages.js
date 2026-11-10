"use strict";
// The app's own pages at cadp://app/… (1.8.2): the start-up and sign-out
// card (update.html with splash.jpg) and the offline page.
//
// The installed app turns off file://'s extra rights (the
// grantFileProtocolExtraPrivileges fuse in package.json), and with them off
// a window can't show a page straight from the app's files: the card came
// up as an empty dark box (Leo, 2026-11-09 16:50: "i get a blank loading
// card when starting the app"; on Windows the packaged app's card stayed
// empty while `npm start`'s drew). Electron's way is a scheme of the app's
// own, served here from the app's files, and only the files listed.

const path = require("node:path");

const SCHEME = "cadp";
const HOST = "app";
/** The only files served, with their types. */
const FILES = new Map([
  ["update.html", "text/html; charset=utf-8"],
  ["offline.html", "text/html; charset=utf-8"],
  ["signin-card.html", "text/html; charset=utf-8"],
  ["splash.jpg", "image/jpeg"],
]);

/** cadp://app/<name>, with `query` as its search. */
function pageUrl(name, query = {}) {
  const url = new URL(`${SCHEME}://${HOST}/${name}`);
  for (const [key, value] of Object.entries(query)) url.searchParams.set(key, String(value));
  return url.href;
}

/** The file a cadp://app/ address names in `dir`, and its type; null for anything not listed. */
function fileFor(address, dir) {
  let url;
  try {
    url = new URL(address);
  } catch {
    return null;
  }
  if (url.protocol !== `${SCHEME}:` || url.host !== HOST) return null;
  const name = url.pathname.replace(/^\//, "");
  const type = FILES.get(name);
  if (!type) return null;
  return { file: path.join(dir, name), type };
}

/** Before the app's ready: cadp:// is a standard, secure scheme (so 'self' and relative addresses work). */
function registerScheme(protocol) {
  protocol.registerSchemesAsPrivileged([{ scheme: SCHEME, privileges: { standard: true, secure: true } }]);
}

/** Once it's ready: cadp://app/ answered from `dir`. */
function serveLocalPages({ protocol, dir, readFile, log = () => {} }) {
  protocol.handle(SCHEME, async (request) => {
    const hit = fileFor(request.url, dir);
    if (!hit) return new Response("Not found", { status: 404 });
    try {
      const body = await readFile(hit.file);
      return new Response(body, { headers: { "content-type": hit.type } });
    } catch (err) {
      log(`couldn't read ${hit.file}: ${err && err.message ? err.message : err}`);
      return new Response("Not found", { status: 404 });
    }
  });
}

module.exports = { SCHEME, HOST, FILES, pageUrl, fileFor, registerScheme, serveLocalPages };
