/**
 * The arithmetic behind Calendar's views, kept apart from drawing them so it can be tested.
 *
 * Every date here is local: a calendar is read in the viewer's own time, and a day is from
 * one local midnight to the next — twenty-three or twenty-five hours on the two days a clock
 * moves, which is why nothing here adds a fixed 86,400 seconds.
 */
import type { CalendarEvent } from "../../lib/ipc";
import { timeText } from "../../lib/when.ts";

export const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
export const addDays = (d: Date, n: number) =>
  new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
export const seconds = (d: Date) => Math.floor(d.getTime() / 1000);
export const dateOf = (s: number) => new Date(s * 1000);
export const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
export const dayKey = (d: Date) => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;

/* ---- the week, as this computer's region keeps it ---------------------------------------- */

/** Which day a week starts on and which days are the weekend, 0 for Sunday as `getDay` counts. */
export type WeekInfo = { firstDay: number; weekend: number[] };

// For an engine without `Intl.Locale#getWeekInfo` (WebKitGTK's, for one), CLDR's own answer
// for the regions that differ from a Monday start and a Saturday–Sunday weekend.
const SUNDAY_FIRST = new Set(
  ("AG AS BD BR BS BT BW BZ CA CO DM DO ET GT GU HK HN ID IL IN JM JP KE KH KR LA MH MM MO MT MX " +
    "MZ NI NP PA PE PH PK PR PT PY SA SG SV TH TT TW UM US VE VI WS YE ZA ZW").split(" ")
);
const SATURDAY_FIRST = new Set("AE AF BH DJ DZ EG IQ IR JO KW LY OM QA SD SY".split(" "));
const FRIDAY_SATURDAY = new Set("BH DZ EG IQ IL JO KW LY OM QA SA SD SY YE".split(" "));

/** The week of `locale`'s region: Monday first in Britain, Sunday in the US, Saturday in Egypt. */
export function weekInfoOf(locale: string): WeekInfo {
  try {
    const l = new Intl.Locale(locale) as Intl.Locale & {
      getWeekInfo?: () => { firstDay: number; weekend: number[] };
      weekInfo?: { firstDay: number; weekend: number[] };
    };
    const info = l.getWeekInfo?.() ?? l.weekInfo;
    // Intl counts Monday 1 to Sunday 7; `getDay`, Sunday 0 to Saturday 6.
    if (info) return { firstDay: info.firstDay % 7, weekend: info.weekend.map((d) => d % 7) };
    const region = l.maximize().region ?? "US";
    return {
      firstDay: SUNDAY_FIRST.has(region) ? 0 : SATURDAY_FIRST.has(region) ? 6 : 1,
      weekend: region === "IN" ? [0] : region === "IR" ? [5] : region === "AF" ? [4, 5] : FRIDAY_SATURDAY.has(region) ? [5, 6] : [6, 0],
    };
  } catch {
    return { firstDay: 0, weekend: [6, 0] };
  }
}

/** The first day chosen in Settings: "auto" for the locale's, else a day (`getDay`'s number). */
export type WeekStartChoice = "auto" | "0" | "1" | "6";
const WEEK_KEY = "quince.weekStart";
export function weekStartChoice(): WeekStartChoice {
  try {
    const v = window.localStorage.getItem(WEEK_KEY);
    return v === "0" || v === "1" || v === "6" ? v : "auto";
  } catch {
    return "auto";
  }
}

/** The locale dates are written in: the engine's language until the system's LC_TIME is read. */
let timeLocale =
  (typeof navigator !== "undefined" && navigator.language) || Intl.DateTimeFormat().resolvedOptions().locale;

/** The locale's own first day, whatever Settings chose. */
export const localeFirstDay = () => weekInfoOf(timeLocale).firstDay;

function weekNow(): WeekInfo {
  const info = weekInfoOf(timeLocale);
  const choice = typeof window === "undefined" ? "auto" : weekStartChoice();
  return choice === "auto" ? info : { ...info, firstDay: Number(choice) };
}

/**
 * This computer's week: its locale's (LC_TIME before LANG, read from the system), or the first
 * day chosen in Settings. One object, kept current in place, so every default that reads it
 * reads the latest; a change is told as `quince:week` for the views to draw again.
 */
export const WEEK: WeekInfo = weekNow();
const WEEK_CHANGED = "quince:week";
function refreshWeek() {
  const next = weekNow();
  if (next.firstDay === WEEK.firstDay && next.weekend.join() === WEEK.weekend.join()) return;
  Object.assign(WEEK, next);
  if (typeof window !== "undefined") window.dispatchEvent(new Event(WEEK_CHANGED));
}
export function setWeekStart(choice: WeekStartChoice) {
  try {
    if (choice === "auto") window.localStorage.removeItem(WEEK_KEY);
    else window.localStorage.setItem(WEEK_KEY, choice);
  } catch {
    // Applies for now; the locale's on the next launch.
  }
  refreshWeek();
}
/** Calls `run` whenever the week changes; answers the way to stop. */
// The first day chosen in Settings, in the library window, reaches Calendar's own window too.
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === WEEK_KEY || e.key === null) refreshWeek();
  });
}
export function onWeekChange(run: () => void): () => void {
  window.addEventListener(WEEK_CHANGED, run);
  return () => window.removeEventListener(WEEK_CHANGED, run);
}
if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
  void import("@tauri-apps/api/core")
    .then(({ invoke }) => invoke<string | null>("system_time_locale"))
    .then((tag) => {
      if (!tag) return;
      try {
        new Intl.Locale(tag);
      } catch {
        return;
      }
      timeLocale = tag;
      refreshWeek();
    }, () => {});
}
export const isWeekend = (d: Date) => WEEK.weekend.includes(d.getDay());

/** The day on or before `d` that a week starts on here — Sunday in the US, Monday in Europe. */
export const startOfWeek = (d: Date, firstDay = WEEK.firstDay) =>
  addDays(midnight(d), -((d.getDay() - firstDay + 7) % 7));

/** The six weeks a month shows: from the week holding the 1st, forty-two days. */
export function monthDays(anchor: Date, firstDay = WEEK.firstDay): Date[] {
  const start = startOfWeek(new Date(anchor.getFullYear(), anchor.getMonth(), 1), firstDay);
  return Array.from({ length: 42 }, (_, i) => addDays(start, i));
}

/** Whether this computer's clock is a twelve-hour one, read from its locale's hour cycle. */
export const TWELVE_HOUR = (() => {
  const o = new Intl.DateTimeFormat(undefined, { hour: "numeric" }).resolvedOptions() as { hourCycle?: string; hour12?: boolean };
  return o.hourCycle ? o.hourCycle === "h12" || o.hourCycle === "h11" : o.hour12 ?? true;
})();

/** An event's first and last local day, the last being the day holding its final moment. */
export function daysOf(event: CalendarEvent): [Date, Date] {
  const first = midnight(dateOf(event.startsAt));
  // An all-day event ends at the next midnight, and one that ends exactly at midnight
  // does not reach into the day after.
  const last = midnight(dateOf(Math.max(event.endsAt - 1, event.startsAt)));
  return [first, last];
}

/**
 * Whether an event is drawn as a bar in the all-day row rather than as a block of time.
 *
 * All-day events, and timed ones of a day or more. An overnight event — eleven to one — is
 * a block on each of the two days it touches, which is how Calendar draws it.
 */
export const spansDays = (event: CalendarEvent) =>
  event.allDay || event.endsAt - event.startsAt >= 24 * 3600;

/** One bar in a week row: the columns it covers and the lane it sits in. */
export type Span = {
  event: CalendarEvent;
  /** 0–6, the first and last column it covers in this week. */
  from: number;
  to: number;
  lane: number;
  /** Whether it carries on from the week before, or into the week after. */
  continuesLeft: boolean;
  continuesRight: boolean;
};

/**
 * The bars of one week: each all-day or multi-day event, cut to the week, stacked into the
 * lowest lane where it collides with nothing — so a three-day trip is one bar, not three.
 */
export function weekSpans(events: CalendarEvent[], weekStart: Date): Span[] {
  const weekEnd = addDays(weekStart, 6);
  const within = events
    .filter(spansDays)
    .map((event) => {
      const [first, last] = daysOf(event);
      return { event, first, last };
    })
    .filter(({ first, last }) => last >= weekStart && first <= weekEnd)
    // Longest first, then earliest: how Apple stacks them.
    .sort(
      (a, b) =>
        b.last.getTime() - b.first.getTime() - (a.last.getTime() - a.first.getTime()) ||
        a.first.getTime() - b.first.getTime() ||
        a.event.title.localeCompare(b.event.title)
    );
  const lanes: boolean[][] = [];
  const out: Span[] = [];
  for (const { event, first, last } of within) {
    const from = Math.max(0, Math.round((midnight(first).getTime() - weekStart.getTime()) / 864e5));
    const to = Math.min(6, Math.round((midnight(last).getTime() - weekStart.getTime()) / 864e5));
    let lane = 0;
    while (lanes[lane]?.slice(from, to + 1).some(Boolean)) lane++;
    (lanes[lane] ??= Array(7).fill(false));
    for (let c = from; c <= to; c++) lanes[lane][c] = true;
    out.push({
      event,
      from,
      to,
      lane,
      continuesLeft: first < weekStart,
      continuesRight: last > weekEnd,
    });
  }
  return out;
}

/** A timed event placed on a day's hour grid. */
export type Placed = {
  event: CalendarEvent;
  /** Minutes from the day's midnight to where the block starts, and how long it is. */
  top: number;
  length: number;
  /** Side-by-side columns for events that overlap, as Apple lays them out. */
  column: number;
  columns: number;
};

/** Something else on the hour grid that takes room there — a reminder due at a time. */
export type Mark = { id: string; top: number; length: number };
export type PlacedMark = Mark & { column: number; columns: number };

/**
 * The timed events of one day, placed and set side by side where they overlap.
 *
 * Events that overlap, directly or through a chain, form a cluster; each takes the first
 * column free at its start, and the whole cluster shares one column count so the blocks line
 * up. A block is never shorter than a quarter hour, or a five-minute call would be unreadable.
 */
export function placeDay(events: CalendarEvent[], day: Date): Placed[] {
  return placeDayWith(events, day, []).events;
}

/**
 * The same, with the day's reminders laid out among the events rather than over them: a
 * reminder due during a meeting sits in a column beside it, as it does in Calendar, and the
 * meeting's title stays readable.
 */
export function placeDayWith(
  events: CalendarEvent[],
  day: Date,
  marks: Mark[],
  /** The shortest a block is drawn, in minutes. Laid out shorter than it is drawn, a
   *  quarter-hour call and the one after it were set one above the other and drawn overlapping. */
  minLength = 15,
): { events: Placed[]; marks: PlacedMark[] } {
  const start = seconds(midnight(day));
  const end = seconds(addDays(day, 1));
  // Minutes on the clock, as the hour labels count them — the day's end its 24:00. Counted
  // as time gone since midnight, every block after the clock moved sat an hour off its label
  // on the two days a year it does.
  const onClock = (s: number) => {
    if (s >= end) return 24 * 60;
    const d = dateOf(s);
    return d.getHours() * 60 + d.getMinutes() + d.getSeconds() / 60;
  };
  type Item = { top: number; length: number; column: number; columns: number } & (
    | { kind: "event"; event: CalendarEvent }
    | { kind: "mark"; mark: Mark }
  );
  const items: Item[] = [
    ...events
      .filter((e) => !spansDays(e) && e.endsAt > start && e.startsAt < end)
      .map((event): Item => {
        const from = Math.max(event.startsAt, start);
        const to = Math.min(Math.max(event.endsAt, event.startsAt), end);
        const top = onClock(from);
        return { kind: "event", event, top, length: Math.max(onClock(to) - top, minLength), column: 0, columns: 1 };
      }),
    ...marks.map((mark): Item => ({ kind: "mark", mark, top: mark.top, length: mark.length, column: 0, columns: 1 })),
  ].sort((a, b) => a.top - b.top || b.length - a.length);

  let cluster: Item[] = [];
  let clusterEnd = -1;
  const close = () => {
    const columns = Math.max(1, ...cluster.map((p) => p.column + 1));
    for (const p of cluster) p.columns = columns;
    cluster = [];
  };
  for (const item of items) {
    if (item.top >= clusterEnd && cluster.length) close();
    const busy = cluster.filter((p) => p.top + p.length > item.top).map((p) => p.column);
    let column = 0;
    while (busy.includes(column)) column++;
    item.column = column;
    cluster.push(item);
    clusterEnd = Math.max(clusterEnd, item.top + item.length);
  }
  if (cluster.length) close();

  const placed: Placed[] = [];
  const placedMarks: PlacedMark[] = [];
  for (const p of items) {
    const { top, length, column, columns } = p;
    if (p.kind === "event") placed.push({ event: p.event, top, length, column, columns });
    else placedMarks.push({ ...p.mark, top, length, column, columns });
  }
  return { events: placed, marks: placedMarks };
}

/** Snap a number of minutes to the nearest quarter hour, as dragging in Calendar does. */
export const snap = (minutes: number, step = 15) => Math.round(minutes / step) * step;

/** "9 AM", "Noon", "3 PM" — the hour labels in the viewer's own clock. */
export function hourLabel(hour: number, twelve = TWELVE_HOUR): string {
  // "Noon" is a twelve-hour clock's word; a 24-hour one writes 12:00 like every other hour.
  if (twelve && hour === 12) return "Noon";
  const d = new Date(2024, 0, 1, hour);
  return twelve ? timeText(d, { hour: "numeric" }) : timeText(d, { hour: "2-digit", minute: "2-digit" });
}

/** "9:30 AM", with ":00" left off, as Calendar writes a time — on a twelve-hour clock. A
 *  24-hour one keeps it: "14:00", never a bare "14". */
export const clock = (s: number, twelve = TWELVE_HOUR) => {
  const text = timeText(dateOf(s), { hour: "numeric", minute: "2-digit" });
  return twelve ? text.replace(/:00(?=\D|$)/, "") : text;
};

/* ---- time zones ------------------------------------------------------------------------ */

/** A clock reading: what a wall clock in some zone shows. */
export type Wall = { y: number; m: number; d: number; h: number; min: number };

const clocks = new Map<string, Intl.DateTimeFormat>();
function clockIn(zone: string): Intl.DateTimeFormat {
  let f = clocks.get(zone);
  if (!f) {
    f = new Intl.DateTimeFormat("en-US", {
      timeZone: zone, hourCycle: "h23",
      year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric",
    });
    clocks.set(zone, f);
  }
  return f;
}

/** What a clock in `zone` shows at the instant `s` (seconds). */
export function wallIn(zone: string, s: number): Wall {
  const parts: Record<string, number> = {};
  for (const p of clockIn(zone).formatToParts(new Date(s * 1000))) {
    if (p.type !== "literal") parts[p.type] = Number(p.value);
  }
  return { y: parts.year, m: parts.month, d: parts.day, h: parts.hour % 24, min: parts.minute };
}

/**
 * The instant a clock in `zone` shows `w`. A reading a clock skips (the hour lost in spring)
 * comes out an hour on, as Calendar moves one; one a clock shows twice, the first time.
 */
export function instantIn(zone: string, w: Wall): number {
  const asUtc = Date.UTC(w.y, w.m - 1, w.d, w.h, w.min) / 1000;
  const offsetAt = (s: number) => {
    const c = wallIn(zone, s);
    return Date.UTC(c.y, c.m - 1, c.d, c.h, c.min) / 1000 - s;
  };
  // The two offsets either side of any clock change near it: a reading shown under both is
  // taken the first time; one shown under neither (skipped) is taken an hour on.
  const tries = [asUtc - offsetAt(asUtc - 86400), asUtc - offsetAt(asUtc + 86400)];
  const same = (s: number) => {
    const c = wallIn(zone, s);
    return c.y === w.y && c.m === w.m && c.d === w.d && c.h === w.h && c.min === w.min;
  };
  const shown = tries.filter(same);
  return shown.length ? Math.min(...shown) : Math.max(...tries);
}

/** An instant's clock reading moved to another zone: 9:00 in one is 9:00 in the other. */
export function sameClockIn(from: string, to: string, s: number): number {
  return instantIn(to, wallIn(from, s));
}

/** Every zone this computer knows, by its IANA name. */
export function allZones(): string[] {
  const known = (Intl as { supportedValuesOf?: (k: string) => string[] }).supportedValuesOf?.("timeZone") ?? [];
  return known.length ? known : ["UTC"];
}

/** A zone as Calendar names one: its city, and how far from GMT it is at `s`. */
export function zoneName(zone: string, s: number): { city: string; offset: string } {
  const city = (zone.split("/").pop() ?? zone).replace(/_/g, " ");
  const w = wallIn(zone, s);
  const minutes = Math.round((Date.UTC(w.y, w.m - 1, w.d, w.h, w.min) / 1000 - Math.floor(s / 60) * 60) / 60);
  const sign = minutes < 0 ? "−" : "+";
  const a = Math.abs(minutes);
  const offset = minutes === 0 ? "GMT" : `GMT${sign}${Math.floor(a / 60)}${a % 60 ? `:${String(a % 60).padStart(2, "0")}` : ""}`;
  return { city, offset };
}

/** `s` moved `days` calendar days in `zone`, at the same clock time — never a fixed 86,400
 * seconds, which landed an hour off (an all-day event on the day before) across a clock change. */
export function addDaysIn(zone: string, s: number, days: number): number {
  const w = wallIn(zone, s);
  const d = new Date(Date.UTC(w.y, w.m - 1, w.d + days));
  return instantIn(zone, { y: d.getUTCFullYear(), m: d.getUTCMonth() + 1, d: d.getUTCDate(), h: w.h, min: w.min });
}

/**
 * The end a timed event takes when its end is set to `end`: an end before the start is the
 * next day's clock, as Calendar reads it, so 23:00 to "1:00" runs past midnight. It was
 * clamped to five minutes after the start, and the event became 23:00 to 23:05 (desktop audit
 * M12). Only an end still before the start a day on is clamped, as before.
 */
export function endAfter(zone: string, startsAt: number, end: number): number {
  const rolled = end < startsAt ? addDaysIn(zone, end, 1) : end;
  return Math.max(rolled, startsAt + 300);
}

/**
 * The number of the week starting `weekStart`, as Calendar shows them: ISO 8601's where weeks
 * start on Monday (the week holding the year's first Thursday is 1), and elsewhere the week
 * holding 1 January is 1 — the US way, and the way a Sunday- or Saturday-first calendar counts.
 */
export function weekNumber(weekStart: Date, firstDay = WEEK.firstDay): number {
  const start = midnight(weekStart);
  if (firstDay === 1) {
    const thursday = addDays(start, (4 - start.getDay() + 7) % 7);
    const jan1 = new Date(thursday.getFullYear(), 0, 1);
    return Math.floor(Math.round((thursday.getTime() - jan1.getTime()) / 864e5) / 7) + 1;
  }
  // The week's last day decides the year it belongs to: the week holding 1 January is week 1.
  const last = addDays(start, 6);
  const firstWeek = startOfWeek(new Date(last.getFullYear(), 0, 1), firstDay);
  return Math.round((start.getTime() - firstWeek.getTime()) / (7 * 864e5)) + 1;
}

/** A video call an event holds a link to, and the service it is on. */
export type Meeting = { href: string; service: string };

// Each service by the shape of its join link, never by a word in the text: a link only counts
// when its host and path are the service's own. Hosts are matched whole (a subdomain may lead,
// as `company.zoom.us`), so `zoom.us.example.com` or `notzoom.us` are never taken for Zoom.
const MEETINGS: { service: string; host: RegExp; path: RegExp }[] = [
  { service: "Zoom", host: /^([a-z0-9-]+\.)*zoom\.(us|com)$/, path: /^\/(j|my|w|wc\/join)\/[\w.-]+/ },
  { service: "Google Meet", host: /^meet\.google\.com$/, path: /^\/[a-z]{3}-[a-z]{4}-[a-z]{3}\b/ },
  { service: "Microsoft Teams", host: /^teams\.(microsoft|live)\.com$/, path: /^\/(l\/meetup-join|meet)\// },
  { service: "Webex", host: /^([a-z0-9-]+\.)*webex\.com$/, path: /^\/(meet|join|wbxmjs|[\w-]+\/j\.php)\b/ },
  { service: "FaceTime", host: /^facetime\.apple\.com$/, path: /^\/join\b/ },
];

/**
 * The video call to join from an event: the first service link in its URL field, then its
 * location, then its notes — where calendars and invitations put them. Only `https:` links,
 * trailing punctuation left off, nothing guessed: no link of a known service, no button.
 */
export function meetingOf(e: { url?: string | null; location?: string | null; notes?: string | null }): Meeting | null {
  for (const text of [e.url, e.location, e.notes]) {
    if (!text) continue;
    for (const raw of text.match(/https:\/\/[^\s<>"'()]+/gi) ?? []) {
      const link = raw.replace(/[.,;:!?)\]}>]+$/, "");
      let url: URL;
      try {
        url = new URL(link);
      } catch {
        continue;
      }
      const host = url.hostname.toLowerCase();
      const found = MEETINGS.find((m) => m.host.test(host) && m.path.test(url.pathname));
      if (found) return { href: url.toString(), service: found.service };
    }
  }
  return null;
}
