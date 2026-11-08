"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const http = require("node:http");
const { createBrowserSignIn, isPublicKeyHex, signInPageUrl, readAnswer } = require("../src/browser-sign-in");

const KEY = "3059301306072a8648ce3d020106082a8648ce3d03010703420004".padEnd(182, "ab");
const PRINCIPAL = "2vxsx-fae";
const USER = "o2ivq-5dsz3-nba5d-pwbk2-hdd3i-vybeq-qfz35-rqg27-lyesf-xghzc-3ae";
const CHAIN = JSON.stringify({
  publicKey: "303c300c060a2b0601040183b8430102032c000a0000000000000007010166",
  delegations: [{ delegation: { pubkey: KEY, expiration: "18a1b2c3d4e5f600" }, signature: "d9d9f7" }],
});
const enc = (text) => Buffer.from(text, "utf8").toString("base64url");

function get(port, path) {
  return new Promise((resolve, reject) => {
    http
      .get({ host: "127.0.0.1", port, path, agent: false }, (res) => {
        let body = "";
        res.on("data", (c) => (body += c));
        res.on("end", () => resolve({ status: res.statusCode, body }));
      })
      .on("error", reject);
  });
}

/** A sign-in whose "browser" just records the page it was asked to open. */
function harness(options = {}) {
  const opened = [];
  let focused = 0;
  const signIn = createBrowserSignIn({
    appUrl: "https://cad-dza.caffeine.xyz/",
    openExternal: (url) => {
      opened.push(url);
      return Promise.resolve();
    },
    focusApp: () => (focused += 1),
    ...options,
  });
  const waitForPage = async () => {
    for (let i = 0; i < 100 && opened.length === 0; i += 1) await new Promise((r) => setTimeout(r, 10));
    const url = new URL(opened[opened.length - 1]);
    const params = new URLSearchParams(url.hash.slice(1));
    return { url, key: params.get("k"), port: Number(params.get("p")), state: params.get("s") };
  };
  return { signIn, opened, waitForPage, focused: () => focused };
}

test("only DER public keys in hex are accepted", () => {
  assert.equal(isPublicKeyHex(KEY), true);
  assert.equal(isPublicKeyHex("ab"), false);
  assert.equal(isPublicKeyHex(KEY.toUpperCase()), false);
  assert.equal(isPublicKeyHex(`${KEY}a`), false);
  assert.equal(isPublicKeyHex(42), false);
});

test("the browser opens the app's page, with everything after the #", () => {
  const url = signInPageUrl("https://cad-dza.caffeine.xyz/", "ab12", 51234, "ff00");
  assert.equal(url, "https://cad-dza.caffeine.xyz/desktop-sign-in#k=ab12&p=51234&s=ff00");
});

test("the answer must be GET /done with the state, an account and a chain", () => {
  const state = "a".repeat(64);
  const good = `/done?s=${state}&a=${USER}&d=${enc(CHAIN)}`;
  assert.deepEqual(readAnswer(good, "GET", state), { result: { chain: CHAIN, principal: USER } });
  assert.equal(readAnswer(good, "POST", state).status, 404);
  assert.equal(readAnswer(`/other?s=${state}`, "GET", state).status, 404);
  assert.equal(readAnswer(good.replace(state, "b".repeat(64)), "GET", state).status, 400);
  assert.equal(readAnswer(`/done?s=${state}&a=${USER}&d=${enc("{}")}`, "GET", state).status, 400);
  assert.equal(readAnswer(`/done?s=${state}&a=${USER}&d=%%%`, "GET", state).status, 400);
  assert.equal(readAnswer(`/done?s=${state}&a=Not%20a%20principal&d=${enc(CHAIN)}`, "GET", state).status, 400);
  assert.equal(readAnswer(`/done?s=${state}&a=${PRINCIPAL}&d=${enc(CHAIN)}`, "GET", state).result.principal, PRINCIPAL);
});

test("a whole sign-in: the browser's answer reaches the app, and the listener closes", async () => {
  const h = harness();
  const pending = h.signIn.start(KEY);
  const { url, key, port, state } = await h.waitForPage();
  assert.equal(url.origin + url.pathname, "https://cad-dza.caffeine.xyz/desktop-sign-in");
  assert.equal(key, KEY);
  assert.match(state, /^[0-9a-f]{64}$/);

  const wrong = await get(port, `/done?s=${"0".repeat(64)}&a=${USER}&d=${enc(CHAIN)}`);
  assert.equal(wrong.status, 400);
  assert.match(wrong.body, /isn&#39;t the one the desktop app is waiting for/);

  const right = await get(port, `/done?s=${state}&a=${USER}&d=${enc(CHAIN)}`);
  assert.equal(right.status, 200);
  assert.match(right.body, /You're signed in|You&#39;re signed in/);
  assert.deepEqual(await pending, { chain: CHAIN, principal: USER });
  assert.equal(h.focused(), 1);
  assert.equal(h.signIn.serverForTests, null);
  await new Promise((r) => setTimeout(r, 50));
  await assert.rejects(get(port, `/done?s=${state}&a=${USER}&d=${enc(CHAIN)}`));
  assert.equal(h.focused(), 1);
});

test("a long chain fits in the address", async () => {
  const h = harness();
  const big = JSON.stringify({ publicKey: "30", delegations: [{ delegation: { pubkey: KEY, expiration: "1" }, signature: "ab".repeat(20000) }] });
  const pending = h.signIn.start(KEY);
  const { port, state } = await h.waitForPage();
  const res = await get(port, `/done?s=${state}&a=${USER}&d=${enc(big)}`);
  assert.equal(res.status, 200);
  assert.equal((await pending).chain, big);
});

test("cancel, a new sign-in, and the time limit each end the one waiting", async () => {
  const h = harness();
  const first = h.signIn.start(KEY);
  await h.waitForPage();
  h.signIn.cancel();
  await assert.rejects(first, /cancelled/);

  const second = h.signIn.start(KEY);
  await h.waitForPage();
  const third = h.signIn.start(KEY);
  await assert.rejects(second, /cancelled/);
  h.signIn.cancel();
  await assert.rejects(third, /cancelled/);

  const quick = harness({ timeoutMs: 50 });
  await assert.rejects(quick.signIn.start(KEY), /timed out/);
  await assert.rejects(quick.signIn.start("nothex"), /can't be used/);
});

test("if the browser can't be opened, the sign-in ends", async () => {
  const signIn = createBrowserSignIn({
    appUrl: "https://cad-dza.caffeine.xyz/",
    openExternal: () => Promise.reject(new Error("no browser")),
  });
  await assert.rejects(signIn.start(KEY), /couldn't be opened/);
});
