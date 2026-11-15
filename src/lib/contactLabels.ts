/**
 * Apple's own names for the labels on a card — the interface's half of
 * `quince-core/src/contacts_labels.rs`, which says why there are two spellings.
 *
 * Contacts on the phone keeps `_$!<Mobile>!$_`; Apple's web service hands the same label out as
 * `MOBILE` and takes it back that way. A label is shown and compared here by what it means; a
 * label picked here is written in the service's spelling, as icloud.com writes it; a label
 * already on a card goes back exactly as it came.
 */
import { t, type Key } from "./i18n.ts";

/** [service spelling, Apple's token, as a card shows it] */
const KNOWN: [string, string, string][] = [
  ["MOBILE", "_$!<Mobile>!$_", "mobile"],
  ["IPHONE", "iPhone", "iPhone"],
  ["APPLE WATCH", "_$!<AppleWatch>!$_", "Apple Watch"],
  ["HOME", "_$!<Home>!$_", "home"],
  ["WORK", "_$!<Work>!$_", "work"],
  ["SCHOOL", "_$!<School>!$_", "school"],
  ["MAIN", "_$!<Main>!$_", "main"],
  ["HOME FAX", "_$!<HomeFAX>!$_", "home fax"],
  ["WORK FAX", "_$!<WorkFAX>!$_", "work fax"],
  ["OTHER FAX", "_$!<OtherFAX>!$_", "other fax"],
  ["PAGER", "_$!<Pager>!$_", "pager"],
  ["PHONE", "_$!<Phone>!$_", "phone"],
  ["OTHER", "_$!<Other>!$_", "other"],
  ["ICLOUD", "iCloud", "iCloud"],
  ["HOMEPAGE", "_$!<HomePage>!$_", "homepage"],
  ["BLOG", "_$!<Blog>!$_", "blog"],
  ["ANNIVERSARY", "_$!<Anniversary>!$_", "anniversary"],
  ["BIRTHDAY", "_$!<Birthday>!$_", "birthday"],
  ["MOTHER", "_$!<Mother>!$_", "mother"],
  ["FATHER", "_$!<Father>!$_", "father"],
  ["PARENT", "_$!<Parent>!$_", "parent"],
  ["BROTHER", "_$!<Brother>!$_", "brother"],
  ["SISTER", "_$!<Sister>!$_", "sister"],
  ["SON", "_$!<Son>!$_", "son"],
  ["DAUGHTER", "_$!<Daughter>!$_", "daughter"],
  ["CHILD", "_$!<Child>!$_", "child"],
  ["FRIEND", "_$!<Friend>!$_", "friend"],
  ["SPOUSE", "_$!<Spouse>!$_", "spouse"],
  ["PARTNER", "_$!<Partner>!$_", "partner"],
  ["ASSISTANT", "_$!<Assistant>!$_", "assistant"],
  ["MANAGER", "_$!<Manager>!$_", "manager"],
];

/** Instant messaging services and social networks, by Apple's names for them. */
const SERVICES: Record<string, string> = {
  AIM: "AIM", YAHOO: "Yahoo", MSN: "MSN", ICQ: "ICQ", JABBER: "Jabber", SKYPE: "Skype",
  GOOGLETALK: "Google Talk", FACEBOOK: "Facebook", QQ: "QQ", GADUGADU: "Gadu-Gadu",
  TWITTER: "Twitter", FLICKR: "Flickr", LINKEDIN: "LinkedIn", MYSPACE: "Myspace", SINAWEIBO: "Sina Weibo",
  GAMECENTER: "Game Center",
};

const byAny = new Map<string, [string, string, string]>();
for (const k of KNOWN) for (const spelling of k) byAny.set(spelling.toLowerCase(), k);

/** The label Apple means, in its service spelling; null for a custom one. */
export function knownLabel(label: string | null | undefined): string | null {
  if (!label) return null;
  return byAny.get(label.trim().toLowerCase())?.[0] ?? null;
}

/** "HOME FAX" → "homeFax": a label's key in the locale files. */
const labelKey = (k: string) =>
  k.toLowerCase().split(" ").map((w, i) => (i ? w[0].toUpperCase() + w.slice(1) : w)).join("");

/** How a card shows a label: Apple's word for one it knows, a service by its name, else as typed. */
export function labelText(label: string): string {
  const k = byAny.get(label.trim().toLowerCase());
  // Apple's own words in the reader's language; iPhone and iCloud are names, never translated.
  if (k) return k[0] === "IPHONE" || k[0] === "ICLOUD" ? k[2] : t(`contacts.label.${labelKey(k[0])}` as Key);
  return SERVICES[label.trim().toUpperCase()] ?? label;
}

/** The labels icloud.com's editor offers for each kind of detail, "other" last as it adds it. */
export const OFFERED: Record<string, string[]> = {
  phone: ["MOBILE", "IPHONE", "APPLE WATCH", "HOME", "WORK", "SCHOOL", "MAIN", "HOME FAX", "WORK FAX", "PAGER", "OTHER"],
  email: ["HOME", "WORK", "SCHOOL", "ICLOUD", "OTHER"],
  url: ["HOMEPAGE", "HOME", "WORK", "SCHOOL", "OTHER"],
  address: ["HOME", "WORK", "SCHOOL", "OTHER"],
  date: ["ANNIVERSARY", "OTHER"],
  related: ["MOTHER", "FATHER", "PARENT", "BROTHER", "SISTER", "SON", "DAUGHTER", "CHILD", "FRIEND", "SPOUSE", "PARTNER", "ASSISTANT", "MANAGER", "OTHER"],
  im: ["AIM", "YAHOO", "MSN", "ICQ", "JABBER", "SKYPE", "GOOGLETALK", "FACEBOOK", "QQ", "GADUGADU"],
  profile: ["TWITTER", "FACEBOOK", "FLICKR", "LINKEDIN", "MYSPACE", "SINAWEIBO"],
};

/** The label a new detail of each kind starts with, as icloud.com starts one. */
export const FIRST_LABEL: Record<string, string> = {
  phone: "MOBILE", email: "HOME", url: "HOMEPAGE", date: "ANNIVERSARY", related: "MOTHER", im: "SKYPE", profile: "TWITTER",
};

/** Whether a label is one of a kind's own (instant messaging and profiles: a service), whichever spelling. */
export function offeredMatch(kind: string, label: string | null): string | null {
  if (!label) return null;
  const list = OFFERED[kind] ?? [];
  const known = knownLabel(label);
  if (known && list.includes(known)) return known;
  const upper = label.trim().toUpperCase();
  return list.includes(upper) && SERVICES[upper] ? upper : null;
}

/** A social profile's page, as icloud.com links one. */
export function profileHref(label: string | null, user: string): string | null {
  const u = user.replace(/^@+/, "");
  switch ((label ?? "").toUpperCase()) {
    case "TWITTER": return `https://twitter.com/${u}`;
    case "FACEBOOK": return `https://www.facebook.com/${u}`;
    case "LINKEDIN": return `https://www.linkedin.com/in/${u}`;
    case "FLICKR": return `https://www.flickr.com/photos/${u}`;
    case "MYSPACE": return `https://myspace.com/${u}`;
    case "SINAWEIBO": return `https://weibo.com/n/${u}`;
    default: return null;
  }
}
