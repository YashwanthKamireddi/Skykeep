// i18n lookup and fallback rules, checked directly.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { t, setLanguage, activeLanguage, languageName, appName, systemLanguage, LANGUAGES } from "../src/lib/i18n.ts";

const locales = new URL("../src/locales/", import.meta.url);
const read = (file) => JSON.parse(readFileSync(new URL(file, locales), "utf8"));
const leaves = (o, p = "") =>
  Object.entries(o).flatMap(([k, v]) => (typeof v === "string" ? [[p + k, v]] : leaves(v, `${p}${k}.`)));
const placeholders = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
const tags = (s) => [...s.matchAll(/<\/?(\w+)>/g)].map((m) => m[0]).join("");

test("a key translates to English when English is active", () => {
  setLanguage("en");
  assert.equal(activeLanguage(), "en");
  assert.equal(t("common.cancel"), "Cancel");
  assert.equal(t("settings.title"), "Settings");
  assert.equal(t("settings.appearance"), "Appearance");
});

test("a key translates to Simplified Chinese when zh-CN is active", () => {
  setLanguage("zh-CN");
  assert.equal(activeLanguage(), "zh-CN");
  assert.equal(t("common.cancel"), "取消");
  assert.equal(t("settings.title"), "设置");
  assert.equal(t("settings.appearance"), "外观");
});

test("a key translates to German when de is active", () => {
  setLanguage("de");
  assert.equal(activeLanguage(), "de");
  assert.equal(t("common.cancel"), "Abbrechen");
  assert.equal(t("settings.title"), "Einstellungen");
  assert.equal(t("sync.updatedMinAgo", { count: 5 }), "Vor 5 Min. aktualisiert");
  assert.equal(appName("drivews"), "Drive");
  assert.equal(appName("reminders"), "Erinnerungen");
});

test("German from Germany, Austria or Switzerland is German, and a language not offered is English", () => {
  const real = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  const prefer = (...languages) =>
    Object.defineProperty(globalThis, "navigator", { value: { languages, language: languages[0] }, configurable: true });
  try {
    for (const tag of ["de", "de-DE", "de-AT", "de-CH"]) {
      prefer(tag, "en-US");
      assert.equal(systemLanguage(), "de", tag);
    }
    prefer("fr-FR", "de-DE");
    assert.equal(systemLanguage(), "de", "the first preference the interface speaks");
    prefer("fr-FR");
    assert.equal(systemLanguage(), "en");
  } finally {
    if (real) Object.defineProperty(globalThis, "navigator", real);
    else delete globalThis.navigator;
  }
});

test("a language asked for by name answers in it, whatever is active", () => {
  setLanguage("zh-CN");
  assert.equal(t("common.cancel", undefined, "en"), "Cancel");
});

test("missing string falls back to key when absent from all dictionaries", () => {
  assert.equal(t("nonexistent.key.test"), "nonexistent.key.test");
});

test("parameters interpolate into template placeholders in English and Chinese", () => {
  setLanguage("en");
  assert.equal(t("sync.updatedMinAgo", { count: 5 }), "Updated 5 min ago");
  assert.equal(t("sync.appUpdating", { name: "Notes" }), "Updating Notes…");

  setLanguage("zh-CN");
  assert.equal(t("sync.updatedMinAgo", { count: 5 }), "更新于 5 分钟前");
  assert.equal(t("sync.appUpdating", { name: "备忘录" }), "正在更新备忘录…");
});

test("language name helper returns readable name", () => {
  assert.equal(languageName("en"), "English");
  assert.equal(languageName("zh-CN"), "简体中文");
  assert.equal(languageName("de"), "Deutsch");
});

test("an app's name comes from its screen key or its service, and nothing else is an app", () => {
  setLanguage("en");
  assert.equal(appName("drivews"), "Drive");
  assert.equal(appName("drive"), "Drive");
  setLanguage("zh-CN");
  assert.equal(appName("notes"), "备忘录");
  assert.equal(appName("settings-extra"), null);
});

test("every file in src/locales is a language the picker offers, and every language has its file", () => {
  const files = readdirSync(locales).filter((f) => f.endsWith(".json")).map((f) => f.slice(0, -5)).sort();
  assert.deepEqual(files, [...LANGUAGES].sort());
});

// A translation is checked against English: the same keys (none missing, none left over from a
// removed string), the same {placeholders} (a translation may leave out a plural's count word,
// never add one), and the same bold parts, so no string loses its emphasis or shows a raw tag.
for (const lang of LANGUAGES.filter((l) => l !== "en")) {
  test(`${lang} has every English string, with the same placeholders and bold parts`, () => {
    const en = new Map(leaves(read("en.json")));
    const other = new Map(leaves(read(`${lang}.json`)));
    assert.deepEqual([...other.keys()].filter((k) => !en.has(k)), [], "keys English doesn't have");
    assert.deepEqual([...en.keys()].filter((k) => !other.has(k)), [], "English keys not translated");
    for (const [k, v] of other) {
      const extra = placeholders(v).filter((p) => !placeholders(en.get(k)).includes(p));
      assert.deepEqual(extra, [], `${k}: placeholders English doesn't fill`);
      assert.equal(tags(v), tags(en.get(k)), `${k}: bold parts differ from English`);
    }
  });
}
