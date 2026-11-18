"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { createRecovery, isCopy, MOST_COPIES } = require("../src/recovery");

const copy = (over = {}) => ({ id: "demo|new:s1:d1", user: "demo", name: "Lot 7", projectId: null, fileId: null, owner: null, savedAt: 1000, objects: 3, text: "CADP 1\n", ...over });

test("1.28: recovery copies kept as files, read back, replaced and let go", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "cadp-recovery-"));
  const r = createRecovery({ dir: path.join(dir, "recovery"), fs });
  assert.deepEqual(await r.list(), []);
  await r.put(copy());
  await r.put(copy({ id: "p|file:42", name: "Smith", fileId: "42", savedAt: 2000 }));
  // The same drawing again: one copy, the newest.
  await r.put(copy({ text: "CADP 2\n", savedAt: 3000 }));
  const all = (await r.list()).sort((a, b) => a.savedAt - b.savedAt);
  assert.equal(all.length, 2);
  assert.equal(all[1].text, "CADP 2\n");
  // Named by the id's hash: no path from the page.
  assert.ok(fs.readdirSync(path.join(dir, "recovery")).every((n) => /^[0-9a-f]{40}\.json$/.test(n)));
  await r.remove("demo|new:s1:d1");
  assert.deepEqual((await r.list()).map((c) => c.name), ["Smith"]);
  // A broken or foreign file is left alone and skipped.
  fs.writeFileSync(path.join(dir, "recovery", "x.json"), "{not json");
  assert.equal((await r.list()).length, 1);
  await assert.rejects(r.put({ id: "../../x", text: 5 }));
  assert.equal(await r.remove(""), false);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("1.28: past the most copies kept, the oldest go", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "cadp-recovery-"));
  const r = createRecovery({ dir, fs });
  for (let i = 0; i < MOST_COPIES + 3; i += 1) await r.put(copy({ id: `u|new:s:${i}`, savedAt: i }));
  const left = await r.list();
  assert.equal(left.length, MOST_COPIES);
  assert.equal(Math.min(...left.map((c) => c.savedAt)), 3);
  assert.equal(isCopy(copy()), true);
  assert.equal(isCopy({ ...copy(), savedAt: "x" }), false);
  fs.rmSync(dir, { recursive: true, force: true });
});
