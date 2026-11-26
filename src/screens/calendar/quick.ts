/**
 * Quick Event, as Calendar reads one typed in a line: "Lunch with Asha tomorrow 1pm",
 * "Gym every Monday 7am", "Flight Fri 6:30–9". Read here, on this computer, word by word —
 * what names a day, a time, a length, a repeat or a place is taken out, and what is left is
 * the title.
 *
 * English only, as the words are. Anything not understood stays in the title, which the
 * preview shows before anything is saved.
 */
import type { EventRepeat } from "../../lib/ipc";

type Quick = {
  title: string;
  location: string | null;
  /** Local times, unix seconds. An all-day event runs from midnight to the next. */
  startsAt: number;
  endsAt: number;
  allDay: boolean;
  repeat: EventRepeat | null;
  /** Whether a day or a time was read at all — with neither, it is the next hour today. */
  dated: boolean;
  timed: boolean;
};

const WEEKDAYS = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];
const MONTHS = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
/** "mon", "tues", "thurs" and the rest, each to its day. */
const WEEKDAY_WORD = String.raw`(sun(?:day)?|mon(?:day)?|tue(?:s(?:day)?)?|wed(?:nesday)?|thu(?:r(?:s(?:day)?)?)?|fri(?:day)?|sat(?:urday)?)`;
const MONTH_WORD = String.raw`(jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)`;
const weekdayOf = (w: string) => WEEKDAYS.findIndex((d) => d.startsWith(w.slice(0, 3)));
const monthOf = (w: string) => MONTHS.findIndex((m) => m.startsWith(w.slice(0, 3)));

/** A clock time: "1pm", "1:30 pm", "13:00", "7", "noon", "midnight". */
const TIME = String.raw`(noon|midday|midnight|\d{1,2}(?::\d{2}|\.\d{2})?\s*(?:a\.?m\.?|p\.?m\.?)?)`;
const DASH = String.raw`\s*(?:-|–|—|to|till|until)\s*`;

/** `exact`: an hour written with its leading zero ("06:15"), a 24-hour clock's, read as written. */
type Clock = { h: number; m: number; meridiem: "am" | "pm" | null; exact?: boolean };

function clockOf(raw: string): Clock | null {
  const t = raw.trim().toLowerCase();
  if (t === "noon" || t === "midday") return { h: 12, m: 0, meridiem: "pm" };
  if (t === "midnight") return { h: 0, m: 0, meridiem: "am" };
  const m = /^(\d{1,2})(?:[:.](\d{2}))?\s*(a\.?m\.?|p\.?m\.?)?$/.exec(t);
  if (!m) return null;
  const h = Number(m[1]);
  const min = m[2] ? Number(m[2]) : 0;
  if (min > 59) return null;
  const meridiem = m[3] ? (m[3][0] === "a" ? "am" : "pm") : null;
  if (meridiem ? h < 1 || h > 12 : h > 23) return null;
  return { h, m: min, meridiem, ...(!meridiem && m[1].length === 2 && m[1][0] === "0" ? { exact: true } : {}) };
}

/** Hours on a 24-hour clock. A bare hour with no am or pm is read as people mean one: 1 to 6
 * in the afternoon, 7 to 11 in the morning — "lunch at 1", "gym at 7". */
function hours24(c: Clock, guess: "am" | "pm" | null = null): number {
  if (c.exact) return c.h;
  const meridiem = c.meridiem ?? (c.h > 12 || c.h === 0 ? null : guess ?? (c.h >= 1 && c.h <= 6 ? "pm" : c.h === 12 ? "pm" : "am"));
  if (!meridiem) return c.h;
  if (meridiem === "am") return c.h === 12 ? 0 : c.h;
  return c.h === 12 ? 12 : c.h + 12;
}

const midnightOf = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
const plusDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
const secondsOf = (d: Date) => Math.floor(d.getTime() / 1000);

/** The next `weekday` after today (today itself only when `orToday`). */
function nextWeekday(today: Date, weekday: number, orToday: boolean) {
  const ahead = (weekday - today.getDay() + 7) % 7;
  return plusDays(today, ahead === 0 && !orToday ? 7 : ahead);
}

/** Whether a numeric date is written day first here ("24/10"), or month first ("10/24"). */
function dayFirst(): boolean {
  try {
    const parts = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "numeric" }).formatToParts(new Date(2026, 9, 24));
    return parts.findIndex((p) => p.type === "day") < parts.findIndex((p) => p.type === "month");
  } catch {
    return true;
  }
}

/** `lengths`: whether "for an hour" is read as how long it lasts — an event's, not a
 *  reminder's, which has no length and keeps the words. */
export function parseQuick(text: string, now: Date = new Date(), dmy: boolean = dayFirst(), lengths = true): Quick {
  let rest = ` ${text.replace(/\s+/g, " ").trim()} `;
  const today = midnightOf(now);
  let day = null as Date | null;
  let start = null as Clock | null;
  let end = null as Clock | null;
  let minutes = null as number | null;
  let repeat = null as EventRepeat | null;
  let location = null as string | null;
  /** "Tonight": a bare hour is the evening's, and with none it is seven. */
  let evening = false;

  /** Take the first match of `re` out of what is left, and hand its groups to `use`. */
  const take = (re: RegExp, use: (m: RegExpExecArray) => boolean | void) => {
    const m = re.exec(rest);
    if (!m) return false;
    if (use(m) === false) return false;
    rest = `${rest.slice(0, m.index)} ${rest.slice(m.index + m[0].length)}`;
    return true;
  };
  const w = (body: string) => new RegExp(String.raw`(?<=\s)${body}(?=[\s,.!?]|$)`, "i");

  // Repeats: "every day", "daily", "every 2 weeks", "every Monday", "weekly", "every year".
  take(w(String.raw`(?:every|each)\s+` + WEEKDAY_WORD), (m) => {
    repeat = { freq: "weekly", interval: 1, count: null };
    day ??= nextWeekday(today, weekdayOf(m[1].toLowerCase()), true);
  }) ||
    take(w(String.raw`(?:every|each)\s+(?:(other|\d+)\s+)?(day|week|month|year)s?`), (m) => {
      const interval = m[1] ? (m[1].toLowerCase() === "other" ? 2 : Number(m[1])) : 1;
      if (!(interval >= 1 && interval <= 99)) return false;
      repeat = { freq: `${m[2].toLowerCase() === "day" ? "dai" : m[2].toLowerCase()}ly` as EventRepeat["freq"], interval, count: null };
    }) ||
    take(w(String.raw`(daily|weekly|monthly|yearly|annually)`), (m) => {
      const word = m[1].toLowerCase();
      repeat = { freq: (word === "annually" ? "yearly" : word) as EventRepeat["freq"], interval: 1, count: null };
    });

  // Days — and "by" one, as a reminder is due by a day: the word goes with the day.
  take(w(String.raw`(?:(?:on|by)\s+)?(\d{4})-(\d{1,2})-(\d{1,2})`), (m) => {
    const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
    if (d.getMonth() !== Number(m[2]) - 1) return false;
    day = d;
  }) ||
    take(w(String.raw`(?:(?:on|by)\s+)?(?:the\s+)?(\d{1,2})(?:st|nd|rd|th)?(?:\s+of)?\s+${MONTH_WORD}(?:,?\s+(\d{4}))?`), (m) => {
      day = dateIn(Number(m[1]), monthOf(m[2].toLowerCase()), m[3] ? Number(m[3]) : null, today);
      if (!day) return false;
    }) ||
    take(w(String.raw`(?:(?:on|by)\s+)?${MONTH_WORD}\.?\s+(\d{1,2})(?:st|nd|rd|th)?(?:,?\s+(\d{4}))?`), (m) => {
      day = dateIn(Number(m[2]), monthOf(m[1].toLowerCase()), m[3] ? Number(m[3]) : null, today);
      if (!day) return false;
    }) ||
    take(w(String.raw`(?:(?:on|by)\s+)?(\d{1,2})/(\d{1,2})(?:/(\d{2,4}))?`), (m) => {
      const [a, b] = [Number(m[1]), Number(m[2])];
      const year = m[3] ? (m[3].length === 2 ? 2000 + Number(m[3]) : Number(m[3])) : null;
      day = dmy ? dateIn(a, b - 1, year, today) : dateIn(b, a - 1, year, today);
      if (!day) return false;
    }) ||
    take(w(String.raw`(?:the\s+)?day\s+after\s+tomorrow`), () => { day = plusDays(today, 2); }) ||
    take(w(String.raw`(?:by\s+)?(today|tonight|tomorrow|tmrw|tmr|yesterday)`), (m) => {
      const word = m[1].toLowerCase();
      day = plusDays(today, word === "yesterday" ? -1 : word.startsWith("tom") || word.startsWith("tm") ? 1 : 0);
      if (word === "tonight") evening = true;
    }) ||
    take(w(String.raw`in\s+(\d{1,3}|a|one|two|three)\s+(day|week)s?`), (m) => {
      const n = { a: 1, one: 1, two: 2, three: 3 }[m[1].toLowerCase()] ?? Number(m[1]);
      day = plusDays(today, n * (m[2].toLowerCase() === "week" ? 7 : 1));
    }) ||
    take(w(String.raw`(?:(next|this|on|by)\s+)?${WEEKDAY_WORD}`), (m) => {
      const target = weekdayOf(m[2].toLowerCase());
      const which = m[1]?.toLowerCase();
      // "next Friday" is the Friday of next week when said early in the week, as people mean it.
      let d = nextWeekday(today, target, which === "this");
      if (which === "next" && (d.getTime() - today.getTime()) / 864e5 < 7 && target > today.getDay() && today.getDay() !== 0) d = plusDays(d, 7);
      day = d;
    }) ||
    take(w(String.raw`next\s+week`), () => { day = nextWeekday(today, 1, false); });

  // Times: a range, then one time, then a length.
  const atTime = String.raw`(?:at\s+|@\s*|from\s+)?`;
  take(w(atTime + TIME + DASH + TIME), (m) => {
    const a = clockOf(m[1]);
    const b = clockOf(m[2]);
    // "6:30–9" needs a number with a colon or am/pm somewhere, or "1-2" in a title is a range.
    if (!a || !b || !(/[:.apn]|noon|mid/i.test(m[0]) || /^(at|@|from)/i.test(m[0].trim()))) return false;
    start = a;
    end = b;
  }) ||
    take(w(String.raw`(?:at|@|by\s)\s*` + TIME), (m) => {
      const c = clockOf(m[1]);
      if (!c) return false;
      start = c;
    }) ||
    take(w(String.raw`(noon|midday|midnight|\d{1,2}(?:[:.]\d{2})?\s*(?:a\.?m\.?|p\.?m\.?)|\d{1,2}[:.]\d{2})`), (m) => {
      const c = clockOf(m[1]);
      if (!c) return false;
      start = c;
    });
  if (lengths) take(w(String.raw`for\s+(\d+(?:\.\d+)?|an?|half\s+an?)\s*(minutes?|mins?|m|hours?|hrs?|h)`), (m) => {
    const n = /half/i.test(m[1]) ? 0.5 : /^an?$/i.test(m[1]) ? 1 : Number(m[1]);
    minutes = Math.round(n * (/^h/i.test(m[2]) ? 60 : 1));
    if (!(minutes > 0 && minutes <= 14 * 24 * 60)) return false;
  });

  // A place: what follows "at" or "@" that is not a time.
  take(/\s(?:at|@)\s+([^,]+?)\s*$/i, (m) => {
    if (clockOf(m[1])) return false;
    location = m[1].trim();
  });

  const title = rest.replace(/\s+/g, " ").replace(/\s+([,.!?])/g, "$1").replace(/^[\s,–—-]+|[\s,–—-]+$/g, "").trim();
  if (evening) start = start ? { ...start, meridiem: start.meridiem ?? (start.h < 12 ? "pm" : null) } : { h: 7, m: 0, meridiem: "pm" };
  const dated = day !== null;
  const timed = start !== null;
  const base: Date = day ?? today;
  let startsAt: number;
  let endsAt: number;
  let allDay = false;
  if (start) {
    const s: Clock = start;
    // A bare start with an am or pm only at the end is read on the end's side where that puts
    // it first ("6-7am" is six in the morning, "7-9pm" seven in the evening), else on the other
    // ("11-1pm" from eleven in the morning). Guessed alone, "6-7am" began at 6 PM and the whole
    // event moved on a day.
    const e0: Clock | null = end;
    let h = hours24(s);
    if (e0 && !s.meridiem && e0.meridiem) {
      const eh0 = hours24(e0);
      const side = hours24(s, e0.meridiem);
      h = side * 60 + s.m < eh0 * 60 + e0.m ? side : hours24(s, e0.meridiem === "am" ? "pm" : "am");
    }
    const from = new Date(base.getFullYear(), base.getMonth(), base.getDate(), h, s.m);
    // Today, a time already gone is tomorrow's — unless a day was named.
    if (!dated && from.getTime() < now.getTime() - 60_000) from.setDate(from.getDate() + 1);
    startsAt = secondsOf(from);
    if (end) {
      const e: Clock = end;
      // The end is read on the start's side of noon when it can be: "1-2pm" is 1 to 2 PM.
      const guess: "am" | "pm" = e.meridiem ?? (h >= 12 ? "pm" : "am");
      let eh = hours24(e, guess);
      if (!e.meridiem && eh < 12 && eh * 60 + e.m <= h * 60 + s.m) eh += 12;
      const to = new Date(from.getFullYear(), from.getMonth(), from.getDate(), eh, e.m);
      if (to.getTime() <= from.getTime()) to.setDate(to.getDate() + 1);
      endsAt = secondsOf(to);
      // "1pm-2" meant the start's side too; "11am–1" crosses noon.
      if (!s.meridiem && e.meridiem && endsAt - startsAt > 12 * 3600) startsAt += 12 * 3600;
    } else {
      endsAt = startsAt + (minutes ?? 60) * 60;
    }
  } else if (dated) {
    allDay = minutes === null;
    const d = base;
    startsAt = secondsOf(allDay ? d : new Date(d.getFullYear(), d.getMonth(), d.getDate(), 9));
    endsAt = allDay ? secondsOf(plusDays(d, 1)) : startsAt + (minutes ?? 60) * 60;
  } else {
    // Nothing said of when: the next whole hour, as a new event is.
    const next = new Date(now.getFullYear(), now.getMonth(), now.getDate(), now.getHours() + 1);
    startsAt = secondsOf(next);
    endsAt = startsAt + (minutes ?? 60) * 60;
  }
  return { title, location, startsAt, endsAt, allDay, repeat, dated, timed };
}

/** A day of a month, this year or the next when it has already gone (a year named, that one). */
function dateIn(day: number, month: number, year: number | null, today: Date): Date | null {
  if (month < 0 || month > 11 || day < 1 || day > 31) return null;
  const y = year ?? today.getFullYear();
  let d = new Date(y, month, day);
  if (d.getMonth() !== month) return null;
  if (year === null && d < today) d = new Date(y + 1, month, day);
  return d;
}
