"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { createPointerSpeed, DEFAULT_SPEED, OFF } = require("../src/pointer-speed");

/** A pretend Windows: one system speed, and a record of every change. */
function fakeWindows(speed = 10) {
  const sets = [];
  return {
    sets,
    get speed() {
      return speed;
    },
    set speed(v) {
      speed = v;
    },
    backend: {
      supported: true,
      get: () => speed,
      set: (v) => {
        sets.push(v);
        speed = v;
        return true;
      },
    },
  };
}

function memoryStore(initial = {}) {
  const data = { ...initial };
  return { data, get: (k, f) => (k in data ? data[k] : f), set: (k, v) => (data[k] = v) };
}

test("automatic: 15 while in front, the person's own speed back on blur", () => {
  const win = fakeWindows(10);
  const store = memoryStore();
  const p = createPointerSpeed({ backend: win.backend, store });
  p.focus();
  assert.equal(win.speed, DEFAULT_SPEED);
  assert.deepEqual(store.data.pointerRestore, { original: 10, applied: 15 });
  p.blur();
  assert.equal(win.speed, 10);
  assert.equal(store.data.pointerRestore, null);
  assert.deepEqual(win.sets, [15, 10]);
});

test("automatic never slows down someone who is already faster", () => {
  const win = fakeWindows(18);
  const p = createPointerSpeed({ backend: win.backend, store: memoryStore() });
  p.focus();
  p.blur();
  assert.equal(win.speed, 18);
  assert.deepEqual(win.sets, []);
});

test("repeated focus and blur never drift from the person's speed", () => {
  const win = fakeWindows(7);
  const p = createPointerSpeed({ backend: win.backend, store: memoryStore() });
  for (let i = 0; i < 25; i += 1) {
    p.focus();
    p.focus();
    assert.equal(win.speed, 15);
    p.blur();
    p.blur();
    assert.equal(win.speed, 7);
  }
});

test("a speed the person changed elsewhere meanwhile is left alone", () => {
  const win = fakeWindows(10);
  const p = createPointerSpeed({ backend: win.backend, store: memoryStore() });
  p.focus();
  win.speed = 12; // changed by something else while the app was in front
  p.blur();
  assert.equal(win.speed, 12);
  p.focus();
  assert.equal(win.speed, 15);
  p.blur();
  assert.equal(win.speed, 12);
});

test("choosing a speed, off, and automatic while in front", () => {
  const win = fakeWindows(10);
  const store = memoryStore();
  const p = createPointerSpeed({ backend: win.backend, store });
  p.focus();
  assert.equal(p.choose(13).inUse, 13);
  assert.equal(win.speed, 13);
  assert.equal(store.data.pointerSpeed, 13);
  assert.equal(p.choose(OFF).inUse, null);
  assert.equal(win.speed, 10);
  assert.equal(p.choose(10).inUse, null); // same as theirs: nothing to change
  assert.equal(win.speed, 10);
  assert.equal(p.choose(null).inUse, 15);
  p.blur();
  assert.equal(win.speed, 10);
  assert.throws(() => p.choose(21), RangeError);
  assert.throws(() => p.choose(1.5), RangeError);
  assert.throws(() => p.choose("15"), RangeError);
});

test("the choice is kept between runs, and off means hands off", () => {
  const win = fakeWindows(10);
  const store = memoryStore({ pointerSpeed: OFF });
  const p = createPointerSpeed({ backend: win.backend, store });
  p.focus();
  assert.equal(win.speed, 10);
  assert.deepEqual(win.sets, []);
  const status = p.status();
  assert.equal(status.off, true);
  assert.equal(status.own, 10);
});

test("a run that stopped with its speed in use is put right at the next launch", () => {
  const win = fakeWindows(15);
  const store = memoryStore({ pointerRestore: { original: 9, applied: 15 } });
  createPointerSpeed({ backend: win.backend, store }).recover();
  assert.equal(win.speed, 9);
  assert.equal(store.data.pointerRestore, null);

  // After signing out and back in, Windows already went back: nothing to do.
  const win2 = fakeWindows(9);
  const store2 = memoryStore({ pointerRestore: { original: 9, applied: 15 } });
  createPointerSpeed({ backend: win2.backend, store: store2 }).recover();
  assert.deepEqual(win2.sets, []);
  assert.equal(store2.data.pointerRestore, null);
});

test("where it isn't supported, nothing happens", () => {
  const backend = { supported: false, get: () => null, set: () => assert.fail("must not set") };
  const p = createPointerSpeed({ backend, store: memoryStore() });
  p.focus();
  p.blur();
  assert.equal(p.status().supported, false);
});
