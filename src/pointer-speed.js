"use strict";
// Pointer speed while CAD Protocol is in front.
//
// Windows keeps one pointer speed for the whole computer (1 to 20, 10 is the
// default). Drafting feels better a little faster, so while a CAD Protocol
// window has focus the app sets the speed, and the moment it loses focus,
// is minimised or quits, the person's own speed goes back.
//
// The change is made for this sign-in session only (it is never written to
// the person's Windows settings), so even a crash can't leave it behind
// after they sign out. The speed we replaced is also kept in a file, and the
// next launch puts it back if the app was stopped while its speed was in use.

const MIN_SPEED = 1;
const MAX_SPEED = 20;
/** The speed used until the person picks one: about one and a half times Windows' default of 10. */
const DEFAULT_SPEED = 15;
/** Stored choice meaning "leave the pointer alone". */
const OFF = 0;

const SPI_GETMOUSESPEED = 0x0070;
const SPI_SETMOUSESPEED = 0x0071;
/** No SPIF_UPDATEINIFILE: never saved to the person's settings. No SPIF_SENDCHANGE: no broadcast, so the call can't stall on a busy program. */
const SPIF_NONE = 0;

const unsupported = Object.freeze({ supported: false, get: () => null, set: () => false });

/** The Windows calls, through koffi. Anywhere else, or if koffi can't load, the feature is simply off. */
function windowsBackend(log = () => {}) {
  if (process.platform !== "win32") return unsupported;
  try {
    const koffi = require("koffi");
    const user32 = koffi.load("user32.dll");
    const getSpeed = user32.func("__stdcall", "SystemParametersInfoW", "int", [
      "uint32",
      "uint32",
      koffi.out(koffi.pointer("int32")),
      "uint32",
    ]);
    // SPI_SETMOUSESPEED takes the speed itself in the pointer argument.
    const setSpeed = user32.func("__stdcall", "SystemParametersInfoW", "int", ["uint32", "uint32", "uintptr_t", "uint32"]);
    return {
      supported: true,
      get() {
        const out = [0];
        if (!getSpeed(SPI_GETMOUSESPEED, 0, out, 0)) return null;
        const speed = out[0];
        return Number.isInteger(speed) && speed >= MIN_SPEED && speed <= MAX_SPEED ? speed : null;
      },
      set(speed) {
        return Boolean(setSpeed(SPI_SETMOUSESPEED, 0, speed, SPIF_NONE));
      },
    };
  } catch (err) {
    log(`pointer speed unavailable: ${err && err.message ? err.message : err}`);
    return unsupported;
  }
}

function validChoice(value) {
  return value === null || value === OFF || (Number.isInteger(value) && value >= MIN_SPEED && value <= MAX_SPEED);
}

/**
 * The controller. `backend` reads and sets the system speed; `store` keeps
 * the person's choice ("pointerSpeed": null for automatic, 0 for off, or
 * 1..20) and the speed to put back ("pointerRestore").
 *
 * Automatic means at least DEFAULT_SPEED: someone whose own speed is already
 * faster keeps theirs.
 */
function createPointerSpeed({ backend, store, log = () => {} }) {
  let chosen = store.get("pointerSpeed", null);
  if (!validChoice(chosen)) chosen = null;
  /** The person's own speed while ours is in use, else null. */
  let original = null;
  /** The speed we set, else null. */
  let applied = null;
  let focused = false;

  function remember() {
    store.set("pointerRestore", applied === null ? null : { original, applied });
  }

  function targetFor(own) {
    if (chosen === OFF) return null;
    if (chosen === null) return Math.max(own, DEFAULT_SPEED);
    return chosen;
  }

  function apply() {
    if (!backend.supported || chosen === OFF) return;
    const own = applied === null ? backend.get() : original;
    if (own === null) return;
    const target = targetFor(own);
    if (target === null) return;
    if (applied === target) return;
    if (applied === null && target === own) return;
    if (!backend.set(target)) {
      log(`couldn't set pointer speed ${target}`);
      return;
    }
    original = own;
    applied = target;
    remember();
  }

  function restore() {
    if (applied === null) return;
    const now = backend.get();
    // Put theirs back only if ours is still in use; if something else changed it meanwhile, leave that.
    if (now === applied || now === null) {
      if (!backend.set(original)) log(`couldn't restore pointer speed ${original}`);
    }
    original = null;
    applied = null;
    remember();
  }

  return {
    /** At launch: put back a speed left in use by a run that didn't finish. */
    recover() {
      const saved = store.get("pointerRestore", null);
      if (!saved || !backend.supported) return;
      const ok = validChoice(saved.original) && validChoice(saved.applied) && saved.original && saved.applied;
      if (ok && backend.get() === saved.applied) {
        backend.set(saved.original);
        log(`pointer speed ${saved.original} put back after an unfinished run`);
      }
      store.set("pointerRestore", null);
    },
    focus() {
      focused = true;
      apply();
    },
    blur() {
      focused = false;
      restore();
    },
    /** Change the choice: null for automatic, 0 for off, or 1..20. */
    choose(value) {
      if (!validChoice(value)) throw new RangeError(`pointer speed must be null, 0 or ${MIN_SPEED}..${MAX_SPEED}`);
      chosen = value;
      store.set("pointerSpeed", value);
      if (!focused) return this.status();
      if (chosen === OFF) restore();
      else {
        if (applied !== null && targetFor(original) === original) restore();
        else apply();
      }
      return this.status();
    },
    status() {
      const own = applied === null ? (backend.supported ? backend.get() : null) : original;
      return {
        supported: backend.supported,
        chosen,
        automatic: chosen === null,
        off: chosen === OFF,
        own,
        inUse: applied,
        defaultSpeed: DEFAULT_SPEED,
        min: MIN_SPEED,
        max: MAX_SPEED,
      };
    },
  };
}

module.exports = {
  MIN_SPEED,
  MAX_SPEED,
  DEFAULT_SPEED,
  OFF,
  SPI_GETMOUSESPEED,
  SPI_SETMOUSESPEED,
  windowsBackend,
  createPointerSpeed,
};
