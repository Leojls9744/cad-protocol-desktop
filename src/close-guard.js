"use strict";
// Asking the app before its window closes (1.2 on), so it can save open
// drawings to the vault and sign out when the person's setting says so
// (Settings › Account and security › Sign out when I close the app).
//
// The page answers "started" at once and "done" when it's finished. A page
// that doesn't answer within 1.5 s (an older app, the offline page) closes
// straight away; one that started gets up to a minute.
//
// 1.18: the page may ask the person first (Save changes to 2 drawings?
// Save, Don't Save, Cancel): "holding" while they answer, with no time
// limit; "started" then gives its saving and signing out the minute from
// then; "cancelled" (Cancel, or a drawing that wasn't saved) keeps the
// window open.

const ACK_MS = 1500;
const MAX_MS = 60 * 1000;

/**
 * `ask(id)` sends the question to the page; `close()` closes for real.
 * Returns `{ request(), started(id), done(id), closing }`: call request() from
 * the window's close event (it says whether to hold the close), and started /
 * done when the page answers.
 */
function createCloseGuard({ ask, close, ackMs = ACK_MS, maxMs = MAX_MS }) {
  let state = "open"; // open | asking | closing
  let id = 0;
  let ackTimer = null;
  let maxTimer = null;
  /** The page is waiting on the person (1.18): no limit till it starts work. */
  let held = false;

  function finish() {
    if (state === "closing") return;
    state = "closing";
    clearTimeout(ackTimer);
    clearTimeout(maxTimer);
    close();
  }

  return {
    /** From the close event: true means hold it (we're asking or about to). */
    request() {
      if (state === "closing") return false;
      if (state === "asking") return true;
      state = "asking";
      held = false;
      id += 1;
      ackTimer = setTimeout(finish, ackMs);
      maxTimer = setTimeout(finish, maxMs);
      ask(id);
      return true;
    },
    started(answerId) {
      if (state !== "asking" || answerId !== id) return;
      clearTimeout(ackTimer);
      // After the person answered: the work gets its minute from now.
      if (held) {
        held = false;
        maxTimer = setTimeout(finish, maxMs);
      }
    },
    /** 1.18: the page is asking the person; it waits however long they take. */
    holding(answerId) {
      if (state !== "asking" || answerId !== id) return;
      clearTimeout(ackTimer);
      clearTimeout(maxTimer);
      held = true;
    },
    /** 1.18: the person chose to stay (or a drawing wasn't saved): the window stays open. */
    cancelled(answerId) {
      if (state === "asking" && answerId === id) this.reset();
    },
    done(answerId) {
      if (state === "asking" && answerId === id) finish();
    },
    /** The close didn't happen after all (the person chose Stay): ask again next time. */
    reset() {
      clearTimeout(ackTimer);
      clearTimeout(maxTimer);
      held = false;
      state = "open";
    },
    get closing() {
      return state === "closing";
    },
    /** Asked and not answered "done" yet (1.7: the card shows meanwhile). */
    get asking() {
      return state === "asking";
    },
  };
}

module.exports = { createCloseGuard, ACK_MS, MAX_MS };
