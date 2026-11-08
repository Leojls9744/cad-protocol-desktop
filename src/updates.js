"use strict";
// Whether a newer installer is out (1.2 on): the newest release on GitHub,
// checked at most every 6 hours, never more than that and never anything sent
// but the request itself. The app's Settings shows it, and says it once per
// new version.

const LATEST_URL = "https://api.github.com/repos/leojls9744/cad-protocol-desktop/releases/latest";
const RELEASES_URL = "https://github.com/leojls9744/cad-protocol-desktop/releases/latest";
const EVERY_MS = 6 * 60 * 60 * 1000;

/** "v1.2.0" or "1.2.0" as [1, 2, 0], or null. */
function parseVersion(text) {
  const m = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(String(text || "").trim());
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

/** Whether `latest` is a later version than `current`. */
function isNewer(latest, current) {
  const a = parseVersion(latest);
  const b = parseVersion(current);
  if (!a || !b) return false;
  for (let i = 0; i < 3; i += 1) if (a[i] !== b[i]) return a[i] > b[i];
  return false;
}

/**
 * `fetchJson(url)` resolves with the parsed body. The answer is kept for
 * 6 hours; a failure answers with what's known (or no latest).
 */
function createUpdateCheck({ current, fetchJson, now = () => Date.now(), log = () => {} }) {
  let cached = null;
  let at = 0;
  let pending = null;
  return {
    async get() {
      if (cached && now() - at < EVERY_MS) return cached;
      if (pending) return pending;
      pending = (async () => {
        try {
          const body = await fetchJson(LATEST_URL);
          const tag = body && typeof body.tag_name === "string" ? body.tag_name : null;
          const latest = tag && parseVersion(tag) ? tag.replace(/^v/, "") : null;
          cached = {
            current,
            latest,
            newer: latest ? isNewer(latest, current) : false,
            url: typeof body?.html_url === "string" && body.html_url.startsWith("https://github.com/") ? body.html_url : RELEASES_URL,
          };
          at = now();
        } catch (err) {
          log(`update check: ${err && err.message ? err.message : err}`);
          if (!cached) cached = { current, latest: null, newer: false, url: RELEASES_URL };
        } finally {
          pending = null;
        }
        return cached;
      })();
      return pending;
    },
  };
}

module.exports = { LATEST_URL, RELEASES_URL, parseVersion, isNewer, createUpdateCheck };
