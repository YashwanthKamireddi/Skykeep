/**
 * Dates and times written out in this machine's language, with each kind of format built
 * once and kept.
 *
 * `Date.toLocaleDateString(undefined, options)` builds a new formatter on every call, and a
 * formatter is not cheap — the locale's data is looked up and compiled each time. A calendar
 * week writes out hundreds of dates and times on every redraw, and those builds were most of
 * what made opening a new event stutter. A formatter made once per set of options and kept
 * gives the same text for a fraction of the cost.
 */
import { useEffect, useState } from "react";

const made = new Map<string, Intl.DateTimeFormat>();

function formatter(options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = JSON.stringify(options);
  let f = made.get(key);
  if (!f) {
    f = new Intl.DateTimeFormat(undefined, options);
    made.set(key, f);
  }
  return f;
}

/** As `date.toLocaleDateString(undefined, options)`, without building a formatter. */
export function dateText(date: Date, options: Intl.DateTimeFormatOptions): string {
  return formatter(options).format(date);
}

/** As `date.toLocaleTimeString(undefined, options)`, without building a formatter. */
export function timeText(date: Date, options: Intl.DateTimeFormatOptions): string {
  return formatter(options).format(date);
}

/** The local midnight after `t`. */
function nextMidnight(t: number): number {
  const d = new Date(t);
  d.setHours(24, 0, 0, 0);
  return d.getTime();
}

/** The local midnight that began the day `t` is in: the same number all day, for memo keys. */
export function dayOf(t: number): number {
  const d = new Date(t);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/**
 * The time, moved on only when what a screen says about it can change: at midnight, when
 * "Tomorrow" becomes "Today" and today's notes become yesterday's, as the next of `moments`
 * passes (a reminder on screen falls due and turns red), and when the window comes back after
 * the machine slept past either. Labels worked out from `new Date()` inside memoised rows
 * otherwise stayed as they were drawn: "Tomorrow" after midnight, an overdue reminder never
 * red (desktop audit M4). One timer, not a tick a minute that would redraw a whole list.
 */
export function useClock(moments?: readonly number[]): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    let at = nextMidnight(now);
    for (const m of moments ?? []) if (m > now && m < at) at = m;
    const wait = Math.min(Math.max(0, at - Date.now()) + 50, 2 ** 31 - 1);
    const timer = window.setTimeout(() => setNow(Date.now()), wait);
    const back = () => {
      if (document.visibilityState === "visible" && Date.now() >= at) setNow(Date.now());
    };
    document.addEventListener("visibilitychange", back);
    window.addEventListener("focus", back);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", back);
      window.removeEventListener("focus", back);
    };
  }, [now, moments]);
  return now;
}
