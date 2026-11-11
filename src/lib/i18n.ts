import { Fragment, createElement, useSyncExternalStore, type ReactNode } from "react";
import en from "../locales/en.json" with { type: "json" };
import de from "../locales/de.json" with { type: "json" };
import zhCN from "../locales/zh-CN.json" with { type: "json" };

/**
 * Every language the interface speaks, by BCP 47 tag, with its own name for the picker. Adding
 * one is a file in src/locales/ and a line here; the picker, the system match and the tests
 * (.uiverify/i18n.test.mjs) follow from this list. English is the source every other file is
 * checked against, and the fallback for any string a translation doesn't have yet.
 */
const LOCALES = {
  en: { name: "English", strings: en },
  de: { name: "Deutsch", strings: de },
  "zh-CN": { name: "简体中文", strings: zhCN },
} as const satisfies Record<string, { name: string; strings: object }>;

export type ResolvedLanguage = keyof typeof LOCALES;
export type LanguageChoice = "auto" | ResolvedLanguage;

/** Every string's key, from the English file: a key that isn't there fails the type check. */
type Leaves<T, P extends string = ""> = {
  [K in keyof T & string]: T[K] extends string ? `${P}${K}` : Leaves<T[K], `${P}${K}.`>;
}[keyof T & string];
export type Key = Leaves<typeof en>;
export type Params = Record<string, string | number>;
export type Translate = (key: Key, params?: Params) => string;

const KEY = "quince.language";

/** The languages in the picker's order. */
export const LANGUAGES = Object.keys(LOCALES) as ResolvedLanguage[];

const isLanguage = (s: string | null): s is ResolvedLanguage => s !== null && s in LOCALES;

const listeners = new Set<() => void>();
const notify = () => listeners.forEach((l) => l());

/** Subscribe to language changes (compatible with useSyncExternalStore). */
export function subscribeLanguage(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * The language of the system's first preference that the interface speaks: the whole tag first
 * ("zh-CN"), then its language alone ("zh-Hans-SG" → "zh-CN"), else English.
 */
export function systemLanguage(): ResolvedLanguage {
  const wanted = typeof navigator === "undefined" ? [] : navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of wanted) {
    if (!tag) continue;
    const exact = LANGUAGES.find((l) => l.toLowerCase() === tag.toLowerCase());
    if (exact) return exact;
    const base = tag.split("-")[0].toLowerCase();
    const near = LANGUAGES.find((l) => l.split("-")[0].toLowerCase() === base);
    if (near) return near;
  }
  return "en";
}

let memoryLanguage: LanguageChoice | null = null;

/** The language chosen in Settings, or "auto" to follow the system. */
export function storedLanguage(): LanguageChoice {
  if (memoryLanguage !== null) return memoryLanguage;
  try {
    const saved = window.localStorage.getItem(KEY);
    return isLanguage(saved) ? saved : "auto";
  } catch {
    return "auto";
  }
}

/** The language the interface is in now. */
export function activeLanguage(): ResolvedLanguage {
  const choice = storedLanguage();
  return choice === "auto" ? systemLanguage() : choice;
}

/** The document's lang attribute, for screen readers, fonts and hyphenation. */
export function applyLanguage(lang: ResolvedLanguage) {
  if (typeof document !== "undefined") document.documentElement.lang = lang;
}

/** Save the choice and redraw every window's strings. */
export function setLanguage(choice: LanguageChoice) {
  memoryLanguage = choice;
  try {
    if (choice === "auto") window.localStorage.removeItem(KEY);
    else window.localStorage.setItem(KEY, choice);
  } catch {
    // Kept for this window only.
  }
  applyLanguage(activeLanguage());
  notify();
}

/** Follow a choice made in another window, as followTheme() does. */
export function followLanguage() {
  if (typeof window === "undefined") return;
  window.addEventListener("storage", (e) => {
    if (e.key === KEY || e.key === null) {
      memoryLanguage = null;
      applyLanguage(activeLanguage());
      notify();
    }
  });
}

/** A language's own name ("简体中文"), as the picker shows it. */
export function languageName(lang: ResolvedLanguage): string {
  return LOCALES[lang].name;
}

function lookup(strings: object, key: string): string | undefined {
  let at: unknown = strings;
  for (const part of key.split(".")) {
    if (at === null || typeof at !== "object") return undefined;
    at = (at as Record<string, unknown>)[part];
  }
  return typeof at === "string" ? at : undefined;
}

/**
 * A string in the active language, else English, with `{name}` filled from `params`. `lang`
 * asks for one language whatever is active.
 */
export function t(key: Key, params?: Params, lang: ResolvedLanguage = activeLanguage()): string {
  const text = lookup(LOCALES[lang].strings, key) ?? lookup(en, key) ?? key;
  return params ? text.replace(/\{(\w+)\}/g, (all, k: string) => (k in params ? String(params[k]) : all)) : text;
}

/**
 * An app's name as the sidebar says it, by its screen key or its service ("drivews" and
 * "drive" are both Drive), or null for something that isn't an app.
 */
export function appName(key: string): string | null {
  const k = `sidebar.${key === "drivews" ? "drive" : key}`;
  return lookup(en, k) === undefined ? null : t(k as Key);
}

/**
 * A translated string whose `<b>…</b>` and `<strong>…</strong>` parts are drawn as those
 * elements (names of buttons, settings and sites), so a translation keeps them where its own
 * word order puts them. Nothing else in a string is markup.
 */
export function rich(text: string): ReactNode {
  const parts: ReactNode[] = [];
  let at = 0;
  for (const m of text.matchAll(/<(b|strong)>(.*?)<\/\1>/g)) {
    parts.push(text.slice(at, m.index), createElement(m[1], { key: parts.length }, m[2]));
    at = m.index + m[0].length;
  }
  parts.push(text.slice(at));
  return createElement(Fragment, null, ...parts);
}

/** The language and `t`, redrawing the component when the language changes. */
export function useI18n() {
  const lang = useSyncExternalStore(subscribeLanguage, activeLanguage, activeLanguage);
  return { t, lang, choice: storedLanguage(), setLanguage };
}
