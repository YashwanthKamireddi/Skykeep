// The video player's clocks, drawn every frame while a film plays: their text is written only
// when it changes, once a second, not sixty times.
import { test } from "node:test";
import assert from "node:assert/strict";
import { clock, say } from "../src/screens/photos/clock.ts";

/** A label that counts how often its text is written. */
const label = () => {
  let text = "0:00";
  const el = { writes: 0 };
  Object.defineProperty(el, "textContent", {
    get: () => text,
    set: (v) => { text = v; el.writes++; },
  });
  return el;
};

test("ten seconds of a film at 60 frames a second writes each clock once a second", () => {
  const said = label();
  const left = label();
  const d = 30;
  for (let frame = 0; frame < 600; frame++) {
    const at = frame / 60;
    say(said, clock(at));
    say(left, `−${clock(d - at)}`);
  }
  // 0:00 is already there; 0:01 … 0:09 are new. What is left starts unwritten (−0:00).
  assert.equal(said.writes, 9);
  assert.ok(left.writes <= 11, `${left.writes} writes`);
  assert.equal(said.textContent, "0:09");
});

test("the clocks say what Photos says", () => {
  assert.equal(clock(7), "0:07");
  assert.equal(clock(750), "12:30");
  assert.equal(clock(3725), "1:02:05");
  assert.equal(clock(Number.NaN), "0:00");
  say(null, "0:01");
});
