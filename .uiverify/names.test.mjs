// Names in the order their own language writes them (#232).
import { test } from "node:test";
import assert from "node:assert/strict";
import { familyFirst, personName } from "../src/lib/names.ts";

test("Chinese, Japanese and Korean names: family name first, nothing between", () => {
  assert.equal(personName("小明", null, "王"), "王小明");
  assert.equal(personName("駿", null, "宮崎"), "宮崎駿");
  assert.equal(personName("ひろし", null, "やまだ"), "やまだひろし");
  assert.equal(personName("민준", null, "김"), "김민준");
  assert.equal(personName("小明", "", "王"), "王小明");
});

test("other names: given name first, a space between", () => {
  assert.equal(personName("Asha", null, "Rao"), "Asha Rao");
  assert.equal(personName("Mary", "Ann", "Lee"), "Mary Ann Lee");
  assert.equal(personName("José", null, "García"), "José García");
});

test("mixed scripts or one part only stay as they are", () => {
  assert.equal(personName("Kevin", null, "王"), "Kevin 王");
  assert.equal(personName("王", null, null), "王");
  assert.equal(personName(null, null, "Rao"), "Rao");
  assert.equal(personName("  ", null, null), "");
  assert.equal(personName(null, null, null), "");
});

test("familyFirst says which order a name takes", () => {
  assert.equal(familyFirst("小明", null, "王"), true);
  assert.equal(familyFirst("Kevin", null, "王"), false);
  assert.equal(familyFirst("Asha", null, "Rao"), false);
  assert.equal(familyFirst(null, null, null), false);
});
