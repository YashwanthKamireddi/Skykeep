import { toast, dismissToast, useErrorToast } from "../../components/Toast";
import { AppFoot } from "../../components/AppFoot";
import { REFUSED_LINK, outsideHref } from "../../lib/links";
import { follow } from "../../lib/follow";
import { ConfirmSheet, useConfirm } from "../../components/Confirm";
import { EmptyState } from "../../components/EmptyState";
import { SearchField } from "../../components/SearchField";
import { useActiveRef } from "../../lib/screen";
import { usePending } from "../../lib/pending";
import { inMenu, Portal } from "../../components/Portal";
import { useReturnFocus } from "../../lib/focus";
import { TimeField } from "../../components/TimeField";
import { ChevronDown, ChevronLeft, ChevronRight, Ellipsis, MapPin, Plus, Repeat, Video } from "lucide-react";
import { createContext, Fragment, memo, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  type EventRepeat,
  type Guest,
  api,
  defaultCalendar,
  messageOf,
  type Calendar,
  type CalendarEvent,
  type Reminder, type EventScope,
  type CalendarSubscription,
  calendarFeedsApi,
  SUBSCRIPTION_PREFIX } from "../../lib/ipc";
import { parseQuick } from "./quick";
import { useShownRevision } from "../../lib/sync";
import { useOutbox } from "../../lib/outbox";
import { Unsent, Waiting } from "../../components/Unsent";
import { Picker } from "../../components/Picker";
import { RowMenu } from "../../components/RowMenu";
import {
  addDays,
  addDaysIn,
  allZones,
  clock,
  dateOf,
  dayKey,
  daysOf,
  hourLabel,
  instantIn,
  midnight,
  isWeekend,
  monthDays,
  onWeekChange,
  placeDayWith,
  sameClockIn,
  sameDay,
  seconds,
  snap,
  spansDays,
  startOfWeek,
  wallIn,
  weekNumber,
  weekSpans,
  zoneName,
  meetingOf,
} from "./model";
import { dateText, timeText } from "../../lib/when";
import { reveal, useReveal } from "../../lib/reveal";

const ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";

const REPEATS = [
  { value: "never", label: "Never" },
  { value: "daily", label: "Every Day" },
  { value: "weekly", label: "Every Week" },
  { value: "monthly", label: "Every Month" },
  { value: "yearly", label: "Every Year" },
];
function repeatKey(r: EventRepeat | null): string {
  if (!r) return "never";
  return r.interval === 1 && r.count === null ? r.freq : "custom";
}
function repeatText(r: EventRepeat | null): string {
  if (!r) return "Never";
  const unit = { daily: "day", weekly: "week", monthly: "month", yearly: "year" }[r.freq];
  const every = r.interval === 1 ? `Every ${unit}` : `Every ${r.interval} ${unit}s`;
  return r.count ? `${every}, ${r.count} times` : every;
}

/** icloud.com's alert presets, minutes before the start; an all-day event's are by the day. */
const ALERTS = [
  { minutes: 0, label: "At time of event" },
  { minutes: 5, label: "5 minutes before" },
  { minutes: 15, label: "15 minutes before" },
  { minutes: 30, label: "30 minutes before" },
  { minutes: 60, label: "1 hour before" },
  { minutes: 120, label: "2 hours before" },
  { minutes: 1440, label: "1 day before" },
  { minutes: 2880, label: "2 days before" },
];
/** Apple's all-day alerts ring at 9 AM: minutes from the day's midnight, negative after it. */
const ALL_DAY_ALERTS = [
  { minutes: -540, label: "On day of event (9 AM)" },
  { minutes: 900, label: "1 day before (9 AM)" },
  { minutes: 2340, label: "2 days before (9 AM)" },
  { minutes: 9540, label: "1 week before (9 AM)" },
];
/** The midnight presets kept before, read as the 9 AM ones they meant. */
const ALL_DAY_WAS: Record<number, number> = { 0: -540, 1440: 900, 2880: 2340, 10080: 9540 };
/** An alert Apple holds that no preset names — set on a phone, say. */
function alertLabel(m: number): string {
  if (m === 0) return "At time of event";
  const after = m < 0;
  const a = Math.abs(m);
  const text = a % 1440 === 0 ? `${a / 1440} day${a === 1440 ? "" : "s"}`
    : a % 60 === 0 ? `${a / 60} hour${a === 60 ? "" : "s"}` : `${a} minutes`;
  return `${text} ${after ? "after" : "before"}`;
}

/** Apple's four scopes, in Apple's order. */
const VIEWS = [
  { key: "day", label: "Day" },
  { key: "week", label: "Week" },
  { key: "month", label: "Month" },
  { key: "year", label: "Year" },
] as const;
type View = (typeof VIEWS)[number]["key"];

/** A day with nothing in it, the same empty list every time. */
const NOTHING: never[] = [];

/** One hour of the Day and Week grids. */
const HOUR = 48;
/** A bar in the all-day row, and the gap under it: 22px and 2px, so each bar is a target
 * 24px from the next (WCAG 2.2). */
const LANE = 24;
/** A line in a month cell, on the same 24px pitch. */
const LINE = 24;
/** Narrower than this, a month cell draws its items as dots. */
const DOT_CELL = 72;
/** The most dots one row of a narrow cell holds; past that the day is simply busy. */
const MOST_DOTS = 4;

/** What a month cell shows before it says "N more". */
const MONTH_ROWS = 4;

/** Calendars the viewer has switched off here, remembered on this computer. */
const HIDDEN_KEY = "quince.calendar.hidden";
const VIEW_KEY = "quince.calendar.view";
/** Sidebar groups folded away, week numbers shown, and each calendar's default alerts —
 *  Apple keeps these as preferences of each Mac, and so does this. */
const FOLDED_KEY = "quince.calendar.folded";
const WEEK_NUMBERS_KEY = "quince.calendar.weekNumbers";
const DEFAULT_ALERTS_KEY = "quince.calendar.defaultAlerts";

type DefaultAlerts = { timed: Record<string, number>; allDay: Record<string, number> };
const readJson = <T,>(key: string, fallback: T): T => {
  try {
    const raw = window.localStorage.getItem(key);
    return raw ? { ...fallback, ...JSON.parse(raw) } : fallback;
  } catch {
    return fallback;
  }
};
const writeJson = (key: string, value: unknown) => {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // A preference of this computer; kept for now if it cannot be written.
  }
};

/** How often a subscription can be fetched again, as Calendar offers it. */
const REFRESHES = [
  { minutes: 60, label: "Every Hour" },
  { minutes: 1440, label: "Every Day" },
  { minutes: 10080, label: "Every Week" },
  { minutes: 0, label: "Only When Asked" },
];

/** A place, for the map app this computer opens `geo:` links with. */
const mapsHref = (place: string) => `geo:0,0?q=${encodeURIComponent(place.split("\n").join(", "))}`;

/** An event's guests and their answers, as Calendar lists them: read from Apple when the event
 *  opens; nothing shown for an event with none, or before the answer comes. */
function Guests({ guid }: { guid: string | null | undefined }) {
  const [guests, setGuests] = useState<Guest[]>([]);
  useEffect(() => {
    setGuests([]);
    if (!guid || guid.startsWith("quince:")) return;
    let live = true;
    api.eventGuests(guid).then((g) => live && setGuests(g), () => {});
    return () => {
      live = false;
    };
  }, [guid]);
  if (guests.length === 0) return null;
  const answer = { accepted: "Accepted", declined: "Declined", tentative: "Maybe", pending: "No reply" } as const;
  const glyph = { accepted: "✓", declined: "✕", tentative: "?", pending: "" } as const;
  return (
    <section className="cx-guests" aria-label="Guests">
      <h4 className="cx-guests-head">
        {guests.length} {guests.length === 1 ? "Guest" : "Guests"}
      </h4>
      <ul className="cx-guest-list">
        {guests.map((g, i) => {
          const name = g.name ?? g.email ?? "Guest";
          const role = g.isOrganizer ? "Organizer" : g.isMe ? "You" : null;
          return (
            <li key={`${g.email ?? g.name ?? ""}-${i}`} className="cx-guest" title={g.email ?? undefined}>
              <span className={`cx-guest-mark is-${g.status}`} aria-hidden="true">{glyph[g.status]}</span>
              <span className="cx-guest-name">
                {name}
                {role && <span className="cx-guest-role"> · {role}</span>}
              </span>
              <span className="cx-guest-answer">{answer[g.status]}</span>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/** Join, for an event with a video call: the call's own link, opened in its app or browser. */
function JoinButton({ event }: { event: { url?: string | null; location?: string | null; notes?: string | null } }) {
  const meeting = useMemo(() => meetingOf(event), [event.url, event.location, event.notes]);
  if (!meeting) return null;
  return (
    <a className="cx-join" href={meeting.href} target="_blank" rel="noreferrer" title={meeting.href}>
      <Video size={14} strokeWidth={2} aria-hidden="true" />
      Join {meeting.service}
    </a>
  );
}

const readSet = (key: string) => {
  try {
    return new Set<string>(JSON.parse(window.localStorage.getItem(key) ?? "[]"));
  } catch {
    return new Set<string>();
  }
};

/** An event being written. A guid means it already exists at Apple. */
type Draft = {
  guid: string | null;
  calendarGuid: string;
  title: string;
  location: string;
  startsAt: number;
  endsAt: number;
  allDay: boolean;
  notes: string;
  /** Carried through untouched: the editor has no field for it yet, and sending null wiped
   *  the link off every event that was edited, moved or resized here. */
  url: string | null;
  /** The zone the event was made in. Sending this machine's instead moved a meeting set in
   *  New York to local time on every edit, on every device. New events take this machine's. */
  timezone: string | null;
  /** The alerts, minutes before the start; `undefined` until read from Apple — and while
   *  undefined a save leaves them as they are, so a slow read can never clear them. */
  alerts?: number[];
  /** Whether the alerts were changed here, and so are to be written. */
  alertsChanged?: boolean;
  /** How it repeats; `undefined` until read from Apple, `null` for never. */
  repeat?: EventRepeat | null;
  repeatChanged?: boolean;
  /** A new event's alerts are its calendar's defaults until one is chosen, and follow a
   *  change of calendar or of all-day. */
  alertsDefault?: boolean;
  /** The event as it was when this draft was made from it, Apple's tag with it: what a save
   *  of it is made from, so a change another device made meanwhile is merged, not overwritten. */
  opened?: CalendarEvent;
};

/**
 * A reminder's ring on the calendar is its checkbox, as in Calendar: ticking it completes the
 * reminder there and then. It fills at once, lingers a moment so the tick is seen, and leaves
 * the schedule — Calendar shows no completed reminders.
 */
const TickContext = createContext<{
  ticked: Set<string>;
  tick: (r: Reminder) => void;
  colour: (r: Reminder) => string;
  /** Open a reminder's card beside where it was clicked: its whole title, when, list and notes. */
  show: (r: Reminder, at: Anchor) => void;
} | null>(null);
/** A reminder's list colour, as Calendar on the Mac draws scheduled reminders in it. */
const useListColour = () => {
  const ctx = useContext(TickContext);
  return (r: Reminder) => ({ "--cal": ctx?.colour(r) ?? "var(--sys-blue)" }) as React.CSSProperties;
};
/**
 * What makes a scheduled reminder open its card, as Calendar's do: a click, or Return or Space
 * on it — and its whole title on hover, since a cell or a narrow column cuts it short. The line
 * is the one control here for the keyboard and screen readers; its ring answers the mouse
 * only (`quiet`), and is in the card for everyone, so no control sits inside another.
 */
const useShowReminder = () => {
  const ctx = useContext(TickContext);
  const open = (r: Reminder, el: HTMLElement, y?: number) => {
    const box = el.getBoundingClientRect();
    ctx?.show(r, { x: box.right, y: y ?? box.top + box.height / 2, left: box.left });
  };
  return (r: Reminder) => ({
    role: "button",
    tabIndex: 0,
    title: r.title,
    "aria-label": r.title,
    // Its own press, not the day's or the timeline's: never a new event or a drag begun under it.
    onMouseDown: (e: React.MouseEvent<HTMLElement>) => e.stopPropagation(),
    onPointerDown: (e: React.PointerEvent<HTMLElement>) => e.stopPropagation(),
    onClick: (e: React.MouseEvent<HTMLElement>) => {
      e.stopPropagation();
      open(r, e.currentTarget, e.clientY);
    },
    onKeyDown: (e: React.KeyboardEvent<HTMLElement>) => {
      if (e.target !== e.currentTarget || (e.key !== "Enter" && e.key !== " ")) return;
      e.preventDefault();
      open(r, e.currentTarget);
    },
  });
};

function Ring({ reminder, quiet = false }: { reminder: Reminder; quiet?: boolean }) {
  const ctx = useContext(TickContext);
  const done = ctx?.ticked.has(reminder.guid) ?? false;
  const press = {
    onMouseDown: (e: React.MouseEvent) => e.stopPropagation(),
    onPointerDown: (e: React.PointerEvent) => e.stopPropagation(),
    onClick: (e: React.MouseEvent) => {
      e.stopPropagation();
      if (!done) ctx?.tick(reminder);
    },
  };
  // Inside a line that is itself the control (see useShowReminder): drawn for the mouse only,
  // not a control of its own — the card it opens has the ring for the keyboard.
  if (quiet) return <span className={`cx-ring${done ? " is-ticked" : ""}`} aria-hidden="true" {...press} />;
  return (
    <button
      type="button"
      className={`cx-ring${done ? " is-ticked" : ""}`}
      role="checkbox"
      aria-checked={done}
      aria-label={`Complete ${reminder.title}`}
      {...press}
    />
  );
}

const draftOf = (e: CalendarEvent): Draft => ({
  guid: e.guid,
  calendarGuid: e.calendarGuid,
  title: e.title,
  location: e.location ?? "",
  startsAt: e.startsAt,
  endsAt: e.endsAt,
  allDay: e.allDay,
  notes: e.notes ?? "",
  url: e.url,
  timezone: e.timezone,
  opened: e,
});

/** Where a saved event ends: an all-day one at least a day on, a timed one never before its
 *  start. A timed event may end as it starts — a deadline sent in an invitation does — and
 *  lengthening it to a quarter hour changed it on every device whenever its title was edited. */
const endOf = (d: Draft) => Math.max(d.endsAt, d.allDay ? addDaysIn(ZONE, d.startsAt, 1) : d.startsAt);

/**
 * `next`, or `was` when every row holds the same values as before. Every read answers new
 * objects; handed on as they came, each one drew every view again — the week, the month and
 * every reminder's ring — though nothing in them had changed.
 */
function same<T extends object>(was: T[], next: T[]): T[] {
  if (was.length !== next.length) return next;
  for (let i = 0; i < next.length; i++) {
    const a = was[i] as Record<string, unknown>;
    const b = next[i] as Record<string, unknown>;
    if (a === b) continue;
    const keys = Object.keys(b);
    if (keys.length !== Object.keys(a).length || keys.some((k) => a[k] !== b[k])) return next;
  }
  return was;
}
function sameMap<V>(was: Map<string, V>, next: Map<string, V>): Map<string, V> {
  if (was.size !== next.size) return next;
  for (const [k, v] of next) if (was.get(k) !== v) return next;
  return was;
}

/** `YYYY-MM-DDTHH:mm` and `YYYY-MM-DD` for the date inputs, in local time. */
const pad = (n: number) => String(n).padStart(2, "0");
/** The zone an event's times are read in: its own, or this computer's (all-day events are
 * dates, not instants, and read here). */
const zoneOf = (d: { allDay: boolean; timezone: string | null }) => (d.allDay || !d.timezone ? ZONE : d.timezone);
const toLocalInput = (s: number, dateOnly: boolean) => {
  const d = dateOf(s);
  const day = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  return dateOnly ? day : `${day}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
};

const weekdayShort = (d: Date) => dateText(d, { weekday: "short" });

/** The colour a calendar is drawn in, or Apple's blue when it has none. */
const tint = (c: Calendar | undefined) =>
  c?.colour && /^#[0-9a-f]{6}$/i.test(c.colour) ? c.colour : "var(--sys-blue)";

/** Where a popover opens: to the right of `x`, or to the left of `left` (or `x`) when the
 *  window has no room on the right. */
type Anchor = { x: number; y: number; left?: number };

export function CalendarView() {
  const today = useMemo(() => new Date(), []);
  const [now, setNow] = useState(() => new Date());
  const [view, setViewState] = useState<View>(() => {
    try {
      const saved = window.localStorage.getItem(VIEW_KEY) as View | null;
      return saved && VIEWS.some((v) => v.key === saved) ? saved : "month";
    } catch {
      return "month";
    }
  });
  const setView = useCallback((v: View) => {
    setViewState(v);
    try {
      window.localStorage.setItem(VIEW_KEY, v);
    } catch {
      // Remembering the view is a convenience.
    }
  }, []);
  /** A found event: its day, in Day view, and the search put away. */
  const openFound = (e: CalendarEvent) => {
    setSelected(midnight(dateOf(e.startsAt)));
    setView("day");
    setQuery("");
    // Its calendar shown if it was hidden, and the event itself opened, once it is drawn.
    if (hidden.has(e.calendarGuid)) toggleCalendar(e.calendarGuid);
    setPicked(e.guid);
    setFinding(e.guid);
  };

  /** The day everything is anchored on: the one selected, and what the views are built around. */
  const [selected, setSelected] = useState<Date>(() => midnight(today));
  // Search, as Calendar's toolbar field: every word, any calendar, soonest first.
  const [query, setQuery] = useState("");
  const [found, setFoundRaw] = useState<CalendarEvent[]>([]);
  /** The row lit in the results, which Return opens and the arrows move. */
  const [lit, setLit] = useState(0);
  // Coming up first, soonest first; then what has been, latest first — what is looked for is
  // most often next, and a long past pushed it out of sight.
  const setFound = (rows: CalendarEvent[]) => {
    const now = Date.now() / 1000;
    const ahead = rows.filter((e) => e.endsAt >= now).sort((a, b) => a.startsAt - b.startsAt);
    const past = rows.filter((e) => e.endsAt < now).sort((a, b) => b.startsAt - a.startsAt);
    setFoundRaw([...ahead, ...past]);
    setLit(0);
  };
  const searchField = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    const q = query.trim();
    if (!q) return setFound([]);
    let cancelled = false;
    const timer = window.setTimeout(() => {
      api.calendarSearch(q).then((r) => !cancelled && setFound(r)).catch(() => !cancelled && setFound([]));
    }, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query]);
  const [events, setEvents] = useState<CalendarEvent[]>([]);
  const [due, setDue] = useState<Reminder[]>([]);
  const [ticked, setTicked] = useState<Set<string>>(() => new Set());
  /** Each Reminders list's colour, by list. */
  const [listColours, setListColours] = useState<Map<string, string>>(() => new Map());
  /** Each Reminders list's name, for a reminder's card. */
  const [listNames, setListNames] = useState<Map<string, string>>(() => new Map());
  /** The reminder whose card is open, and where it was clicked. */
  const [card, setCard] = useState<{ reminder: Reminder; at: Anchor } | null>(null);
  const tickValue = useMemo(() => ({
    ticked,
    colour: (r: Reminder) => listColours.get(r.listGuid) || "var(--sys-blue)",
    tick: (r: Reminder) => {
      setTicked((was) => new Set(was).add(r.guid));
      const untick = () => setTicked((was) => { const next = new Set(was); next.delete(r.guid); return next; });
      api.reminderSetDone(r.guid, r.listGuid, true).then(
        () => window.setTimeout(() => { setDue((rows) => rows.filter((x) => x.guid !== r.guid)); untick(); }, 600),
        (e) => { untick(); setError(messageOf(e)); },
      );
    },
    show: (r: Reminder, at: Anchor) => setCard({ reminder: r, at }),
  }), [ticked, listColours]);
  const closeCard = useCallback(() => setCard(null), []);
  const [calendars, setCalendars] = useState<Calendar[]>([]);
  /** A calendar being named: "new" for New Calendar, or the guid of one being renamed. */
  const [naming, setNaming] = useState<string | null>(null);
  const [calMenu, setCalMenu] = useState<{ cal: Calendar; at: { x: number; y: number; alignRight: boolean } } | null>(null);
  const [hidden, setHidden] = useState<Set<string>>(() => readSet(HIDDEN_KEY));
  /** Calendars subscribed to by address, as kept on this computer. */
  const [subs, setSubs] = useState<CalendarSubscription[]>([]);
  /** Sidebar groups folded away. */
  const [folded, setFolded] = useState<Set<string>>(() => readSet(FOLDED_KEY));
  const fold = (key: string) =>
    setFolded((was) => {
      const next = new Set(was);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      writeJson(FOLDED_KEY, [...next]);
      return next;
    });
  const [weekNumbers, setWeekNumbersState] = useState<boolean>(() => readJson(WEEK_NUMBERS_KEY, false));
  const setWeekNumbers = (on: boolean) => {
    setWeekNumbersState(on);
    writeJson(WEEK_NUMBERS_KEY, on);
  };
  const [defaultAlerts, setDefaultAlertsState] = useState<DefaultAlerts>(() => {
    const kept = readJson<DefaultAlerts>(DEFAULT_ALERTS_KEY, { timed: {}, allDay: {} });
    const allDay = Object.fromEntries(Object.entries(kept.allDay ?? {}).map(([g, m]) => [g, ALL_DAY_WAS[m] ?? m]));
    return { timed: kept.timed ?? {}, allDay };
  });
  const setDefaultAlert = (kind: "timed" | "allDay", guid: string, minutes: number | null) =>
    setDefaultAlertsState((was) => {
      const next = { ...was, [kind]: { ...was[kind] } };
      if (minutes === null) delete next[kind][guid];
      else next[kind][guid] = minutes;
      writeJson(DEFAULT_ALERTS_KEY, next);
      return next;
    });
  /** The event picked — the last one clicked — that Return opens, Delete deletes, ⌘D
   *  duplicates and ⌘C copies; and the event copied, for ⌘V to put on the selected day. */
  const [picked, setPicked] = useState<string | null>(null);
  const [clip, setClip] = useState<CalendarEvent | null>(null);
  /** Quick Event, opened under the + button. */
  const [quick, setQuick] = useState(false);
  /** New Calendar Subscription, opened from the sidebar's +. */
  const [subscribing, setSubscribing] = useState(false);
  const [addMenu, setAddMenu] = useState<{ x: number; y: number; alignRight: boolean } | null>(null);
  const [groupMenu, setGroupMenu] = useState<{ list: Calendar[]; at: { x: number; y: number; alignRight: boolean } } | null>(null);
  const [moreMenu, setMoreMenu] = useState<{ x: number; y: number; alignRight: boolean } | null>(null);
  const [eventMenu, setEventMenu] = useState<{ event: CalendarEvent; at: { x: number; y: number; alignRight: boolean } } | null>(null);
  // An event found by the search across everything: its day shown, its calendar too if it
  // was hidden (and kept shown, as its tick in the list says) — never a found event that
  // cannot be seen — and the event itself opened, as Calendar opens one found.
  const [finding, setFinding] = useState<string | null>(null);
  // "New Event" from the taskbar's jump list (`skykeep --new event`, Library.tsx).
  const [newAsked, setNewAsked] = useState(false);
  useReveal("calendar", (r) => {
    if (r.startNew) {
      setNewAsked(true);
      return;
    }
    if (r.at) setSelected(midnight(new Date(r.at * 1000)));
    if (view === "year") setView("day");
    const cal = r.within;
    if (cal && hidden.has(cal)) toggleCalendar(cal);
    setFinding(r.id);
  });
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useErrorToast(error, () => setError(null));
  const [confirm, confirmSheet] = useConfirm();
  const [draft, setDraft] = useState<Draft | null>(null);
  const draftRef = useRef<Draft | null>(null);
  draftRef.current = draft;
  const [anchor, setAnchor] = useState<Anchor | null>(null);
  /** The last event deleted, with its alerts (read from Apple as it goes), held so the deletion
   *  can be taken back. */
  const [undo, setUndo] = useState<(CalendarEvent & { alerts: Promise<number[]> }) | null>(null);
  // Held while another screen is in front, and read once on coming back (lib/sync).
  const revision = useShownRevision("calendar", "reminders", "contacts");
  // Changes kept here and not yet at Apple (the outbox): a saved event says so, and one Apple
  // turned down is shown with what it held.
  const outbox = useOutbox("calendar");

  // The current-time line moves once a minute, on the minute — not on a busy timer.
  useEffect(() => {
    let timer = 0;
    const tick = () => {
      setNow(new Date());
      timer = window.setTimeout(tick, 60_000 - (Date.now() % 60_000));
    };
    timer = window.setTimeout(tick, 60_000 - (Date.now() % 60_000));
    // A timer sleeps with the machine: woken after midnight, today stayed yesterday — the red
    // circle and the line for now in the wrong place — until the minute came round. Coming
    // back to the window reads the clock again.
    const wake = () => {
      if (document.visibilityState === "hidden") return;
      window.clearTimeout(timer);
      tick();
    };
    document.addEventListener("visibilitychange", wake);
    window.addEventListener("focus", wake);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", wake);
      window.removeEventListener("focus", wake);
    };
  }, []);

  // The first day of the week, changed in Settings or read late from the system's LC_TIME:
  // every view is drawn again from it.
  const [weekStart, setWeekStartTick] = useState(0);
  useEffect(() => onWeekChange(() => setWeekStartTick((n) => n + 1)), []);
  const week = useMemo(() => {
    const start = startOfWeek(selected);
    return Array.from({ length: 7 }, (_, i) => addDays(start, i));
  }, [selected, weekStart]);
  const month = useMemo(() => monthDays(selected), [selected, weekStart]);

  // What each view reads from the library. Day and Week read the month around them and a
  // month either side: asked for one day at a time, stepping to the next day showed the last
  // day's data — none of it on the new day — until the read came back, and the day's list
  // flickered empty and full. Read from this computer, three months cost nothing, and moving
  // through them waits on nothing. Month reads a month either side too: read alone, the next
  // month came up empty and filled in a moment later on every step.
  //
  // And the range read is kept while what is on screen, and a step either way, is inside it:
  // a day stepped into the next month, or a week across one, asks for nothing again.
  const heldRange = useRef<{ from: number; to: number; year: boolean } | null>(null);
  const range = useMemo(() => {
    if (view === "year") {
      const year = {
        from: seconds(new Date(selected.getFullYear(), 0, 1)),
        to: seconds(new Date(selected.getFullYear() + 1, 0, 1)),
        year: true,
      };
      const h = heldRange.current;
      return (heldRange.current = h && h.year && h.from === year.from ? h : year);
    }
    // On screen, and a step on either side: a week in Day and Week, a month's grid in Month.
    const need = view === "month"
      ? { from: seconds(monthDays(addDays(month[0], -1))[0]), to: seconds(addDays(monthDays(addDays(month[41], 1))[41], 1)) }
      : { from: seconds(addDays(week[0], -7)), to: seconds(addDays(week[6], 8)) };
    const h = heldRange.current;
    if (h && !h.year && h.from <= need.from && h.to >= need.to) return h;
    return (heldRange.current = {
      from: Math.min(need.from, seconds(addDays(month[0] < week[0] ? month[0] : week[0], -35))),
      to: Math.max(need.to, seconds(addDays(month[41] > week[6] ? month[41] : week[6], 36))),
      year: false,
    });
  }, [view, selected, week, month]);

  const { hold, over } = usePending<CalendarEvent>();
  // Reads in the order they were asked: one answered after a later one — a month stepped
  // past, a slow read under a quick one — is let go, never drawn over what is newer.
  const asked = useRef(0);
  const shownRead = useRef(0);
  const read = useCallback(async () => {
    const turn = ++asked.current;
    const [found, cals, reminders, lists, feeds] = await Promise.all([
      api.calendarEvents(range.from, range.to),
      api.calendarList(),
      // A day either side: an all-day reminder is kept as its day's midnight in UTC, which west
      // of Greenwich is the evening before the view's first local midnight — and was left out.
      // Each is placed by its own day below, so one outside the view is simply not drawn.
      api.remindersDueBetween(range.from - 86_400, range.to + 86_400),
      api.reminderLists().catch(() => []),
      calendarFeedsApi.list().catch(() => [] as CalendarSubscription[]),
    ]);
    if (turn < shownRead.current) return;
    shownRead.current = turn;
    // What reads the same as before is kept as it was, so a read that found nothing new —
    // after every save, every sync, every month stepped into — draws nothing again.
    setSubs((was) => same(was, Array.isArray(feeds) ? feeds : []));
    setListColours((was) => sameMap(was, new Map((Array.isArray(lists) ? lists : []).map((l) => [l.guid, l.colour && /^#[0-9a-f]{6}$/i.test(l.colour) ? l.colour : ""]))));
    setListNames((was) => sameMap(was, new Map((Array.isArray(lists) ? lists : []).map((l) => [l.guid, l.title]))));
    setEvents((was) => same(was, over(Array.isArray(found) ? found : [])));
    setCalendars((was) => same(was, Array.isArray(cals) ? cals : []));
    // What is done is not on the schedule any more; Calendar hides completed reminders.
    // An all-day reminder's date is Apple's: midnight UTC on its day. Read as an instant it
    // landed a day early for anyone west of UTC; it is that day, on this clock.
    const onItsDay = (r: Reminder) => {
      if (!r.allDay || r.dueAt === null) return r;
      const d = new Date(r.dueAt * 1000);
      return { ...r, dueAt: Math.floor(new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()).getTime() / 1000) };
    };
    setDue((was) => same(was, Array.isArray(reminders) ? reminders.filter((r) => !r.completedAt).map(onItsDay) : []));
    setLoaded(true);
  }, [range, over]);

  useEffect(() => {
    let cancelled = false;
    read().catch((e) => {
      if (!cancelled) {
        setError(messageOf(e));
        setLoaded(true);
      }
    });
    return () => {
      cancelled = true;
    };
    // `outbox.tick`: a change sent, or turned down, changes what the library holds.
  }, [read, revision, outbox.tick]);

  // Beyond the months the shared refresh keeps warm, fetch what is being looked at — once,
  // and not for every month flicked past on the way.
  // What Month fetches is the month on screen: the months either side are read only from
  // what is here, and fetched when they are looked at.
  const fetched = useRef(new Set<string>());
  const fetchFrom = view === "month" ? seconds(month[0]) : range.from;
  const fetchTo = view === "month" ? seconds(addDays(month[41], 1)) : range.to;
  useEffect(() => {
    const from = toLocalInput(fetchFrom, true);
    const to = toLocalInput(fetchTo - 1, true);
    const key = `${from}:${to}`;
    if (fetched.current.has(key)) return undefined;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      fetched.current.add(key);
      api
        .calendarSync(from, to, ZONE)
        .then(() => (cancelled ? undefined : read()))
        .catch((e) => {
          fetched.current.delete(key);
          if (!cancelled) setError(messageOf(e));
        });
    }, 350);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [fetchFrom, fetchTo, read]);

  const byGuid = useMemo(() => new Map(calendars.map((c) => [c.guid, c])), [calendars]);
  const shown = useMemo(
    () => events.filter((e) => !hidden.has(e.calendarGuid)),
    [events, hidden]
  );
  const writable = calendars.filter((c) => c.writable);
  const target = defaultCalendar(calendars);

  /** A group's Show All and Hide All. */
  const showOnly = (list: Calendar[], show: boolean) =>
    setHidden((was) => {
      const next = new Set(was);
      for (const c of list) {
        if (show) next.delete(c.guid);
        else next.add(c.guid);
      }
      writeJson(HIDDEN_KEY, [...next]);
      return next;
    });
  const toggleCalendar = (guid: string) =>
    setHidden((was) => {
      const next = new Set(was);
      if (next.has(guid)) next.delete(guid);
      else next.add(guid);
      try {
        window.localStorage.setItem(HIDDEN_KEY, JSON.stringify([...next]));
      } catch {
        // Still hidden for now; forgotten on restart.
      }
      return next;
    });

  /**
   * Every write shows at once — an event moved, resized, retitled or deleted is drawn that
   * way the moment it is asked for — then goes to Apple and the library is re-read. A
   * refusal re-reads, which puts the event back where it was, and says why.
   */
  const write = useCallback(
    async (run: () => Promise<void>, predict?: (rows: CalendarEvent[]) => CalendarEvent[]) => {
      setError(null);
      if (predict) setEvents((rows) => predict(rows));
      // Kept over any refresh until Apple has it; let go before the read that confirms it.
      const release = predict ? hold(predict) : () => {};
      try {
        await run();
        release();
        await read();
        return true;
      } catch (e) {
        release();
        setError(messageOf(e));
        read().catch(() => {});
        return false;
      } finally {
        release();
      }
    },
    [read, hold]
  );

  /** Calendars made, renamed, recoloured and removed — shown at once, put back if Apple refuses. */
  const createCalendar = (title: string) => {
    setNaming(null);
    if (title.trim()) void write(async () => { await api.calendarCreate(title.trim()); });
  };
  const updateCalendar = (c: Calendar, change: { title?: string; colour?: string }) => {
    setNaming(null);
    if (change.title !== undefined && (!change.title.trim() || change.title.trim() === c.title)) return;
    setCalendars((cs) => cs.map((x) => (x.guid === c.guid ? { ...x, ...change } : x)));
    void write(async () => { await api.calendarUpdate(c.guid, change); });
  };
  const deleteCalendar = (c: Calendar) => {
    // icloud.com's own warning: the events go with it, on every device.
    void confirm({
      title: `Delete \u201c${c.title}\u201d?`,
      body: "All events in this calendar will be deleted from every device.",
      action: "Delete",
      danger: true,
      symbol: "trash",
    }).then((sure) => {
      if (!sure) return;
      setCalendars((cs) => cs.filter((x) => x.guid !== c.guid));
      void write(async () => { await api.calendarDelete(c.guid); });
    });
  };

  /**
   * One occurrence of a repeating event is changed only once the person has said it is for
   * that occurrence alone, as Calendar asks — sent as though it were an event, Apple took it
   * as the whole series.
   */
  const [asking, setAsking] = useState<{ verb: "change" | "delete"; answer: (scope: EventScope | null) => void } | null>(null);
  const live = useActiveRef();
  useEffect(() => {
    if (!asking) return undefined;
    const onKey = (e: KeyboardEvent) => {
      if (!live.current) return;
      if (e.key === "Escape") {
        e.stopPropagation();
        asking.answer(null);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [asking]);
  const whichEvents = (verb: "change" | "delete") =>
    new Promise<EventScope | null>((answer) => setAsking({ verb, answer: (scope) => { setAsking(null); answer(scope); } }));

  /**
   * What ⌘Z (Ctrl+Z here) takes back, as Edit ▸ Undo does in Calendar: the last event deleted,
   * or the last change to one — a drag, a resize, a nudge, an edit — put back as it was. One
   * step, and only for an event of its own: one day of a repeating event, or a change of its
   * alerts or repeat, is not something putting the old fields back would undo.
   */
  const lastAct = useRef<{ kind: "delete"; gone: CalendarEvent & { alerts: Promise<number[]> } } | { kind: "change"; before: Draft } | null>(null);

  const save = useCallback(
    async (d: Draft, close = false, undoing = false) => {
      const occurrence = Boolean(d.guid && d.guid.includes("__"));
      // A repeat belongs to the series, so changing it is a change to all of it.
      const scope = occurrence ? (d.repeatChanged ? "all" : await whichEvents("change")) : null;
      if (occurrence && !scope) return false;
      // Closed the moment it is saved, drawn as saved, never waiting on Apple: kept open until
      // Apple answered, the popover hung for a second or two on every Done.
      if (close) setDraft(null);
      const was = d.guid ? events.find((e) => e.guid === d.guid) : undefined;
      const undoable = !undoing && !occurrence && !d.alertsChanged && !d.repeatChanged && was &&
        !was.guid.startsWith("quince:") && !was.guid.startsWith("pending:") && (
          was.startsAt !== d.startsAt || was.endsAt !== endOf(d) || was.title !== (d.title.trim() || "New Event") ||
          (was.location ?? "") !== d.location.trim() || (was.notes ?? "") !== d.notes.trim() ||
          was.allDay !== d.allDay || was.calendarGuid !== d.calendarGuid || (was.url ?? "") !== (d.url?.trim() ?? "") ||
          (!d.allDay && was.timezone !== d.timezone));
      const step = undoable ? { kind: "change" as const, before: draftOf(was) } : null;
      if (step) lastAct.current = step;
      else if (undoing) lastAct.current = null;
      // What the save is made from: the event as its draft was made from it, so a change
      // another device made meanwhile is merged rather than overwritten. Undo puts the old
      // fields back over the event as it is now.
      const base = d.guid ? (undoing ? was : (d.opened ?? was)) : undefined;
      const saved = await write(async () => {
        await api.eventSave({
          ...(scope ? { scope } : {}),
          guid: d.guid,
          calendarGuid: d.calendarGuid,
          title: d.title.trim() || "New Event",
          location: d.location.trim() || null,
          startsAt: d.startsAt,
          endsAt: endOf(d),
          allDay: d.allDay,
          timezone: d.allDay ? ZONE : (d.timezone ?? ZONE),
          notes: d.notes.trim() || null,
          url: d.url?.trim() || null,
          ...(d.alertsChanged && d.alerts ? { alerts: d.alerts } : {}),
          ...(d.repeatChanged ? { repeat: d.repeat ?? null, repeatChanged: true } : {}),
          ...(base ? { base } : {}),
        });
      }, (rows) => {
        // Shown where it now sits at once; a new one is drawn by its draft until Apple names it.
        if (!d.guid) {
          return [...rows, {
            guid: `pending:${d.startsAt}:${d.title}`, calendarGuid: d.calendarGuid,
            title: d.title.trim() || "New Event", location: d.location.trim() || null,
            startsAt: d.startsAt, endsAt: endOf(d),
            allDay: d.allDay, timezone: d.allDay ? ZONE : (d.timezone ?? ZONE),
            url: d.url?.trim() || null, notes: d.notes.trim() || null, etag: null,
          }];
        }
        // Future and All carry the words to the other days too; their times Apple settles.
        const series = d.guid.split("__")[0];
        const from = events.find((e) => e.guid === d.guid)?.startsAt ?? d.startsAt;
        const alsoThis = (e: { guid: string; startsAt: number }) =>
          e.guid !== d.guid && e.guid.split("__")[0] === series &&
          (scope === "all" || (scope === "future" && e.startsAt >= from));
        return rows.map((e) => alsoThis(e) ? {
          ...e, title: d.title.trim() || "New Event", location: d.location.trim() || null,
          notes: d.notes.trim() || null,
        } : e.guid === d.guid ? {
          ...e, calendarGuid: d.calendarGuid, title: d.title.trim() || "New Event",
          location: d.location.trim() || null, startsAt: d.startsAt,
          endsAt: endOf(d),
          allDay: d.allDay, notes: d.notes.trim() || null,
        } : e);
      });
      // Turned down, there is nothing of it to take back.
      if (!saved && step && lastAct.current === step) lastAct.current = null;
      return saved;
    },
    [write, events]
  );

  /**
   * iCloud keeps no bin for calendar events: deleting one is final at Apple. So the deletion
   * can be taken back here, the way Undo works in Calendar — the event is written again.
   */
  const remove = useCallback(
    async (guid: string) => {
      const gone = events.find((e) => e.guid === guid) ?? null;
      const occurrence = guid.includes("__");
      const scope = occurrence ? await whichEvents("delete") : null;
      if (occurrence && !scope) return;
      // Its alerts, for Undo to make it again with them. Read from Apple alongside the deletion,
      // not before it: awaited first, the event stayed on the grid for a second or two online
      // and until the request gave up offline. The read starts first, so it finds the event in
      // the library, and the deletion is queued once it has its answer — the grid has changed
      // long before. Never on a timer: a deletion that reached Apple first left the read
      // nothing to find, and Undo made the event again without its alerts.
      const alerts: Promise<number[]> = occurrence
        ? Promise.resolve([])
        : draftRef.current?.guid === guid && draftRef.current.alerts
          ? Promise.resolve(draftRef.current.alerts)
          : api.eventAlerts(guid).catch(() => [] as number[]);
      setDraft(null);
      // All of a series leaves every one of its days; one, only itself.
      const series = guid.split("__")[0];
      if (await write(
        async () => {
          await alerts;
          await api.eventDelete(guid, scope ?? undefined);
        },
        (rows) => rows.filter((e) =>
          scope === "all" ? e.guid.split("__")[0] !== series
          : scope === "future" ? !(e.guid.split("__")[0] === series && e.startsAt >= (gone?.startsAt ?? 0))
          : e.guid !== guid),
      )) {
        // Undo makes the event again; for one day out of a series that would be a new event,
        // not the day put back, so it is not offered.
        if (!occurrence && gone) {
          setUndo({ ...gone, alerts });
          lastAct.current = { kind: "delete", gone: { ...gone, alerts } };
        }
      }
    },
    [events, write]
  );
  /** A deleted event made again as it was, its alerts with it (its guests are not this app's
   *  to invite). */
  const restore = useCallback(async (gone: CalendarEvent & { alerts: Promise<number[]> }) => {
    if (lastAct.current?.kind === "delete" && lastAct.current.gone.guid === gone.guid) lastAct.current = null;
    const alerts = await gone.alerts;
    await save({ ...draftOf(gone), guid: null, alerts, alertsChanged: alerts.length > 0 }, false, true);
  }, [save]);
  const takeBack = useCallback(async () => {
    const gone = undo;
    setUndo(null);
    if (gone) await restore(gone);
  }, [undo, restore]);
  const moveEvent = useCallback(
    // `allDay` when it was carried between the all-day row and the hours: Undo puts it back as
    // it was, all-day or not, as for any change.
    (e: CalendarEvent, startsAt: number, endsAt: number, allDay = e.allDay) =>
      void save({ ...draftOf(e), startsAt, endsAt, allDay }),
    [save]
  );
  /** Dragged to another day in Month: the same clock time, that many days on. */
  const moveDays = useCallback((e: CalendarEvent, days: number) => {
    if (!days) return;
    const zone = zoneOf(e);
    void save({ ...draftOf(e), startsAt: addDaysIn(zone, e.startsAt, days), endsAt: addDaysIn(zone, e.endsAt, days) });
  }, [save]);

  /** Where a copy of `e` can go: its own calendar, or the default when that one is read-only. */
  const homeFor = (e: CalendarEvent) => (byGuid.get(e.calendarGuid)?.writable ? e.calendarGuid : targetGuid);
  /** Real events at Apple carry alerts and a repeat to read; birthdays and feeds do not. */
  const atApple = (guid: string) => !guid.startsWith("quince:") && !guid.startsWith("pending:");

  /**
   * ⌘D: a copy at the same time, its alerts and repeat with it, as Calendar duplicates. The
   * copy is drawn at once; its alerts and repeat are read from Apple meanwhile, and it is made
   * with them once they come. Read first, the copy appeared a second or two after the key.
   */
  const duplicate = async (e: CalendarEvent) => {
    const into = homeFor(e);
    if (!into) return;
    const occurrence = e.guid.includes("__");
    if (!atApple(e.guid)) {
      await save({ ...draftOf(e), guid: null, calendarGuid: into, alerts: alertsFor(into, e.allDay), alertsChanged: true, repeat: null, repeatChanged: false });
      return;
    }
    const copy: CalendarEvent = { ...e, guid: `pending:copy:${e.guid}:${Date.now()}`, calendarGuid: into, etag: null };
    const draw = (rows: CalendarEvent[]) => [...rows, copy];
    setEvents(draw);
    const release = hold(draw);
    const [alerts, repeat] = await Promise.all([
      api.eventAlerts(e.guid).catch(() => [] as number[]),
      occurrence ? Promise.resolve(null) : api.eventRepeat(e.guid).catch(() => null),
    ]);
    // The stand-in gives way to the copy as it is saved, in the same draw.
    release();
    setEvents((rows) => rows.filter((x) => x.guid !== copy.guid));
    await save({ ...draftOf(e), guid: null, calendarGuid: into, alerts, alertsChanged: true, repeat, repeatChanged: repeat !== null });
  };
  /** ⌘V: the copied event on the selected day, at its own time of day. */
  const paste = (e: CalendarEvent) => {
    const into = homeFor(e);
    if (!into) return;
    const day = selected;
    let startsAt: number;
    let endsAt: number;
    if (e.allDay) {
      const days = Math.max(1, Math.round((e.endsAt - e.startsAt) / 86_400));
      startsAt = seconds(day);
      endsAt = seconds(addDays(day, days));
    } else {
      const was = dateOf(e.startsAt);
      startsAt = seconds(new Date(day.getFullYear(), day.getMonth(), day.getDate(), was.getHours(), was.getMinutes()));
      endsAt = startsAt + (e.endsAt - e.startsAt);
    }
    void save({ ...draftOf(e), guid: null, calendarGuid: into, startsAt, endsAt, alerts: alertsFor(into, e.allDay), alertsChanged: true });
  };
  /** Delete from the keyboard: asked first, as a key is easily pressed. A repeating event's
   *  own question (this one, future or all) is the asking. */
  const deleteAsking = async (e: CalendarEvent) => {
    if (!byGuid.get(e.calendarGuid)?.writable) return;
    if (!e.guid.includes("__")) {
      const sure = await confirm({
        title: `Delete \u201c${e.title || "New Event"}\u201d?`,
        body: "It will be deleted from all your devices.",
        action: "Delete",
        danger: true,
        symbol: "trash",
      });
      if (!sure) return;
    }
    void remove(e.guid);
  };
  /** Return on a picked event: opened beside where it is drawn. */
  const openPicked = (e: CalendarEvent) => {
    const block = document.querySelector(`[data-event="${CSS.escape(e.guid)}"]`);
    const r = block?.getBoundingClientRect();
    openDraft(draftOf(e), r ? { x: r.right, y: r.top, left: r.left } : { x: window.innerWidth / 2, y: 120 });
  };

  /** Quick Event: the line read here, shown, and on Return made as read. */
  const addQuick = async (text: string, details: boolean, at: Anchor) => {
    if (!targetGuid) return;
    const q = parseQuick(text);
    setQuick(false);
    const d: Draft = {
      guid: null, calendarGuid: targetGuid, title: q.title, location: q.location ?? "",
      startsAt: q.startsAt, endsAt: q.endsAt, allDay: q.allDay, notes: "", url: null, timezone: null,
      repeat: q.repeat, repeatChanged: q.repeat !== null,
    };
    setSelected(midnight(dateOf(q.startsAt)));
    if (details || !q.title) {
      openDraft(d, at);
      setFollowDraft(true);
      return;
    }
    await save({ ...d, alerts: alertsFor(targetGuid, q.allDay), alertsChanged: true });
  };

  /** Subscriptions: renamed, recoloured, refreshed or let go, kept on this computer. */
  const updateSub = (id: string, change: { title?: string; colour?: string; refreshMinutes?: number }) => {
    setNaming(null);
    if (change.title !== undefined && !change.title.trim()) return;
    setSubs((list) => list.map((x) => (x.id === id ? { ...x, ...change } : x)));
    setCalendars((cs) => cs.map((c) => (c.guid === SUBSCRIPTION_PREFIX + id ? { ...c, ...(change.title ? { title: change.title } : {}), ...(change.colour ? { colour: change.colour } : {}) } : c)));
    void write(async () => { await calendarFeedsApi.update(id, change); });
  };
  const refreshSub = (id: string) => {
    calendarFeedsApi.refresh(id).then(
      (s) => { toast(`\u201c${s.title}\u201d is up to date`); void read(); },
      (e) => setError(messageOf(e)),
    );
  };
  const unsubscribe = (sub: CalendarSubscription) => {
    void confirm({
      title: `Unsubscribe from \u201c${sub.title}\u201d?`,
      body: "Its events leave this computer. You can subscribe again at any time.",
      action: "Unsubscribe",
      danger: true,
    }).then((sure) => {
      if (!sure) return;
      setCalendars((cs) => cs.filter((c) => c.guid !== SUBSCRIPTION_PREFIX + sub.id));
      setEvents((rows) => rows.filter((e) => e.calendarGuid !== SUBSCRIPTION_PREFIX + sub.id));
      void write(async () => { await calendarFeedsApi.unsubscribe(sub.id); });
    });
  };
  /**
   * The popover put away by a click elsewhere keeps what was written in it, as Calendar's
   * does — a title typed and then a click on the next day had thrown the title away. Escape
   * and Cancel still leave the event as it was, and a new event nothing was written into is
   * not made.
   */
  const dismiss = useCallback(() => {
    const d = draftRef.current;
    const was = openedAs.current;
    const changed = Boolean(d && was && (
      d.title !== was.title || d.location !== was.location || d.notes !== was.notes ||
      (d.url ?? "") !== (was.url ?? "") || d.calendarGuid !== was.calendarGuid ||
      d.timezone !== was.timezone || d.allDay !== was.allDay ||
      (d.alertsChanged && d.alerts !== was.alerts) || (d.repeatChanged && d.repeat !== was.repeat) ||
      // A new event's times are where it was made; only an existing one's are a change.
      (d.guid !== null && (d.startsAt !== was.startsAt || d.endsAt !== was.endsAt))
    ));
    if (d && changed) void save(d, true);
    else setDraft(null);
  }, [save]);
  // Deleted, said in a toast with Undo for as long as Undo can put it back.
  useEffect(() => {
    if (!undo) return undefined;
    const id = toast({ message: `Deleted \u201c${undo.title}\u201d`, action: { label: "Undo", run: () => void takeBack() }, duration: 8000 });
    const timer = window.setTimeout(() => setUndo(null), 8000);
    return () => {
      window.clearTimeout(timer);
      dismissToast(id);
    };
  }, [undo, takeBack]);

  /** A calendar's default alert for a new event, kept on this computer (Apple's web service
   *  has no field for one): minutes before, as the alert pickers count. */
  const defaultsRef = useRef(defaultAlerts);
  defaultsRef.current = defaultAlerts;
  const alertsFor = useCallback((calendarGuid: string, allDay: boolean): number[] => {
    const m = defaultsRef.current[allDay ? "allDay" : "timed"][calendarGuid];
    return m === undefined ? [] : [m];
  }, []);
  /** The popover's changes, a new event's default alerts kept in step with its calendar. */
  const changeDraft = useCallback((next: Draft) => {
    setDraft(next.alertsDefault && !next.guid ? { ...next, alerts: alertsFor(next.calendarGuid, next.allDay) } : next);
  }, [alertsFor]);

  /** The draft as it was opened, to tell whether anything in it has been changed since. */
  const openedAs = useRef<Draft | null>(null);
  const openDraft = useCallback((d: Draft, at: Anchor) => {
    // The event already open stays as it is: a second click — a double-click — closed it and
    // opened it again, and the popover flickered.
    if (d.guid && draftRef.current?.guid === d.guid) return;
    const opened = d.guid ? d : {
      ...d,
      alerts: d.alerts ?? alertsFor(d.calendarGuid, d.allDay),
      alertsChanged: true,
      alertsDefault: d.alerts === undefined,
      repeat: d.repeat ?? null,
      repeatChanged: Boolean(d.repeat),
    };
    openedAs.current = opened;
    setDraft(opened);
    setAnchor(at);
    const guid = d.guid;
    if (guid && !guid.startsWith("quince:")) {
      api.eventAlerts(guid).then(
        (alerts) => setDraft((cur) => (cur && cur.guid === guid && !cur.alertsChanged ? { ...cur, alerts } : cur)),
        () => {},
      );
      api.eventRepeat(guid).then(
        (repeat) => setDraft((cur) => (cur && cur.guid === guid && !cur.repeatChanged ? { ...cur, repeat } : cur)),
        () => {},
      );
    }
  }, []);
  // Opened once it is drawn: its day and calendar may take a render or two to come up. One
  // that never shows (gone since it was found) is let go of rather than opened later.
  useLayoutEffect(() => {
    if (!finding) return;
    const event = events.find((e) => e.guid === finding);
    const block = document.querySelector(`[data-event="${CSS.escape(finding)}"]`);
    if (!event || !block) return;
    setFinding(null);
    block.scrollIntoView({ block: "nearest" });
    const r = block.getBoundingClientRect();
    openDraft(draftOf(event), { x: r.right, y: r.top, left: r.left });
  });
  useEffect(() => {
    if (!finding) return undefined;
    const t = window.setTimeout(() => setFinding(null), 3000);
    return () => window.clearTimeout(t);
  }, [finding]);
  const targetGuid = target?.guid ?? null;
  const newAt = useCallback((startsAt: number, endsAt: number, allDay: boolean, at: Anchor) => {
    if (!targetGuid) return;
    openDraft(
      { guid: null, calendarGuid: targetGuid, title: "", location: "", startsAt, endsAt, allDay, notes: "", url: null, timezone: null },
      at
    );
  }, [targetGuid, openDraft]);

  /*
   * What the views are handed, kept the same from one draw to the next so a view whose own
   * inputs have not changed is not drawn again — not the week for every key typed in the
   * popover, not the year for every minute the clock moves on.
   */
  const openEvent = useCallback((e: CalendarEvent, at: Anchor) => {
    setPicked(e.guid);
    openDraft(draftOf(e), at);
  }, [openDraft]);
  /** Right-click on an event: what can be done with it, as Calendar's shortcut menu offers. */
  const menuFor = useCallback((e: CalendarEvent, at: { x: number; y: number }) => {
    setPicked(e.guid);
    setEventMenu({ event: e, at: { ...at, alignRight: false } });
  }, []);
  const createInMonth = useCallback((day: Date, at: Anchor) => {
    const start = new Date(day);
    start.setHours(9, 0, 0, 0);
    newAt(seconds(start), seconds(start) + 3600, false, at);
  }, [newAt]);
  const showDay = useCallback((day: Date) => {
    setSelected(day);
    setView("day");
  }, [setView]);
  const canWrite = useCallback((guid: string) => byGuid.get(guid)?.writable ?? false, [byGuid]);
  const gridDays = useMemo(() => (view === "week" ? week : [selected]), [view, week, selected]);
  const pendingFrom = draft && !draft.guid ? draft.startsAt : null;
  const pendingTo = draft && !draft.guid ? draft.endsAt : null;
  const pendingTitle = draft && !draft.guid ? draft.title : "";
  const pendingAllDay = Boolean(draft && !draft.guid && draft.allDay);
  const pending = useMemo(
    () => (pendingFrom === null || pendingTo === null ? null : { startsAt: pendingFrom, endsAt: pendingTo, title: pendingTitle, allDay: pendingAllDay }),
    [pendingFrom, pendingTo, pendingTitle, pendingAllDay]
  );

  /** "New event": the next whole hour on the selected day, for an hour — Apple's default. */
  const newEvent = (e?: React.MouseEvent) => {
    const at = new Date(selected);
    // Late in the evening the next whole hour is tomorrow — in Week, often next week, off
    // the screen it was asked for on. It stays on the day: the last hour of it.
    at.setHours(Math.min(now.getHours() + 1, 23), 0, 0, 0);
    const button = (e?.currentTarget ?? document.querySelector('[aria-label="New event"]'))?.getBoundingClientRect()
      ?? new DOMRect(window.innerWidth - 40, 40, 0, 0);
    // Under the button, never over the toolbar; in Day and Week it moves beside the new event
    // once that is drawn.
    newAt(seconds(at), seconds(at) + 3600, false, { x: button.right, y: button.bottom + 32 });
    setFollowDraft(true);
  };
  const [followDraft, setFollowDraft] = useState(false);
  // "New Event" asked from outside the window (useReveal above), held until there is a
  // calendar to put it in, so a window still loading starts it once it can.
  useEffect(() => {
    if (!newAsked || !targetGuid) return;
    setNewAsked(false);
    newEvent();
  });
  useLayoutEffect(() => {
    if (!followDraft) return;
    setFollowDraft(false);
    const block = document.querySelector(".cx-block.is-draft");
    if (!block) return;
    block.scrollIntoView({ block: "nearest" });
    const r = block.getBoundingClientRect();
    setAnchor({ x: r.right, y: r.top, left: r.left });
  }, [followDraft]);

  const shift = (by: number) =>
    setSelected((d) => {
      if (view === "day") return addDays(d, by);
      if (view === "week") return addDays(d, by * 7);
      if (view === "month") return new Date(d.getFullYear(), d.getMonth() + by, 1);
      return new Date(d.getFullYear() + by, d.getMonth(), 1);
    });

  /** Go to Date…: a day typed in, shown in the view as it is. */
  const [goingTo, setGoingTo] = useState(false);
  const goTo = (value: string) => {
    const [y, m, d] = value.split("-").map(Number);
    if (!y || !m || !d) return;
    setSelected(new Date(y, m - 1, d));
    setGoingTo(false);
  };

  // Keyboard, as in Calendar on a Mac (Ctrl here): arrows page, T today, 1–4 the views, N an event.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!live.current) return;
      const typing = (e.target as HTMLElement | null)?.closest("input, textarea, select, [contenteditable]");
      // Find, as everywhere on a Mac: ⌘F (Ctrl+F here) goes to the search field.
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        searchField.current?.focus();
        return;
      }
      if (typing || draft) return;
      const command = (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey;
      const k = e.key.toLowerCase();
      // The picked event: Return opens it, Delete asks and deletes it, ⌘D duplicates it, ⌘C
      // copies it; ⌘V puts the copy on the selected day; Escape lets it go.
      const event = picked ? events.find((x) => x.guid === picked) ?? null : null;
      const plain = !e.metaKey && !e.ctrlKey && !e.altKey;
      const inMenuNow = document.querySelector('[role="menu"], [role="alertdialog"], .modal-veil');
      if (event && !inMenuNow) {
        const textSelected = (window.getSelection()?.toString() ?? "") !== "";
        if (plain && e.key === "Enter") { e.preventDefault(); openPicked(event); return; }
        if (plain && (e.key === "Delete" || e.key === "Backspace")) { e.preventDefault(); void deleteAsking(event); return; }
        if (command && k === "d") { e.preventDefault(); void duplicate(event); return; }
        if (command && k === "c" && !textSelected) { e.preventDefault(); setClip(event); toast(`Copied \u201c${event.title}\u201d`); return; }
        if (plain && e.key === "Escape") { setPicked(null); return; }
      }
      if (command && k === "v" && clip && !inMenuNow) { e.preventDefault(); paste(clip); return; }
      // ⌘Z: the last deletion or change taken back (fields have their own undo, above).
      if (command && k === "z" && !inMenuNow) {
        e.preventDefault();
        const last = lastAct.current;
        lastAct.current = null;
        if (last?.kind === "delete") {
          if (undo?.guid === last.gone.guid) setUndo(null);
          void restore(last.gone);
        } else if (last) void save(last.before, false, true);
        return;
      }
      // ⇧⌘T, Go to Date…, as in Calendar's View menu.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && !e.altKey && k === "t") {
        e.preventDefault();
        setGoingTo(true);
        return;
      }
      // ⌘← and ⌘→ page, and so do the bare arrows wherever they have nothing else to do — a
      // focused day in Month moves its selection with them instead, and has said so already.
      const bare = !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.defaultPrevented &&
        !(e.target as HTMLElement | null)?.closest?.('[role="menu"], [role="listbox"], [role="dialog"], [role="alertdialog"]') &&
        !document.querySelector('[role="menu"]');
      if ((command || bare) && e.key === "ArrowLeft") shift(-1);
      else if ((command || bare) && e.key === "ArrowRight") shift(1);
      // ⌘T, and the bare T this screen always had.
      else if (k === "t" && !e.altKey && !e.shiftKey) setSelected(midnight(new Date()));
      // ⌘1–⌘4: Day, Week, Month, Year, in the order the switch shows them.
      else if (command && /^[1-4]$/.test(e.key)) setView(VIEWS[Number(e.key) - 1].key);
      else if (command && k === "n") newEvent();
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  /** The title, and the shorter one a narrow calendar shows in its place ("Sep 2026"). */
  const title = useMemo(() => {
    const short = (d: Date) => dateText(d, { month: "short" });
    if (view === "year") {
      const year = String(selected.getFullYear());
      return { strong: year, rest: "", brief: year };
    }
    if (view === "day") {
      // The language's own way of writing a date — "October 17, 2026", "17 October 2026" —
      // with the day and month in bold and the year light, as Calendar sets it.
      const parts = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "long", year: "numeric" })
        .formatToParts(selected);
      const at = parts.findIndex((p) => p.type === "year");
      const strong = parts.slice(0, at).map((p) => p.value).join("").replace(/[\s,]+$/, "");
      const sep = parts.slice(0, at).map((p) => p.value).join("").slice(strong.length);
      return {
        strong: `${strong}${sep.trimEnd()}`,
        rest: parts.slice(at).map((p) => p.value).join(""),
        brief: dateText(selected, { day: "numeric", month: "short" }),
      };
    }
    // A week across two months is named for both, as Calendar does: "Sep – Oct 2026".
    if (view === "week" && week[0].getMonth() !== week[6].getMonth()) {
      const sameYear = week[0].getFullYear() === week[6].getFullYear();
      const strong = `${short(week[0])}${sameYear ? "" : ` ${week[0].getFullYear()}`} – ${short(week[6])}`;
      return { strong, rest: String(week[6].getFullYear()), brief: `${strong} ${week[6].getFullYear()}` };
    }
    const base = view === "week" ? week[6] : selected;
    return {
      strong: dateText(base, { month: "long" }),
      rest: String(base.getFullYear()),
      brief: `${short(base)} ${base.getFullYear()}`,
    };
  }, [view, selected, week]);

  /**
   * Every day's items, worked out once for what is shown: a month cell's reminders and timed
   * events in time order, and which days have anything at all — all-day events too, which
   * Month draws as bars and so leaves out of a cell's list. The dots in the mini month and Year
   * read this; with the cell's list a birthday, a holiday or a trip left its days looking empty.
   * Asked of the whole list once per day, Year did it 365 times on every draw.
   */
  const byDay = useMemo(() => {
    const items = new Map<string, Item[]>();
    const busy = new Set<string>();
    const put = (d: Date, item: Item | null) => {
      const key = dayKey(d);
      busy.add(key);
      if (!item) return;
      const list = items.get(key);
      if (list) list.push(item);
      else items.set(key, [item]);
    };
    for (const event of shown) {
      const [first, last] = daysOf(event);
      const timed = !spansDays(event);
      // A year at most: a feed's "forever" event is not walked day by day.
      for (let d = first, n = 0; d <= last && n < 366; d = addDays(d, 1), n++) {
        put(d, timed ? { kind: "event", at: Math.max(event.startsAt, seconds(d)), event } : null);
      }
    }
    for (const reminder of due) {
      if (reminder.dueAt !== null) put(midnight(dateOf(reminder.dueAt)), { kind: "reminder", at: reminder.dueAt, reminder });
    }
    for (const list of items.values()) list.sort((a, b) => a.at - b.at);
    return { items, busy };
  }, [shown, due]);
  const dayItems = useCallback((day: Date) => byDay.items.get(dayKey(day)) ?? NOTHING, [byDay]);
  const busyOn = useCallback((day: Date) => byDay.busy.has(dayKey(day)), [byDay]);

  return (
    <TickContext.Provider value={tickValue}>
    <div className="cx">
      <aside className="cx-side" aria-label="Calendars">
        <div className="cx-cals">
          {[
            { key: "icloud", heading: "iCloud", list: calendars.filter((c) => c.writable) },
            { key: "subs", heading: "Subscriptions", list: calendars.filter((c) => c.guid.startsWith(SUBSCRIPTION_PREFIX)) },
            { key: "other", heading: "Other", list: calendars.filter((c) => !c.writable && !c.guid.startsWith(SUBSCRIPTION_PREFIX)) },
          ]
            .filter((g) => g.list.length > 0 || (g.key === "icloud" && loaded && calendars.length > 0))
            .map((group) => {
              const open = !folded.has(group.key);
              return (
              <Fragment key={group.key}>
                <div className="cx-group" role="group" aria-label={group.heading}>
                  <button
                    className="side-heading cx-side-heading"
                    aria-expanded={open}
                    onClick={() => fold(group.key)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setGroupMenu({ list: group.list, at: { x: e.clientX, y: e.clientY, alignRight: false } });
                    }}
                  >
                    <ChevronDown size={11} strokeWidth={2.6} aria-hidden="true" className="cx-fold" />
                    {group.heading}
                  </button>
                  {group.key === "icloud" && (
                    <button
                      className="icon-button small cx-side-add"
                      aria-label="Add Calendar"
                      title="Add Calendar"
                      aria-haspopup="menu"
                      onClick={(e) => {
                        const r = e.currentTarget.getBoundingClientRect();
                        setAddMenu({ x: r.left, y: r.bottom + 4, alignRight: false });
                      }}
                    >
                      <Plus size={14} strokeWidth={2.2} aria-hidden="true" />
                    </button>
                  )}
                </div>
                {open && group.list.map((c) => {
                  const sub = c.guid.startsWith(SUBSCRIPTION_PREFIX) ? subs.find((x) => SUBSCRIPTION_PREFIX + x.id === c.guid) ?? null : null;
                  return naming === c.guid ? (
                    <CalendarName
                      key={c.guid}
                      initial={c.title}
                      label={`Rename ${c.title}`}
                      onDone={(t) => (sub ? updateSub(sub.id, { title: t.trim() }) : updateCalendar(c, { title: t }))}
                      onCancel={() => setNaming(null)}
                    />
                  ) : (
                    <label
                      className="source-row cx-cal"
                      key={c.guid}
                      title={sub?.problem ? `Not refreshed: ${sub.problem}` : undefined}
                      onContextMenu={(e) => {
                        // A calendar of this account, or a subscription, can be renamed and
                        // recoloured; Birthdays and holidays are as they are.
                        if (!c.writable && !sub) return;
                        e.preventDefault();
                        setCalMenu({ cal: c, at: { x: e.clientX, y: e.clientY, alignRight: false } });
                      }}
                      onDoubleClick={(e) => {
                        if (!c.writable && !sub) return;
                        e.preventDefault();
                        setNaming(c.guid);
                      }}
                    >
                      <input
                        type="checkbox"
                        checked={!hidden.has(c.guid)}
                        onChange={() => toggleCalendar(c.guid)}
                        style={{ "--cal": tint(c) } as React.CSSProperties}
                      />
                      <span className="source-name cx-cal-name">{c.title}</span>
                      {sub?.problem && <span className="cx-cal-problem" aria-label="Not refreshed">!</span>}
                    </label>
                  );
                })}
                {open && group.key === "icloud" && naming === "new" && (
                  <CalendarName initial="Untitled" label="New calendar name" onDone={createCalendar} onCancel={() => setNaming(null)} />
                )}
              </Fragment>
              );
            })}
          {loaded && calendars.length === 0 && (
            <EmptyState className="cal-empty">There are no calendars on this account yet.</EmptyState>
          )}
          {subscribing && (
            <SubscribeForm
              onDone={(sub) => {
                setSubscribing(false);
                toast(`Subscribed to \u201c${sub.title}\u201d`);
                void read();
              }}
              onCancel={() => setSubscribing(false)}
            />
          )}
        </div>
        <MiniMonth
          selected={selected}
          today={now}
          busy={busyOn}
          onPick={setSelected}
          weekNumbers={weekNumbers}
        />
        <AppFoot />
      </aside>

      <section className="cx-main">
        <header className="cx-bar">
          <h1 className="cx-title" aria-label={`${title.strong} ${title.rest}`.trim()}>
            <span className="cx-title-full" aria-hidden="true">
              <strong>{title.strong}</strong> {title.rest}
            </span>
            <strong className="cx-title-brief" aria-hidden="true">
              {title.brief}
            </strong>
          </h1>
          <div className="segmented toolbar-group" role="group" aria-label="View">
            {VIEWS.map((v) => (
              <button
                key={v.key}
                className="segment"
                aria-pressed={view === v.key}
                onClick={() => setView(v.key)}
              >
                {v.label}
              </button>
            ))}
          </div>
          <div className="cx-tools">
            {writable.length > 0 && (
              <span className="toolbar-group cx-quick-anchor">
                <button
                  className="tool cx-icon"
                  aria-label="New event"
                  title="New Event"
                  aria-expanded={quick}
                  onClick={() => setQuick((q) => !q)}
                >
                  <Plus size={17} strokeWidth={2} aria-hidden="true" />
                </button>
                {quick && (
                  <QuickEvent
                    calendar={target}
                    onAdd={(text, details, at) => void addQuick(text, details, at)}
                    onClose={() => setQuick(false)}
                  />
                )}
              </span>
            )}
            <span className="toolbar-group">
              <button
                className="tool cx-icon"
                aria-label="More"
                title="More"
                aria-haspopup="menu"
                onClick={(e) => {
                  const r = e.currentTarget.getBoundingClientRect();
                  setMoreMenu({ x: r.right, y: r.bottom + 4, alignRight: true });
                }}
              >
                <Ellipsis size={17} strokeWidth={2} aria-hidden="true" />
              </button>
            </span>
            <span className="toolbar-group cx-step">
              <button className="tool cx-icon" aria-label="Previous" onClick={() => shift(-1)}>
                <ChevronLeft size={17} strokeWidth={2} aria-hidden="true" />
              </button>
              <button className="tool has-label cx-today" title="Today · Shift-Ctrl-T to go to a date" onClick={() => setSelected(midnight(new Date()))}>
                Today
              </button>
              {goingTo && (
                <form
                  className="popover cx-goto"
                  role="dialog"
                  aria-label="Go to Date"
                  onSubmit={(e) => {
                    e.preventDefault();
                    goTo(new FormData(e.currentTarget).get("day") as string);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") {
                      e.stopPropagation();
                      setGoingTo(false);
                    }
                  }}
                  onBlur={(e) => {
                    if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setGoingTo(false);
                  }}
                >
                  <input
                    className="field"
                    name="day"
                    type="date"
                    aria-label="Date"
                    autoFocus
                    defaultValue={`${selected.getFullYear()}-${String(selected.getMonth() + 1).padStart(2, "0")}-${String(selected.getDate()).padStart(2, "0")}`}
                  />
                  <button type="submit" className="primary">Go</button>
                </form>
              )}
              <button className="tool cx-icon" aria-label="Next" onClick={() => shift(1)}>
                <ChevronRight size={17} strokeWidth={2} aria-hidden="true" />
              </button>
            </span>
            <SearchField
              ref={searchField}
              inToolbar
              className="cx-search"
              placeholder="Search"
              aria-label="Search events"
              value={query}
              onChange={setQuery}
              onKeyDown={(e) => {
                if (e.key === "Escape") setQuery("");
                // Its default kept from the event's own field, which takes the focus as it
                // opens: Return there is Save, and it saved and closed the event at once.
                else if (e.key === "Enter" && found[lit]) { e.preventDefault(); openFound(found[lit]); }
                else if (e.key === "ArrowDown" && found.length) { e.preventDefault(); setLit((i) => Math.min(found.length - 1, i + 1)); }
                else if (e.key === "ArrowUp" && found.length) { e.preventDefault(); setLit((i) => Math.max(0, i - 1)); }
              }}
            />
          </div>
        </header>
        <Unsent refused={outbox.refused} what="a change to an event" onResend={(r) => void outbox.resend(r)} onDiscard={(r) => void outbox.discard(r)} />

        {query.trim() && (
          <div className="popover cx-found" role="listbox" aria-label="Events found" id="cx-found">
            {found.length === 0 ? (
              <p className="cx-found-none">No events match “{query.trim()}”.</p>
            ) : (
              found.map((e, i) => {
                const pastStarts = i > 0 && found[i - 1].endsAt >= Date.now() / 1000 && e.endsAt < Date.now() / 1000;
                return (
                  <Fragment key={e.guid}>
                    {pastStarts && <p className="cx-found-section">Earlier</p>}
                    <button
                      className={`cx-found-row${i === lit ? " is-lit" : ""}`}
                      role="option"
                      aria-selected={i === lit}
                      ref={i === lit ? (el) => el?.scrollIntoView({ block: "nearest" }) : undefined}
                      style={{ "--cal": tint(byGuid.get(e.calendarGuid)) } as React.CSSProperties}
                      onMouseMove={() => i !== lit && setLit(i)}
                      onClick={() => openFound(e)}
                    >
                      <span className="cx-dot" aria-hidden="true" />
                      <span className="cx-found-text">
                        <span className="cx-found-title"><Marked text={e.title} query={query} /></span>
                        {e.location && <span className="cx-found-where"><Marked text={e.location} query={query} /></span>}
                      </span>
                      <span className="cx-found-when">
                        {dateText(dateOf(e.startsAt), { weekday: "short", day: "numeric", month: "short", year: "numeric" })}
                        {e.allDay ? "" : `, ${clock(e.startsAt)}`}
                      </span>
                    </button>
                  </Fragment>
                );
              })
            )}
          </div>
        )}

        {calMenu && (() => {
          const cal = calMenu.cal;
          const sub = subs.find((x) => SUBSCRIPTION_PREFIX + x.id === cal.guid) ?? null;
          const colours = CALENDAR_COLOURS.map(([hex, name]) => ({
            label: name,
            checked: (cal.colour ?? "").toLowerCase() === hex,
            run: () => (sub ? updateSub(sub.id, { colour: hex }) : updateCalendar(cal, { colour: hex })),
          }));
          const alertMenu = (kind: "timed" | "allDay") => {
            const current = defaultAlerts[kind][cal.guid];
            return [
              { label: "None", checked: current === undefined, run: () => setDefaultAlert(kind, cal.guid, null) },
              ...(kind === "allDay" ? ALL_DAY_ALERTS : ALERTS).map((a) => ({
                label: a.label,
                checked: current === a.minutes,
                run: () => setDefaultAlert(kind, cal.guid, a.minutes),
              })),
            ];
          };
          return (
            <RowMenu
              at={calMenu.at}
              onClose={() => setCalMenu(null)}
              entries={sub ? [
                { label: "Rename", run: () => setNaming(cal.guid) },
                { label: "Colour", submenu: colours },
                {
                  label: "Refresh",
                  submenu: REFRESHES.map((r) => ({ label: r.label, checked: sub.refreshMinutes === r.minutes, run: () => updateSub(sub.id, { refreshMinutes: r.minutes }) })),
                },
                { label: "Refresh Now", run: () => refreshSub(sub.id) },
                null,
                { label: "Unsubscribe", danger: true, run: () => unsubscribe(sub) },
              ] : [
                { label: "Rename", run: () => setNaming(cal.guid) },
                { label: "Colour", submenu: colours },
                { label: "Default Alert", submenu: alertMenu("timed") },
                { label: "Default All-Day Alert", submenu: alertMenu("allDay") },
                null,
                ...(cal.isDefault ? [] : [{ label: "Delete Calendar", danger: true, run: () => deleteCalendar(cal) }]),
              ]}
            />
          );
        })()}
        {addMenu && (
          <RowMenu
            at={addMenu}
            onClose={() => setAddMenu(null)}
            entries={[
              { label: "New Calendar", run: () => { if (folded.has("icloud")) fold("icloud"); setNaming("new"); } },
              { label: "New Subscription", run: () => setSubscribing(true) },
            ]}
          />
        )}
        {groupMenu && (
          <RowMenu
            at={groupMenu.at}
            onClose={() => setGroupMenu(null)}
            entries={[
              { label: "Show All", run: () => showOnly(groupMenu.list, true) },
              { label: "Hide All", run: () => showOnly(groupMenu.list, false) },
            ]}
          />
        )}
        {moreMenu && (
          <RowMenu
            at={moreMenu}
            onClose={() => setMoreMenu(null)}
            entries={[
              { label: "Go to Date", run: () => setGoingTo(true) },
              { label: "Show Week Numbers", checked: weekNumbers, run: () => setWeekNumbers(!weekNumbers) },
              null,
              { label: "New Subscription", run: () => setSubscribing(true) },
            ]}
          />
        )}
        {eventMenu && (() => {
          const e = eventMenu.event;
          const mine = Boolean(byGuid.get(e.calendarGuid)?.writable);
          return (
            <RowMenu
              at={eventMenu.at}
              onClose={() => setEventMenu(null)}
              entries={[
                { label: mine ? "Edit" : "Show", run: () => openPicked(e) },
                ...(writable.length > 0 ? [
                  { label: "Duplicate", run: () => void duplicate(e) },
                  { label: "Copy", run: () => setClip(e) },
                ] : []),
                ...(mine && writable.length > 1 ? [{
                  label: "Calendar",
                  submenu: writable.map((c) => ({
                    label: c.title,
                    checked: c.guid === e.calendarGuid,
                    run: () => { if (c.guid !== e.calendarGuid) void save({ ...draftOf(e), calendarGuid: c.guid }); },
                  })),
                }] : []),
                ...(e.location ? [{ label: "Open in Maps", run: () => void follow(mapsHref(e.location ?? "")) }] : []),
                ...(mine ? [null, { label: "Delete", danger: true, run: () => void deleteAsking(e) }] : []),
              ]}
            />
          );
        })()}

        {view === "month" && (
          <MonthView
            days={month}
            selected={selected}
            today={now}
            events={shown}
            calendars={byGuid}
            items={dayItems}
            onSelect={setSelected}
            onOpen={openEvent}
            onCreate={createInMonth}
            onMore={showDay}
            canWrite={canWrite}
            onMoveDays={moveDays}
            onMenu={menuFor}
            picked={picked}
            weekNumbers={weekNumbers}
          />
        )}

        {(view === "week" || view === "day") && (
          <div className={view === "day" ? "cx-dayview" : "cx-weekview"}>
          <TimeGrid
            days={gridDays}
            today={now}
            events={shown}
            reminders={due}
            calendars={byGuid}
            canWrite={canWrite}
            onOpen={openEvent}
            onCreate={newAt}
            onMove={moveEvent}
            onPickDay={showDay}
            pending={pending}
            onMenu={menuFor}
            picked={picked}
            weekNumbers={weekNumbers}
          />
          {view === "day" && (
            <DayPane
              day={selected}
              events={shown}
              reminders={due}
              calendars={byGuid}
              onOpen={openEvent}
              onMenu={menuFor}
              week={weekNumbers ? weekNumber(startOfWeek(selected)) : null}
            >
              {/* As macOS 27's Day view sets it: the month above the day, to step across it. */}
              <MiniMonth
                selected={selected}
                today={now}
                busy={busyOn}
                onPick={setSelected}
                weekNumbers={weekNumbers}
              />
            </DayPane>
          )}
          </div>
        )}

        {view === "year" && (
          <YearView
            year={selected.getFullYear()}
            today={now}
            busy={busyOn}
            onPick={showDay}
          />
        )}
      </section>

      {draft && anchor && (
        <EventPopover
          draft={draft}
          at={anchor}
          calendars={calendars}
          onChange={changeDraft}
          onClose={() => setDraft(null)}
          onDeleteKey={draft.guid ? () => {
            const e = events.find((x) => x.guid === draft.guid);
            if (e) void deleteAsking(e);
          } : undefined}
          onDismiss={dismiss}
          waiting={Boolean(draft.guid && outbox.waiting.has(draft.guid))}
          onSave={() => void save(draft, true)}
          onDelete={draft.guid ? () => void remove(draft.guid as string) : undefined}
          onNudge={draft.guid ? (by) => {
            // Calendar's Control-Option-arrows: the open event moves, stays open, and is saved.
            const zone = zoneOf(draft);
            const shift = (t: number) => ("days" in by ? addDaysIn(zone, t, by.days) : t + by.minutes * 60);
            const moved = { ...draft, startsAt: shift(draft.startsAt), endsAt: shift(draft.endsAt) };
            setDraft(moved);
            void save(moved);
          } : undefined}
          view={view}
        />
      )}

      {asking && (
        <ConfirmSheet<EventScope>
          title="This is a repeating event."
          body={asking.verb === "delete"
            ? "Do you want to delete only this event, this and all future events, or all events in the series?"
            : "Do you want to change only this event, this and all future events, or all events in the series?"}
          choices={asking.verb === "delete"
            ? [
                { label: "Delete Only This Event", value: "this", primary: true, danger: true, focus: true },
                { label: "Delete All Future Events", value: "future", danger: true },
                { label: "Delete All Events", value: "all", danger: true },
              ]
            : [
                { label: "Only This Event", value: "this", primary: true, focus: true },
                { label: "All Future Events", value: "future" },
                { label: "All Events", value: "all" },
              ]}
          onAnswer={(scope) => asking.answer(scope)}
          onCancel={() => asking.answer(null)}
        />
      )}
      {confirmSheet}

    {card && (
      <ReminderCard
        reminder={due.find((r) => r.guid === card.reminder.guid) ?? card.reminder}
        at={card.at}
        list={listNames.get(card.reminder.listGuid) ?? null}
        onClose={closeCard}
      />
    )}
    </div>
    </TickContext.Provider>
  );
}

/* ---- the mini month in the sidebar ------------------------------------------------ */

const MiniMonth = memo(function MiniMonth({
  selected,
  today,
  busy,
  onPick,
  weekNumbers = false,
}: {
  selected: Date;
  today: Date;
  busy: (d: Date) => boolean;
  onPick: (d: Date) => void;
  weekNumbers?: boolean;
}) {
  const [shown, setShown] = useState(() => new Date(selected.getFullYear(), selected.getMonth(), 1));
  useEffect(() => {
    setShown(new Date(selected.getFullYear(), selected.getMonth(), 1));
  }, [selected]);
  const days = monthDays(shown);
  // One stop for the Tab key, as a date picker has: the arrows move a day or a week, across
  // into the next month as they go, and Return or Space picks. Forty-two stops had made the
  // calendar beside it a long way off.
  const [keyed, setKeyed] = useState<Date | null>(null);
  const grid = useRef<HTMLDivElement | null>(null);
  /** Set by an arrow: the day it moved to takes the focus once drawn — never otherwise. */
  const follow = useRef(false);
  const stop = keyed && days.some((d) => sameDay(d, keyed))
    ? keyed
    : days.some((d) => sameDay(d, selected)) ? selected : days.find((d) => d.getMonth() === shown.getMonth())!;
  useLayoutEffect(() => {
    if (!follow.current) return;
    follow.current = false;
    grid.current?.querySelector<HTMLElement>('[tabindex="0"]')?.focus({ preventScroll: true });
  }, [keyed, shown]);
  const onKey = (e: React.KeyboardEvent, d: Date) => {
    const by = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 }[e.key];
    if (by === undefined) return;
    e.preventDefault();
    const next = new Date(d.getFullYear(), d.getMonth(), d.getDate() + by);
    if (!monthDays(shown).some((x) => sameDay(x, next)) || next.getMonth() !== shown.getMonth()) {
      setShown(new Date(next.getFullYear(), next.getMonth(), 1));
    }
    follow.current = true;
    setKeyed(next);
  };
  return (
    <div className="cx-minimonth">
      <div className="cx-minimonth-head">
        <span>{dateText(shown, { month: "long", year: "numeric" })}</span>
        <span>
          <button
            className="icon-button small"
            aria-label="Previous month"
            onClick={() => setShown((m) => new Date(m.getFullYear(), m.getMonth() - 1, 1))}
          >
            <ChevronLeft size={14} strokeWidth={2.2} aria-hidden="true" />
          </button>
          <button
            className="icon-button small"
            aria-label="Next month"
            onClick={() => setShown((m) => new Date(m.getFullYear(), m.getMonth() + 1, 1))}
          >
            <ChevronRight size={14} strokeWidth={2.2} aria-hidden="true" />
          </button>
        </span>
      </div>
      <div
        ref={grid}
        className={`cx-minimonth-grid${weekNumbers ? " has-weeks" : ""}`}
        onBlur={(e) => !e.currentTarget.contains(e.relatedTarget as Node) && setKeyed(null)}
      >
        {weekNumbers && <span className="cx-minimonth-wd" aria-hidden="true" />}
        {days.slice(0, 7).map((d) => (
          <span className="cx-minimonth-wd" key={`wd${d.getDay()}`} aria-hidden="true">
            {weekdayShort(d).slice(0, 1)}
          </span>
        ))}
        {days.map((d, i) => (
          <Fragment key={i}>
          {weekNumbers && i % 7 === 0 && (
            <span className="cx-minimonth-wk" aria-label={`Week ${weekNumber(d)}`}>{weekNumber(d)}</span>
          )}
          <button
            key={i}
            className={`cx-minimonth-day${d.getMonth() !== shown.getMonth() ? " is-out" : ""}${
              sameDay(d, today) ? " is-today" : ""
            }${sameDay(d, selected) ? " is-selected" : ""}${busy(d) ? " is-busy" : ""}`}
            aria-label={dateText(d, { dateStyle: "full" })}
            aria-current={sameDay(d, today) ? "date" : undefined}
            aria-pressed={sameDay(d, selected)}
            tabIndex={sameDay(d, stop) ? 0 : -1}
            onKeyDown={(e) => onKey(e, d)}
            onClick={() => onPick(d)}
          >
            {d.getDate()}
          </button>
          </Fragment>
        ))}
      </div>
    </div>
  );
});

/* ---- month ------------------------------------------------------------------------ */

type Item =
  | { kind: "event"; at: number; event: CalendarEvent }
  | { kind: "reminder"; at: number; reminder: Reminder };

const MonthView = memo(function MonthView({
  days,
  selected,
  today,
  events,
  calendars,
  items,
  onSelect,
  onOpen,
  onCreate,
  onMore,
  canWrite,
  onMoveDays,
  onMenu,
  picked,
  weekNumbers,
}: {
  days: Date[];
  selected: Date;
  today: Date;
  events: CalendarEvent[];
  calendars: Map<string, Calendar>;
  items: (d: Date) => Item[];
  onSelect: (d: Date) => void;
  onOpen: (e: CalendarEvent, at: Anchor) => void;
  onCreate: (d: Date, at: Anchor) => void;
  onMore: (d: Date) => void;
  canWrite: (calendarGuid: string) => boolean;
  /** An event dragged to another day: moved that many days, its clock time kept. */
  onMoveDays: (e: CalendarEvent, days: number) => void;
  onMenu: (e: CalendarEvent, at: { x: number; y: number }) => void;
  picked: string | null;
  weekNumbers: boolean;
}) {
  const listColour = useListColour();
  const showReminder = useShowReminder();

  /*
   * Dragging an event to another day, as in Calendar's Month: pressed and moved a few pixels
   * it is carried, the day under the pointer is marked, and let go there it moves by that
   * many days. Followed from the window, and ended by a move with no button held — WebKitGTK
   * did not always deliver the release (see TimeGrid).
   */
  const [carry, setCarry] = useState<{ guid: string; over: number } | null>(null);
  const carried = useRef<{ event: CalendarEvent; from: number; x: number; y: number; over: number; moved: boolean } | null>(null);
  const swallowClick = useRef(false);
  const cellAt = (x: number, y: number) => {
    // Under whatever is drawn over the day — a bar, the event being carried.
    const el = document.elementsFromPoint(x, y).find((n) => n.hasAttribute("data-cell"));
    return el ? Number(el.getAttribute("data-cell")) : null;
  };
  const startCarry = (e: React.PointerEvent, event: CalendarEvent) => {
    if (e.button !== 0 || !canWrite(event.calendarGuid)) return;
    const from = cellAt(e.clientX, e.clientY);
    if (from === null) return;
    carried.current = { event, from, x: e.clientX, y: e.clientY, over: from, moved: false };
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("blur", cancel);
      window.removeEventListener("keydown", key, true);
    };
    const finish = (commit: boolean) => {
      const c = carried.current;
      carried.current = null;
      stop();
      setCarry(null);
      if (!c?.moved) return;
      // The click that ends a drag is not a click on the event.
      swallowClick.current = true;
      window.setTimeout(() => (swallowClick.current = false), 0);
      if (commit && c.over !== c.from) onMoveDays(c.event, c.over - c.from);
    };
    const move = (ev: PointerEvent) => {
      const c = carried.current;
      if (!c) return;
      if (ev.buttons === 0) return finish(true);
      if (!c.moved && Math.hypot(ev.clientX - c.x, ev.clientY - c.y) < 5) return;
      const first = !c.moved;
      c.moved = true;
      const over = cellAt(ev.clientX, ev.clientY) ?? c.over;
      if (over !== c.over || first) {
        c.over = over;
        setCarry({ guid: c.event.guid, over });
      }
    };
    const up = () => finish(true);
    const cancel = () => finish(false);
    const key = (ev: KeyboardEvent) => {
      if (ev.key === "Escape" && carried.current?.moved) {
        ev.stopPropagation();
        finish(false);
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("blur", cancel);
    window.addEventListener("keydown", key, true);
  };
  const eventHandlers = (event: CalendarEvent) => ({
    onPointerDown: (e: React.PointerEvent) => startCarry(e, event),
    onContextMenu: (e: React.MouseEvent) => {
      e.preventDefault();
      onMenu(event, { x: e.clientX, y: e.clientY });
    },
  });
  const cell = useRef<HTMLButtonElement | null>(null);
  const focused = useRef(false);
  const inMonth = selected.getMonth();
  useEffect(() => {
    if (focused.current) cell.current?.focus();
  }, [selected]);

  const onKey = (e: React.KeyboardEvent, day: Date) => {
    const by = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: 7, ArrowUp: -7 }[e.key];
    if (by !== undefined) {
      e.preventDefault();
      focused.current = true;
      onSelect(addDays(day, by));
    } else if (e.key === "Home") {
      e.preventDefault();
      onSelect(startOfWeek(day));
    } else if (e.key === "End") {
      e.preventDefault();
      onSelect(addDays(startOfWeek(day), 6));
    }
  };

  // How many lines a cell has room for, measured rather than assumed: a small window has
  // room for two, a large one for eight, and a fixed guess spilt items out of the cell.
  const grid = useRef<HTMLDivElement | null>(null);
  const [rows, setRows] = useState(MONTH_ROWS);
  // A cell too narrow for any title shows its day's items as dots, as Calendar does on a
  // small screen; a title cut to two letters and "3 more" squeezed below it said nothing.
  const [dots, setDots] = useState(false);
  const [tiny, setTiny] = useState(false);
  useLayoutEffect(() => {
    const el = grid.current;
    if (!el) return undefined;
    const measure = () => {
      // Out of sight (another screen in front) it has no box: kept as it was, so it comes
      // back as it was rather than with one row a day for a frame.
      if (!el.clientWidth) return;
      const week = el.firstElementChild as HTMLElement | null;
      const height = week?.getBoundingClientRect().height ?? 0;
      const width = (week?.firstElementChild as HTMLElement | null)?.getBoundingClientRect().width ?? 0;
      // Lines 24px apart (calendar.css: each a 24px target, WCAG 2.2).
      setRows(Math.max(1, Math.floor((height - 28) / LINE)));
      // Dots too in a cell too short for one whole line under its date: one line was kept
      // regardless, and its event drawn cut in half.
      const short = height > 0 && height - 28 < LINE;
      setDots((width > 0 && width < DOT_CELL) || short);
      // Too short even for a row of 24px dots: small marks to see, the day itself the thing
      // to press (it opens with its events), so no control is ever drawn half out of its cell.
      setTiny(short);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const weeks = Array.from({ length: 6 }, (_, w) => days.slice(w * 7, w * 7 + 7));
  return (
    <div className="cx-month">
      <div className="cx-weekdays" aria-hidden="true">
        {days.slice(0, 7).map((d) => (
          <span key={d.getDay()}>{weekdayShort(d)}</span>
        ))}
      </div>
      <div className={`cal-grid${carry ? " is-carrying" : ""}`} role="grid" aria-label="Month" ref={grid}>
        {weeks.map((row, w) => {
          const all = weekSpans(events, row[0]);
          // Only the lanes that fit under the dates are drawn: in a short week the rest ran on
          // over the weeks below. With any left over, one line stays free for "N more" — the
          // day opens with every bar in its all-day row.
          const fit = all.some((s) => s.lane >= rows) ? rows - 1 : rows;
          const spans = all.filter((s) => s.lane < fit);
          return (
            <div className="cx-week" role="row" key={w}>
              {row.map((day, i) => {
                const list = items(day);
                // Only the lanes of the bars that cross this day are kept clear in it: a bar
                // at the end of the week had left the days before it room for nothing.
                const lanesHere = Math.max(0, ...spans.filter((s) => s.from <= i && s.to >= i).map((s) => s.lane + 1));
                // Bars over this day that did not fit are counted in its "N more".
                const cut = all.filter((s) => s.lane >= fit && s.from <= i && s.to >= i).length;
                const room = Math.max(0, rows - lanesHere);
                // What fits, and when it does not, one line fewer so "N more" has a line too.
                // As dots, a row of them fits in any cell.
                const visible = dots
                  ? list.slice(0, MOST_DOTS)
                  : list.length > room || cut
                    ? list.slice(0, Math.max(0, room - 1))
                    : list;
                const more = list.length - visible.length + cut;
                const isSelected = sameDay(day, selected);
                const cellIndex = w * 7 + i;
                return (
                  <div
                    className={`cx-day${day.getMonth() !== inMonth ? " is-out" : ""}${
                      isWeekend(day) ? " is-weekend" : ""
                    }${isSelected ? " is-selected" : ""}${carry?.over === cellIndex ? " is-drop" : ""}`}
                    role="gridcell"
                    aria-selected={isSelected}
                    data-cell={cellIndex}
                    key={i}
                  >
                    {weekNumbers && i === 0 && (
                      <span className="cx-weeknum" aria-label={`Week ${weekNumber(day)}`}>{weekNumber(day)}</span>
                    )}
                    <button
                      ref={isSelected ? cell : undefined}
                      className="cx-day-hit"
                      tabIndex={isSelected ? 0 : -1}
                      aria-current={sameDay(day, today) ? "date" : undefined}
                      aria-label={`${dateText(day, { dateStyle: "full" })}${
                        list.length ? `, ${list.length} item${list.length === 1 ? "" : "s"}` : ""
                      }`}
                      onClick={() => onSelect(day)}
                      onDoubleClick={(e) => onCreate(day, { x: e.clientX, y: e.clientY })}
                      onKeyDown={(e) => onKey(e, day)}
                      onBlur={() => (focused.current = false)}
                    />
                    <span className={`cx-daynum${sameDay(day, today) ? " is-today" : ""}`}>
                      {day.getDate() === 1 && day.getMonth() !== inMonth
                        ? dateText(day, { month: "short", day: "numeric" })
                        : day.getDate()}
                    </span>
                    <ul className={`cx-items${dots ? " is-dots" : ""}${tiny ? " is-tiny" : ""}`} {...(tiny ? { inert: "" } : {})} style={{ marginTop: `${lanesHere * LANE}px` }}>
                      {visible.map((item) =>
                        item.kind === "event" ? (
                          <li key={item.event.guid}>
                            <button
                              className={`cx-item${picked === item.event.guid ? " is-picked" : ""}${carry?.guid === item.event.guid ? " is-carried" : ""}`}
                              data-event={item.event.guid}
                              {...eventHandlers(item.event)}
                              style={{ "--cal": tint(calendars.get(item.event.calendarGuid)) } as React.CSSProperties}
                              // Named in full, for the dot that is all a narrow cell shows.
                              aria-label={`${item.event.title}, ${clock(item.event.startsAt)}`}
                              title={dots ? `${item.event.title} · ${clock(item.event.startsAt)}` : undefined}
                              onClick={(e) => !swallowClick.current && onOpen(item.event, { x: e.clientX, y: e.clientY })}
                            >
                              <span className="cx-dot" aria-hidden="true" />
                              <span className="cx-item-title">{item.event.title}</span>
                              <span className="cx-item-time">{clock(item.event.startsAt)}</span>
                            </button>
                          </li>
                        ) : (
                          <li key={item.reminder.guid}>
                            <span
                              className={`cx-item is-reminder${item.reminder.completedAt ? " is-done" : ""}`}
                              style={listColour(item.reminder)}
                              {...showReminder(item.reminder)}
                              title={dots && !item.reminder.allDay ? `${item.reminder.title} · ${clock(item.at)}` : item.reminder.title}
                            >
                              <Ring reminder={item.reminder} quiet />
                              <span className="cx-item-title">{item.reminder.title}</span>
                              {/* A day's reminder has no time: "12 AM" beside it said it had one. */}
                              {!item.reminder.allDay && <span className="cx-item-time">{clock(item.at)}</span>}
                            </span>
                          </li>
                        )
                      )}
                      {more > 0 && (
                        <li>
                          <button className="cx-more" onClick={() => onMore(day)}>
                            {more} more
                          </button>
                        </li>
                      )}
                    </ul>
                  </div>
                );
              })}
              {/* A row holds cells only (ARIA's grid): the week's bars are one more cell, which
                  lays out as if it were not there, so they keep the week as their frame. */}
              {spans.length > 0 && (
              <div className="cx-week-bars" role="gridcell" aria-label="Across days">
              {spans.map((s) => (
                <button
                  key={`${s.event.guid}-${dayKey(row[0])}`}
                  data-event={s.event.guid}
                  {...eventHandlers(s.event)}
                  className={`cx-bar-event${s.continuesLeft ? " from-before" : ""}${
                    s.continuesRight ? " to-after" : ""
                  }${picked === s.event.guid ? " is-picked" : ""}${carry?.guid === s.event.guid ? " is-carried" : ""}`}
                  style={
                    {
                      "--cal": tint(calendars.get(s.event.calendarGuid)),
                      left: `calc(${(s.from / 7) * 100}% + 3px)`,
                      width: `calc(${((s.to - s.from + 1) / 7) * 100}% - 6px)`,
                      top: `${26 + s.lane * LANE}px`,
                    } as React.CSSProperties
                  }
                  onClick={(e) => !swallowClick.current && onOpen(s.event, { x: e.clientX, y: e.clientY })}
                >
                  {s.event.title}
                </button>
              ))}
              </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
});

/* ---- week and day ------------------------------------------------------------------- */

type Drag =
  /** `anchor` is the quarter hour pressed: the drag runs from it up or down, as in Calendar. */
  | { kind: "create"; day: Date; column: number; anchor: number; from: number; to: number }
  /** `column` is the day the event was taken from, so it can be carried only to days shown. */
  /** `startY` is where it was pressed in the grid's own height (the pointer plus the scroll),
   *  so the hours scrolled past while it is held count as hours moved. */
  /** `allDay` while it is held over the all-day row: let go there, it becomes all-day on the
   *  day it is over, as in Calendar. */
  | { kind: "move"; event: CalendarEvent; column: number; startX: number; startY: number; dx: number; dMinutes: number; allDay?: boolean }
  /** Dragging an event's bottom edge: its end moves, its start stays — or, by its top edge
   *  (`top`), its start moves and its end stays. */
  | { kind: "resize"; event: CalendarEvent; startY: number; dMinutes: number; top?: boolean }
  /** A bar in the all-day row carried to other days: `from` and `to` are its first and last
   *  columns shown, and `dx` how many days it has been carried — or, by one of its ends
   *  (`edge`), how many days that end has been stretched. `into` while the whole bar is held
   *  over the hours: the day and quarter hour it would start at, let go there — an hour long,
   *  no longer all-day, as in Calendar. */
  | { kind: "days"; event: CalendarEvent; from: number; to: number; startX: number; dx: number; edge?: "start" | "end"; into?: { column: number; minutes: number } };

const TimeGrid = memo(function TimeGrid({
  days,
  today,
  events,
  reminders,
  calendars,
  canWrite,
  onOpen,
  onCreate,
  onMove,
  onPickDay,
  pending,
  onMenu,
  picked,
  weekNumbers,
}: {
  days: Date[];
  today: Date;
  events: CalendarEvent[];
  reminders: Reminder[];
  calendars: Map<string, Calendar>;
  canWrite: (calendarGuid: string) => boolean;
  onOpen: (e: CalendarEvent, at: Anchor) => void;
  onCreate: (startsAt: number, endsAt: number, allDay: boolean, at: Anchor) => void;
  /** `allDay` given when the event was carried into the all-day row or out of it. */
  onMove: (e: CalendarEvent, startsAt: number, endsAt: number, allDay?: boolean) => void;
  onPickDay: (d: Date) => void;
  /** A new event being named in the popover, held on the grid where it was made — in the
   *  all-day row when it is all-day. */
  pending?: { startsAt: number; endsAt: number; title: string; allDay: boolean } | null;
  onMenu: (e: CalendarEvent, at: { x: number; y: number }) => void;
  picked: string | null;
  weekNumbers: boolean;
}) {
  const listColour = useListColour();
  const showReminder = useShowReminder();
  const scroller = useRef<HTMLDivElement | null>(null);
  const columnsEl = useRef<HTMLDivElement | null>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  const daysKey = days.map((d) => dayKey(d)).join();
  // A bar that runs past the last day shown carries on to the right — in Day, past the one
  // day, not only past the week the layout counts from.
  // A new all-day event being named takes its place among them, so it is seen where it is
  // being made rather than nowhere until it is saved.
  const draftFrom = pending?.allDay ? pending.startsAt : null;
  const draftTo = pending?.allDay ? pending.endsAt : null;
  const spans = useMemo(
    () => weekSpans(
      draftFrom === null || draftTo === null ? events : [...events, {
        guid: PENDING, calendarGuid: "", title: "", location: null, startsAt: draftFrom, endsAt: draftTo,
        allDay: true, timezone: null, url: null, notes: null, etag: null,
      }],
      days[0],
    )
      .filter((s) => s.from < days.length)
      .map((s) => (s.to >= days.length ? { ...s, to: days.length - 1, continuesRight: true } : s)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [events, daysKey, draftFrom, draftTo]
  );
  const eventLanes = Math.max(0, ...spans.map((s) => s.lane + 1));
  // A reminder for a day has no time: it sits in the all-day row under the day's bars, as in
  // Calendar. On the hour grid it was drawn on the midnight line, scrolled out of sight.
  const dayReminders = useMemo(
    () => days.map((d) => reminders.filter((r) => r.allDay && r.dueAt !== null && sameDay(dateOf(r.dueAt), d))),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [reminders, daysKey]
  );
  const reminderLanes = Math.max(0, ...dayReminders.map((l) => l.length));
  // A timed event held over the all-day row is drawn there, in a lane of its own below the
  // rest, so it covers nothing while it is carried.
  const carriedUp = drag?.kind === "move" && drag.allDay ? drag : null;
  const lanes = Math.max(1, eventLanes + reminderLanes + (carriedUp ? 1 : 0));
  /** An all-day bar held down over the hours, and where it would start. */
  const carriedDown = drag?.kind === "days" && drag.into ? drag : null;
  const alldayEl = useRef<HTMLDivElement | null>(null);

  // Opened, it shows the working day rather than midnight — as Calendar does. Only when it
  // opens: stepping to the next week or day keeps the hours scrolled to, as Calendar keeps
  // them — set again on every step, an evening being looked at jumped back to the morning.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const showsToday = days.some((d) => sameDay(d, new Date()));
    const hour = showsToday ? Math.max(0, new Date().getHours() - 2) : 7;
    // A little above the hour, so its label — centred on the line — shows whole.
    el.scrollTop = Math.max(0, hour * HOUR - 12);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const scrolled = () => scroller.current?.scrollTop ?? 0;

  /** Minutes since midnight under the pointer, in the column it is over. */
  const where = (clientX: number, clientY: number) => {
    const el = columnsEl.current;
    if (!el) return null;
    const r = el.getBoundingClientRect();
    const column = Math.min(days.length - 1, Math.max(0, Math.floor(((clientX - r.left) / r.width) * days.length)));
    const minutes = Math.min(24 * 60, Math.max(0, ((clientY - r.top) / HOUR) * 60));
    return { column, minutes };
  };

  /** Beside the day column, not under the pointer: the popover must not cover the event. */
  const besideColumn = (column: number, y: number): Anchor => {
    const r = columnsEl.current?.getBoundingClientRect();
    if (!r) return { x: 0, y };
    const width = r.width / days.length;
    return { x: r.left + (column + 1) * width, y, left: r.left + column * width } as Anchor;
  };

  /*
   * A drag is followed from the window, not from the grid, and it ends on whichever comes
   * first: the button's release, a move with no button held, the window losing focus, or
   * Escape. In the app's WebKitGTK the release did not always reach the grid — a click left a
   * sliver of "New Event" behind that then grew with every move of the mouse — so a release
   * that never arrives is inferred from the next move, where the buttons say it is up.
   */
  const dragRef = useRef<Drag | null>(null);
  const setDragNow = (d: Drag | null) => {
    dragRef.current = d;
    setDrag(d);
  };

  /*
   * The pointer reports every pixel, the grid moves by quarter hours: only a move that lands
   * on a new quarter hour (or day) is drawn. Re-rendering the whole week on every pixel was
   * the stutter of a drag in WebKitGTK.
   */
  const follow = (clientX: number, clientY: number) => {
    const current = dragRef.current;
    const at = where(clientX, clientY);
    if (!current || !at) return;
    let next: Drag;
    if (current.kind === "create") {
      const to = snap(at.minutes);
      next = { ...current, from: Math.min(current.anchor, to), to: Math.max(current.anchor + 15, to) };
      if (next.from === current.from && next.to === current.to) return;
    } else if (current.kind === "resize") {
      // Never shorter than a quarter hour, as Calendar holds it.
      const length = (current.event.endsAt - current.event.startsAt) / 60;
      const by = snap(((clientY + scrolled() - current.startY) / HOUR) * 60);
      next = { ...current, dMinutes: current.top ? Math.min(length - 15, by) : Math.max(15 - length, by) };
      if (next.dMinutes === current.dMinutes) return;
    } else if (current.kind === "days") {
      const el = columnsEl.current;
      const width = el ? el.getBoundingClientRect().width / days.length : 1;
      // Its first day shown kept on the screen, as a timed event is; stretched, an end never
      // past the other one.
      const want = Math.round((clientX - current.startX) / width);
      const dx = current.edge === "end"
        ? Math.min(days.length - 1 - current.to, Math.max(current.from - current.to, want))
        : current.edge === "start"
          ? Math.min(current.to - current.from, Math.max(-current.from, want))
          : Math.min(days.length - 1 - current.from, Math.max(-current.from, want));
      // The whole bar held down over the hours: the quarter hour under the pointer.
      const hours = scroller.current?.getBoundingClientRect();
      const into = !current.edge && hours && clientY > hours.top
        ? { column: at.column, minutes: Math.min(Math.floor(at.minutes / 15) * 15, 23 * 60) }
        : undefined;
      if (dx === current.dx && into?.column === current.into?.column && into?.minutes === current.into?.minutes) return;
      next = { ...current, dx, into };
    } else {
      const el = columnsEl.current;
      const width = el ? el.getBoundingClientRect().width / days.length : 1;
      // Carried only as far as the days on screen: past them it went nowhere visible and
      // landed on a day no one could see.
      const dx = Math.min(days.length - 1 - current.column, Math.max(-current.column, Math.round((clientX - current.startX) / width)));
      // Held up over the all-day row, it would become all-day there.
      const row = alldayEl.current?.getBoundingClientRect();
      const allDay = !!row && clientY < row.bottom;
      next = { ...current, dx, dMinutes: allDay ? current.dMinutes : snap(((clientY + scrolled() - current.startY) / HOUR) * 60), allDay };
      if (next.dx === current.dx && next.dMinutes === current.dMinutes && next.allDay === !!current.allDay) return;
    }
    setDragNow(next);
  };

  const finish = (clientX: number, clientY: number) => {
    const done = dragRef.current;
    if (!done) return;
    setDragNow(null);
    if (done.kind === "create") {
      // A click on empty time does nothing, as in Calendar — only a drag or a double-click
      // makes an event, so looking around the week never opens a form by accident.
      if (done.to - done.from > 15) {
        onCreate(onClockOf(done.day, done.from), onClockOf(done.day, done.to), false, besideColumn(done.column, clientY));
      }
    } else if (done.kind === "days" && done.into) {
      // Let go over the hours: an hour from the quarter hour it was let go at, on that day.
      swallowClick.current = true;
      window.setTimeout(() => (swallowClick.current = false), 0);
      const day = days[done.into.column];
      onMove(done.event, onClockOf(day, done.into.minutes), onClockOf(day, done.into.minutes + 60), false);
    } else if (done.kind === "days") {
      // Let go where it was taken, it was a click: the bar's own click opens it.
      if (done.dx !== 0) {
        swallowClick.current = true;
        window.setTimeout(() => (swallowClick.current = false), 0);
        // Whole days on the event's own clock, never 86,400 seconds (model.ts).
        const zone = zoneOf(done.event);
        onMove(
          done.event,
          done.edge === "end" ? done.event.startsAt : addDaysIn(zone, done.event.startsAt, done.dx),
          done.edge === "start" ? done.event.endsAt : addDaysIn(zone, done.event.endsAt, done.dx),
        );
      }
    } else if (done.kind === "resize") {
      if (done.dMinutes !== 0) {
        if (done.top) onMove(done.event, done.event.startsAt + done.dMinutes * 60, done.event.endsAt);
        else onMove(done.event, done.event.startsAt, done.event.endsAt + done.dMinutes * 60);
      }
    } else if (done.allDay) {
      // Let go over the all-day row: all-day on the day it is over.
      const day = midnight(days[done.column + done.dx]);
      onMove(done.event, seconds(day), seconds(addDays(day, 1)), true);
    } else if (done.dx !== 0 || done.dMinutes !== 0) {
      const shift = done.dMinutes * 60;
      const moved = (t: number) => {
        const d = dateOf(t + shift);
        return seconds(new Date(d.getFullYear(), d.getMonth(), d.getDate() + done.dx, d.getHours(), d.getMinutes()));
      };
      onMove(done.event, moved(done.event.startsAt), moved(done.event.endsAt));
    } else {
      onOpen(done.event, { x: clientX, y: clientY });
    }
  };

  /** The click that ends a bar's drag is not a click on the bar. */
  const swallowClick = useRef(false);

  // The newest handlers, for listeners attached once per drag.
  const handlers = useRef({ follow, finish });
  handlers.current = { follow, finish };
  const dragging = drag !== null;
  const live = useActiveRef();
  useEffect(() => {
    if (!dragging) return undefined;
    // Held near the top or bottom of the hours, they scroll, as Calendar's do — faster the
    // nearer the edge — so an event can be carried to an hour out of sight. The pointer still,
    // the drag follows the hours scrolling under it.
    let last: { x: number; y: number } | null = null;
    // Only once the pointer has really gone somewhere: a press on an event near the edge, the
    // hand not quite still, had scrolled it away under the click.
    let first: { x: number; y: number } | null = null;
    let travelled = false;
    let inZoneSince = 0;
    let frame = 0;
    const edge = () => {
      frame = 0;
      const el = scroller.current;
      // Held over the all-day row, it is going there, not to an earlier hour.
      const held = dragRef.current;
      if (!el || !last || !held || held.kind === "days" || (held.kind === "move" && held.allDay)) return;
      const r = el.getBoundingClientRect();
      const ZONE_PX = 28;
      // Toward the edge it was carried to, never the other way: an event taken near the top
      // and carried across to another day stays at its hour.
      const up = first && last.y < first.y - 8;
      const down = first && last.y > first.y + 8;
      const into = up && last.y < r.top + ZONE_PX ? last.y - (r.top + ZONE_PX) : down && last.y > r.bottom - ZONE_PX ? last.y - (r.bottom - ZONE_PX) : 0;
      if (!into) {
        inZoneSince = 0;
        return;
      }
      // Only once held there a moment, as Calendar waits: an edge dragged to the last hour
      // in view, and let go, had already scrolled the hours and stretched it far past it.
      const now = performance.now();
      inZoneSince ||= now;
      if (now - inZoneSince < 300) {
        frame = requestAnimationFrame(edge);
        return;
      }
      const before = el.scrollTop;
      el.scrollTop += Math.max(-10, Math.min(10, into / 3));
      if (el.scrollTop === before) return;
      handlers.current.follow(last.x, last.y);
      frame = requestAnimationFrame(edge);
    };
    const move = (e: PointerEvent) => {
      if (e.buttons === 0) return handlers.current.finish(e.clientX, e.clientY);
      last = { x: e.clientX, y: e.clientY };
      first ??= last;
      travelled ||= Math.hypot(last.x - first.x, last.y - first.y) > 6;
      handlers.current.follow(e.clientX, e.clientY);
      if (travelled && !frame) frame = requestAnimationFrame(edge);
    };
    // Scrolled by the wheel while held: followed as if the pointer had moved over the hours.
    const scrolledBy = () => last && handlers.current.follow(last.x, last.y);
    const hours = scroller.current;
    hours?.addEventListener("scroll", scrolledBy, { passive: true });
    const up = (e: PointerEvent | MouseEvent) => handlers.current.finish(e.clientX, e.clientY);
    const drop = () => setDragNow(null);
    const key = (e: KeyboardEvent) => live.current && e.key === "Escape" && drop();
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("mouseup", up);
    window.addEventListener("pointercancel", drop);
    window.addEventListener("blur", drop);
    window.addEventListener("keydown", key);
    // A native drag, if one starts anyway, takes the pointer until it ends: the drag here
    // ends with it rather than waiting for a release that will not come.
    window.addEventListener("dragstart", drop);
    window.addEventListener("dragend", drop);
    return () => {
      cancelAnimationFrame(frame);
      hours?.removeEventListener("scroll", scrolledBy);
      window.removeEventListener("dragstart", drop);
      window.removeEventListener("dragend", drop);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("mouseup", up);
      window.removeEventListener("pointercancel", drop);
      window.removeEventListener("blur", drop);
      window.removeEventListener("keydown", key);
    };
  }, [dragging]);

  const onColumnDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest(".cx-block")) return;
    const at = where(e.clientX, e.clientY);
    if (!at) return;
    // No text selection or native drag while making an event.
    e.preventDefault();
    const from = snap(at.minutes) - (snap(at.minutes) === 24 * 60 ? 15 : 0);
    setDragNow({ kind: "create", day: days[at.column], column: at.column, anchor: from, from, to: from + 15 });
  };

  // Each day's events and timed reminders, laid out together so neither is drawn over the other.
  // Worked out once for what is shown, not again for every move of a drag or key typed in the
  // popover; the title of the event being named changes nothing here.
  const pendingFrom = pending && !pending.allDay ? pending.startsAt : null;
  const pendingTo = pending && !pending.allDay ? pending.endsAt : null;
  const layouts = useMemo(() => days.map((d) => {
    const pending = pendingFrom === null || pendingTo === null ? null : { startsAt: pendingFrom, endsAt: pendingTo };
    const due = stackReminders(reminders.filter((r) => r.dueAt !== null && !r.allDay && sameDay(dateOf(r.dueAt), d)));
    const marks = due.map(({ reminder, top }) => ({
      id: reminder.guid,
      top: (top / HOUR) * 60,
      length: (REMINDER_LINE / HOUR) * 60,
    }));
    // The event being named takes its own column too, beside what it overlaps, not over it.
    if (pending && sameDay(dateOf(pending.startsAt), d)) {
      const from = dateOf(pending.startsAt);
      marks.push({
        id: PENDING,
        top: from.getHours() * 60 + from.getMinutes(),
        length: Math.max((pending.endsAt - pending.startsAt) / 60, MIN_BLOCK),
      });
    }
    const laid = placeDayWith(events, d, marks, MIN_BLOCK);
    return {
      events: laid.events,
      marks: laid.marks.filter((m) => m.id !== PENDING),
      pending: laid.marks.find((m) => m.id === PENDING) ?? null,
      reminders: new Map(due.map((x) => [x.reminder.guid, x.reminder])),
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }), [daysKey, events, reminders, pendingFrom, pendingTo]);

  const nowMinutes = today.getHours() * 60 + today.getMinutes();
  const todayColumn = days.findIndex((d) => sameDay(d, today));

  return (
    <div className={`cal-hours${days.length === 1 ? " is-day" : ""}`}>
      <div className="cx-grid-head" style={{ "--cols": days.length } as React.CSSProperties}>
        <span className="cx-gutter cx-head-week" aria-hidden={!weekNumbers}>
          {weekNumbers ? `W${weekNumber(startOfWeek(days[0]))}` : ""}
        </span>
        {days.map((d, i) => (
          <button
            className={`cal-col-head${sameDay(d, today) ? " is-today" : ""}`}
            key={i}
            onClick={() => onPickDay(d)}
            aria-label={dateText(d, { dateStyle: "full" })}
          >
            <span className="cx-col-weekday">{weekdayShort(d)}</span>
            <span className="cx-col-num">{d.getDate()}</span>
          </button>
        ))}
      </div>

      <div className="cx-allday" style={{ "--cols": days.length, height: `${lanes * LANE + 8}px` } as React.CSSProperties}>
        <span className="cx-gutter cx-allday-label">all-day</span>
        <div
          className="cx-allday-lanes"
          ref={alldayEl}
          // A double-click on a day's free part of the row makes an all-day event on it, as in
          // Calendar.
          onDoubleClick={(e) => {
            if ((e.target as HTMLElement).closest(".cx-bar-event, .cx-reminder")) return;
            const r = e.currentTarget.getBoundingClientRect();
            const column = Math.min(days.length - 1, Math.max(0, Math.floor(((e.clientX - r.left) / r.width) * days.length)));
            const day = midnight(days[column]);
            onCreate(seconds(day), seconds(addDays(day, 1)), true, besideColumn(column, e.clientY));
          }}
        >
          {spans.map((s) => {
            if (s.event.guid === PENDING) {
              return (
                <span
                  key={PENDING}
                  className="cx-bar-event is-draft"
                  style={{
                    left: `calc(${(s.from / days.length) * 100}% + 2px)`,
                    width: `calc(${((s.to - s.from + 1) / days.length) * 100}% - 4px)`,
                    top: `${4 + s.lane * LANE}px`,
                  }}
                >
                  {pending?.title.trim() || "New Event"}
                </span>
              );
            }
            const carried = drag?.kind === "days" && drag.event.guid === s.event.guid ? drag : null;
            const from = s.from + (carried && carried.edge !== "end" ? carried.dx : 0);
            const to = Math.min(s.to + (carried && carried.edge !== "start" ? carried.dx : 0), days.length - 1);
            const stretch = (edge: "start" | "end") => (e: React.PointerEvent) => {
              if (e.button !== 0) return;
              e.stopPropagation();
              e.preventDefault();
              setDragNow({ kind: "days", event: s.event, from: s.from, to: s.to, startX: e.clientX, dx: 0, edge });
            };
            const mine = canWrite(s.event.calendarGuid);
            return (
            <button
              key={s.event.guid}
              data-event={s.event.guid}
              onContextMenu={(e) => {
                e.preventDefault();
                onMenu(s.event, { x: e.clientX, y: e.clientY });
              }}
              className={`cx-bar-event${s.continuesLeft ? " from-before" : ""}${s.continuesRight ? " to-after" : ""}${picked === s.event.guid ? " is-picked" : ""}${carried ? " is-moving" : ""}${carried?.into ? " is-away" : ""}`}
              style={
                {
                  "--cal": tint(calendars.get(s.event.calendarGuid)),
                  left: `calc(${(from / days.length) * 100}% + 2px)`,
                  width: `calc(${((to - from + 1) / days.length) * 100}% - 4px)`,
                  top: `${4 + s.lane * LANE}px`,
                } as React.CSSProperties
              }
              // Carried to other days, as a bar in Month is; held still, a click opens it.
              // In Day too: there is no other day to carry it to, but there are the hours.
              onPointerDown={(e) => {
                if (e.button !== 0 || !mine) return;
                e.preventDefault();
                setDragNow({ kind: "days", event: s.event, from: s.from, to: s.to, startX: e.clientX, dx: 0 });
              }}
              onClick={(e) => !swallowClick.current && onOpen(s.event, { x: e.clientX, y: e.clientY })}
            >
              {s.event.title}
              {/* Its ends, to stretch it over more days or fewer, as in Calendar — where they
                  are on screen. */}
              {mine && days.length > 1 && !s.continuesLeft && (
                <span className="cx-bar-grip is-start" aria-hidden="true" onPointerDown={stretch("start")} />
              )}
              {mine && days.length > 1 && !s.continuesRight && (
                <span className="cx-bar-grip is-end" aria-hidden="true" onPointerDown={stretch("end")} />
              )}
            </button>
            );
          })}
          {dayReminders.map((list, column) => list.map((r, k) => (
            <span
              key={r.guid}
              {...showReminder(r)}
              className={`cx-reminder${r.completedAt ? " is-done" : ""}`}
              style={{
                ...listColour(r),
                top: `${4 + (eventLanes + k) * LANE + 2}px`,
                left: `calc(${(column / days.length) * 100}% + 4px)`,
                width: `calc(${100 / days.length}% - 8px)`,
                maxWidth: `calc(${100 / days.length}% - 8px)`,
              }}
            >
              <Ring reminder={r} quiet />
              <span className="cx-reminder-title">{r.title}</span>
            </span>
          )))}
          {carriedUp && (
            <span
              className="cx-bar-event is-moving is-carried"
              style={
                {
                  "--cal": tint(calendars.get(carriedUp.event.calendarGuid)),
                  left: `calc(${((carriedUp.column + carriedUp.dx) / days.length) * 100}% + 2px)`,
                  width: `calc(${100 / days.length}% - 4px)`,
                  top: `${4 + (eventLanes + reminderLanes) * LANE}px`,
                } as React.CSSProperties
              }
            >
              {carriedUp.event.title}
            </span>
          )}
        </div>
      </div>

      <div className="cx-scroll" ref={scroller}>
        <div className="cx-hours-body" style={{ height: `${24 * HOUR}px` }}>
          <div className="cx-gutter cx-hour-labels" aria-hidden="true">
            {Array.from({ length: 24 }, (_, h) =>
              // An hour the red "now" label sits over is left out, as Calendar does, rather
              // than drawn underneath it.
              // Measured in pixels, since that is what overlaps: two labels about 14px tall
              // touch until they are 18px apart, whatever an hour is drawn as.
              h === 0 || (todayColumn >= 0 && (Math.abs(h * 60 - nowMinutes) / 60) * HOUR < 18) ? null : (
                <span key={h} style={{ top: `${h * HOUR}px` }}>
                  {hourLabel(h)}
                </span>
              )
            )}
            {todayColumn >= 0 && (
              <span className="cx-now-label" style={{ top: `${Math.round((nowMinutes / 60) * HOUR)}px` }}>
                {clock(seconds(today))}
              </span>
            )}
          </div>
          <div
            className="cx-columns"
            ref={columnsEl}
            style={{ "--cols": days.length } as React.CSSProperties}
            onPointerDown={onColumnDown}
            // A press on the grid starts making an event, never a text selection or a native
            // drag: in WebKitGTK either would take the pointer and the events that end the
            // drag with it.
            onMouseDown={(e) => e.preventDefault()}
            onDragStart={(e) => e.preventDefault()}
            onDoubleClick={(e) => {
              if ((e.target as HTMLElement).closest(".cx-block")) return;
              const at = where(e.clientX, e.clientY);
              if (!at) return;
              const start = Math.min(snap(Math.floor(at.minutes / 30) * 30), 23 * 60);
              const day = days[at.column];
              onCreate(onClockOf(day, start), onClockOf(day, start + 60), false, besideColumn(at.column, e.clientY));
            }}
          >
            {days.map((d, column) => (
              <div className={`cx-column${isWeekend(d) ? " is-weekend" : ""}`} key={column}>
                {layouts[column].events.map((p) => {
                  const moving = drag?.kind === "move" && drag.event.guid === p.event.guid ? drag : null;
                  const sizing = drag?.kind === "resize" && drag.event.guid === p.event.guid ? drag : null;
                  const c = calendars.get(p.event.calendarGuid);
                  return (
                    <button
                      key={p.event.guid}
                      data-event={p.event.guid}
                      className={`cx-block${moving || sizing ? " is-moving" : ""}${moving?.allDay ? " is-away" : ""}${p.length < 45 ? " is-short" : ""}${picked === p.event.guid ? " is-picked" : ""}`}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        onMenu(p.event, { x: e.clientX, y: e.clientY });
                      }}
                      style={
                        {
                          "--cal": tint(c),
                          top: `${((p.top + (moving?.dMinutes ?? 0) + (sizing?.top ? sizing.dMinutes : 0)) / 60) * HOUR}px`,
                          height: `${Math.max(((p.length + (sizing ? (sizing.top ? -sizing.dMinutes : sizing.dMinutes) : 0)) / 60) * HOUR - 2, 20)}px`,
                          left: `calc(${(p.column / p.columns) * 100}% + ${(moving?.dx ?? 0) * 100}%)`,
                          width: `calc(${100 / p.columns}% - 3px)`,
                        } as React.CSSProperties
                      }
                      onPointerDown={(e) => {
                        if (e.button !== 0) return;
                        e.stopPropagation();
                        e.preventDefault();
                        if (canWrite(p.event.calendarGuid)) {
                          setDragNow({ kind: "move", event: p.event, column, startX: e.clientX, startY: e.clientY + scrolled(), dx: 0, dMinutes: 0 });
                        } else {
                          onOpen(p.event, { x: e.clientX, y: e.clientY });
                        }
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          onOpen(p.event, pointOf(e.currentTarget));
                        }
                      }}
                    >
                      <span className="cx-block-title">{p.event.title}</span>
                      <span className="cx-block-time">{clock(p.event.startsAt)}</span>
                      {p.event.location && <span className="cx-block-where">{p.event.location}</span>}
                      {canWrite(p.event.calendarGuid) && p.length >= 30 && (
                        <span
                          className="cx-block-grip is-top"
                          aria-hidden="true"
                          onPointerDown={(e) => {
                            if (e.button !== 0) return;
                            e.stopPropagation();
                            e.preventDefault();
                            setDragNow({ kind: "resize", event: p.event, startY: e.clientY + scrolled(), dMinutes: 0, top: true });
                          }}
                        />
                      )}
                      {canWrite(p.event.calendarGuid) && (
                        <span
                          className="cx-block-grip"
                          aria-hidden="true"
                          onPointerDown={(e) => {
                            if (e.button !== 0) return;
                            e.stopPropagation();
                            e.preventDefault();
                            setDragNow({ kind: "resize", event: p.event, startY: e.clientY + scrolled(), dMinutes: 0 });
                          }}
                        />
                      )}
                    </button>
                  );
                })}
                {layouts[column].marks.map((m) => {
                  const r = layouts[column].reminders.get(m.id)!;
                  return (
                    <span
                      key={r.guid}
                      {...showReminder(r)}
                      className={`cx-reminder${r.completedAt ? " is-done" : ""}`}
                      style={{
                        ...listColour(r),
                        top: `${(m.top / 60) * HOUR}px`,
                        left: `calc(${(m.column / m.columns) * 100}% + 4px)`,
                        maxWidth: `calc(${100 / m.columns}% - 8px)`,
                      }}
                    >
                      <Ring reminder={r} quiet />
                      <span className="cx-reminder-title">{r.title}</span>
                    </span>
                  );
                })}
                {pending && !drag && layouts[column].pending && (
                  <span
                    className="cx-block is-draft"
                    style={{
                      top: `${(layouts[column].pending!.top / 60) * HOUR}px`,
                      height: `${Math.max((layouts[column].pending!.length / 60) * HOUR - 2, 20)}px`,
                      left: `${(layouts[column].pending!.column / layouts[column].pending!.columns) * 100}%`,
                      width: `calc(${100 / layouts[column].pending!.columns}% - 3px)`,
                    }}
                  >
                    <span className="cx-block-title">{pending.title.trim() || "New Event"}</span>
                    <span className="cx-block-time">{clock(pending.startsAt)}</span>
                  </span>
                )}
                {/* An all-day bar held over the hours: the hour it would become. */}
                {carriedDown?.into?.column === column && (
                  <span
                    className="cx-block is-moving is-carried"
                    style={
                      {
                        "--cal": tint(calendars.get(carriedDown.event.calendarGuid)),
                        top: `${(carriedDown.into.minutes / 60) * HOUR}px`,
                        height: `${HOUR - 2}px`,
                      } as React.CSSProperties
                    }
                  >
                    <span className="cx-block-title">{carriedDown.event.title}</span>
                    <span className="cx-block-time">{clock(onClockOf(d, carriedDown.into.minutes))}</span>
                  </span>
                )}
                {/* Drawn once the drag is one, never for a press held still: a
                    quarter-hour sliver that release then discards reads as a bug. */}
                {drag?.kind === "create" && drag.column === column && drag.to - drag.from > 15 && (
                  <span
                    className="cx-block is-draft"
                    style={{ top: `${(drag.from / 60) * HOUR}px`, height: `${((drag.to - drag.from) / 60) * HOUR}px` }}
                  >
                    <span className="cx-block-title">New Event</span>
                  </span>
                )}
              </div>
            ))}
            {todayColumn >= 0 && (
              <span
                className="cx-now"
                aria-hidden="true"
                style={{
                  // On a whole pixel, so the 2px line is two rows and not three half-lit ones.
                  top: `${Math.round((nowMinutes / 60) * HOUR)}px`,
                  left: `${(todayColumn / days.length) * 100}%`,
                  width: `${100 / days.length}%`,
                }}
              />
            )}
          </div>
        </div>
      </div>
    </div>
  );
});

/**
 * Where each reminder sits on a day's hour grid. Two due at the same time — a morning routine
 * set for eight — stack one under the other instead of being drawn over each other.
 */
const REMINDER_LINE = 18;
/** The shortest a block is drawn — 20px, room for one line — in minutes, and so laid out. */
const MIN_BLOCK = ((20 + 2) / HOUR) * 60;
/** The layout's name for the event being named, which has no guid yet. */
const PENDING = "\u0000pending";
export function stackReminders(list: Reminder[]): { reminder: Reminder; top: number }[] {
  const LINE = REMINDER_LINE;
  const sorted = [...list].sort((a, b) => (a.dueAt ?? 0) - (b.dueAt ?? 0) || a.title.localeCompare(b.title));
  let lastBottom = -Infinity;
  return sorted.map((reminder) => {
    const at = dateOf(reminder.dueAt ?? 0);
    const wanted = ((at.getHours() * 60 + at.getMinutes()) / 60) * HOUR;
    const top = Math.max(wanted, lastBottom);
    lastBottom = top + LINE;
    return { reminder, top };
  });
}

/** The moment `minutes` on the clock into `day` — the grid's own measure. Added to midnight as
 *  seconds, an event made after the clock moved landed an hour off where it was drawn. */
const onClockOf = (day: Date, minutes: number) =>
  seconds(new Date(day.getFullYear(), day.getMonth(), day.getDate(), 0, minutes));

const pointOf = (el: Element): Anchor => {
  const r = el.getBoundingClientRect();
  return { x: r.right, y: r.top };
};

/* ---- year --------------------------------------------------------------------------- */

const YearView = memo(function YearView({
  year,
  today,
  busy,
  onPick,
}: {
  year: number;
  today: Date;
  busy: (d: Date) => boolean;
  onPick: (d: Date) => void;
}) {
  // One Tab stop for the year's 365 days, the arrows moving a day or a week through it.
  const [at, setAt] = useState<Date>(() => (today.getFullYear() === year ? today : new Date(year, 0, 1)));
  const held = at.getFullYear() === year ? at : today.getFullYear() === year ? today : new Date(year, 0, 1);
  const box = useRef<HTMLDivElement | null>(null);
  const keyed = useRef(false);
  useLayoutEffect(() => {
    if (keyed.current) box.current?.querySelector<HTMLElement>('.cx-minimonth-day[tabindex="0"]')?.focus({ preventScroll: false });
    keyed.current = false;
  }, [at]);
  const onKey = (e: React.KeyboardEvent, d: Date) => {
    const by = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 }[e.key];
    if (by === undefined) return;
    e.preventDefault();
    const next = new Date(d.getFullYear(), d.getMonth(), d.getDate() + by);
    if (next.getFullYear() !== year) return;
    keyed.current = true;
    setAt(next);
  };
  return (
    <div className="cal-year" role="group" aria-label={`${year}`} ref={box}>
      {Array.from({ length: 12 }, (_, m) => {
        const first = new Date(year, m, 1);
        const days = monthDays(first);
        const current = today.getFullYear() === year && today.getMonth() === m;
        return (
          <section className="cal-mini" key={m} aria-label={dateText(first, { month: "long" })}>
            <h2 className={`cx-year-month${current ? " is-current" : ""}`}>
              {dateText(first, { month: "long" })}
            </h2>
            <div className="cx-minimonth-grid">
              {days.slice(0, 7).map((d) => (
                <span className="cx-minimonth-wd" key={`wd${d.getDay()}`} aria-hidden="true">
                  {weekdayShort(d).slice(0, 1)}
                </span>
              ))}
              {days.map((d, i) =>
                d.getMonth() !== m ? (
                  <span key={i} className="cx-minimonth-day is-blank" aria-hidden="true" />
                ) : (
                  <button
                    key={i}
                    className={`cx-minimonth-day${sameDay(d, today) ? " is-today" : ""}${busy(d) ? " is-busy" : ""}`}
                    aria-label={dateText(d, { dateStyle: "full" })}
                    aria-current={sameDay(d, today) ? "date" : undefined}
                    tabIndex={sameDay(d, held) ? 0 : -1}
                    onKeyDown={(e) => onKey(e, d)}
                    onClick={() => onPick(d)}
                  >
                    {d.getDate()}
                  </button>
                )
              )}
            </div>
          </section>
        );
      })}
    </div>
  );
});

/* ---- the event popover ---------------------------------------------------------------- */

/** A date and, unless it is all day, a time — two compact fields, as Calendar lays them out —
 * read on a clock in `zone`, the event's own. */
function When({
  at,
  allDay,
  editable,
  label,
  zone,
  onChange,
}: {
  at: number;
  allDay: boolean;
  editable: boolean;
  label: string;
  zone: string;
  onChange: (s: number) => void;
}) {
  const w = wallIn(zone, at);
  const day = `${w.y}-${pad(w.m)}-${pad(w.d)}`;
  const at_ = (date: string, h: number, min: number) => {
    const [y, m, d] = date.split("-").map(Number);
    return instantIn(zone, { y, m, d, h, min });
  };
  return (
    <span className="cx-when">
      <input
        type="date"
        aria-label={`${label} date`}
        value={day}
        readOnly={!editable}
        onChange={(e) => e.target.value && onChange(at_(e.target.value, allDay ? 0 : w.h, allDay ? 0 : w.min))}
      />
      {!allDay && (
        <TimeField
          label={`${label} time`}
          hours={w.h}
          minutes={w.min}
          disabled={!editable}
          onChange={(h, m) => onChange(at_(day, h, m))}
        />
      )}
    </span>
  );
}

/**
 * An event's time zone, as Calendar sets one: its city and offset shown, and a field that
 * finds any zone by city or region as it is typed. A zone chosen keeps the event's clock time
 * (9:00 stays 9:00, now in that zone), as Calendar does.
 */
function ZoneField({ zone, at, editable, onChange }: { zone: string; at: number; editable: boolean; onChange: (z: string) => void }) {
  const [query, setQuery] = useState<string | null>(null);
  const [lit, setLit] = useState(0);
  const named = zoneName(zone, at);
  const found = useMemo(() => {
    if (query === null) return [];
    const q = query.trim().toLowerCase().replace(/\s+/g, "_");
    const all = allZones();
    if (!q) return [ZONE, ...all.filter((z) => z !== ZONE)].slice(0, 8);
    // Cities that begin with what is typed first, then a word in one that does, then the rest.
    const rank = (z: string) => {
      const city = (z.split("/").pop() ?? z).toLowerCase();
      if (city.startsWith(q)) return 0;
      if (city.split(/[_-]/).some((w) => w.startsWith(q))) return 1;
      return z.toLowerCase().includes(q) ? 2 : 3;
    };
    return all
      .map((z) => [z, rank(z)] as const)
      .filter(([, r]) => r < 3)
      .sort((a, b) => a[1] - b[1] || (a[0].split("/").pop() ?? "").localeCompare(b[0].split("/").pop() ?? ""))
      .map(([z]) => z)
      .slice(0, 8);
  }, [query]);
  const choose = (z: string) => {
    setQuery(null);
    if (z !== zone) onChange(z);
  };
  if (query === null) {
    return (
      <button
        type="button"
        className="cx-zone"
        aria-label={`Time zone: ${named.city}, ${named.offset}`}
        disabled={!editable}
        onClick={() => { setQuery(""); setLit(0); }}
      >
        <span>{named.city}</span>
        <span className="cx-zone-offset">{named.offset}</span>
      </button>
    );
  }
  return (
    <span className="cx-zone-find">
      <input
        autoFocus
        type="search"
        role="combobox"
        aria-label="Find a time zone"
        aria-expanded={found.length > 0}
        aria-controls="cx-zones"
        placeholder="City or region"
        value={query}
        onChange={(e) => { setQuery(e.target.value); setLit(0); }}
        onBlur={() => setQuery(null)}
        onKeyDown={(e) => {
          if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); setQuery(null); }
          else if (e.key === "ArrowDown") { e.preventDefault(); setLit((i) => Math.min(found.length - 1, i + 1)); }
          else if (e.key === "ArrowUp") { e.preventDefault(); setLit((i) => Math.max(0, i - 1)); }
          else if (e.key === "Enter" && found[lit]) { e.preventDefault(); choose(found[lit]); }
        }}
      />
      {found.length > 0 && (
        <span className="cx-zones" id="cx-zones" role="listbox" aria-label="Time zones">
          {found.map((z, i) => {
            const n = zoneName(z, at);
            return (
              <span
                key={z}
                role="option"
                aria-selected={i === lit}
                className={`cx-zone-option${i === lit ? " is-lit" : ""}`}
                // Chosen before the field loses focus and closes the list.
                onMouseDown={(e) => { e.preventDefault(); choose(z); }}
                onMouseMove={() => setLit(i)}
              >
                <span>{n.city}</span>
                <span className="cx-zone-region">{z.includes("/") ? z.split("/")[0].replace(/_/g, " ") : ""}</span>
                <span className="cx-zone-offset">{n.offset}</span>
              </span>
            );
          })}
        </span>
      )}
    </span>
  );
}

/**
 * A scheduled reminder's card, as Calendar shows one when it is clicked: its whole title —
 * which a cell cuts short — when it is due, its list and its notes, with its ring to complete
 * it and a way to it in Reminders. Put away by Escape, a click elsewhere, or once it is done.
 */
function ReminderCard({ reminder, at, list, onClose }: {
  reminder: Reminder;
  at: Anchor;
  list: string | null;
  onClose: () => void;
}) {
  const box = useRef<HTMLDivElement | null>(null);
  const ctx = useContext(TickContext);
  const listColour = useListColour();
  useReturnFocus(() => null);
  const [pos, setPos] = useState<{ left: number; top: number }>({ left: at.x, top: at.y });
  // Beside what was clicked, and always wholly on screen — as an event's popover is placed.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const gap = 12;
    let left = at.x + gap;
    if (left + width > window.innerWidth - 8) left = (at.left ?? at.x) - width - gap;
    if (left < 8) left = Math.max(8, window.innerWidth - width - 8);
    const top = Math.min(Math.max(8, at.y - 24), window.innerHeight - height - 8);
    setPos({ left, top });
  }, [at]);
  const live = useActiveRef();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (live.current && e.key === "Escape") onClose();
    };
    const onDown = (e: PointerEvent) => {
      if (box.current?.contains(e.target as Node)) return;
      onClose();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("pointerdown", onDown, true);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("pointerdown", onDown, true);
    };
  }, [onClose, live]);
  useEffect(() => {
    box.current?.focus();
  }, []);
  // Completed from its ring: it leaves the calendar, and the card with it.
  const done = ctx?.ticked.has(reminder.guid) ?? false;
  useEffect(() => {
    if (!done) return;
    const t = window.setTimeout(onClose, 600);
    return () => window.clearTimeout(t);
  }, [done, onClose]);
  const due = reminder.dueAt === null ? null : dateOf(reminder.dueAt);
  const when = due
    ? reminder.allDay
      // Its day as read in (`onItsDay`): already this clock's midnight. Read again in UTC, it
      // named the day before east of Greenwich.
      ? dateText(due, { weekday: "long", day: "numeric", month: "long", year: "numeric" })
      : `${dateText(due, { weekday: "long", day: "numeric", month: "long", year: "numeric" })}, ${timeText(due, { hour: "numeric", minute: "2-digit" })}`
    : null;
  return (
    <Portal>
      <div
        ref={box}
        className="cx-popover is-info is-reminder"
        role="dialog"
        aria-label={reminder.title}
        tabIndex={-1}
        style={{ left: pos.left, top: pos.top, ...listColour(reminder) }}
      >
        <div className="cx-rcard-head">
          <Ring reminder={reminder} />
          <h3 className="cx-info-title">{reminder.title}</h3>
        </div>
        <div className="cx-rcard-body">
        {when && <p className="cx-info-line">{when}</p>}
        {list && <p className="cx-info-cal"><span className="cx-info-dot" aria-hidden="true" />{list}</p>}
        {reminder.notes && <p className="cx-info-notes">{reminder.notes}</p>}
        <button
          type="button"
          className="cx-rcard-open"
          onClick={() => {
            onClose();
            reveal({ screen: "reminders", id: reminder.guid, within: reminder.listGuid });
            window.dispatchEvent(new CustomEvent("quince:revealed", { detail: { screen: "reminders" } }));
          }}
        >
          Show in Reminders
        </button>
        </div>
      </div>
    </Portal>
  );
}

function EventPopover({
  draft,
  at,
  calendars,
  onChange,
  onClose,
  onDismiss,
  onSave,
  onDelete,
  onDeleteKey,
  onNudge,
  view,
  waiting = false,
}: {
  draft: Draft;
  at: Anchor;
  calendars: Calendar[];
  onChange: (d: Draft) => void;
  onClose: () => void;
  /** Put away by a click outside it. */
  onDismiss: () => void;
  onSave: () => void;
  onDelete?: () => void;
  /** Delete pressed outside the fields: asked first. */
  onDeleteKey?: () => void;
  onNudge?: (by: { days: number } | { minutes: number }) => void;
  view?: string;
  /** Saved here and not yet at Apple. */
  waiting?: boolean;
}) {
  const box = useRef<HTMLFormElement | null>(null);
  // Put away, the focus goes back to the event it was open for, or the day it was made on.
  useReturnFocus(() =>
    (draft.guid ? document.querySelector<HTMLElement>(`[data-event="${CSS.escape(draft.guid)}"]`) : null) ??
    document.querySelector<HTMLElement>('.cx-day-hit[tabindex="0"], .cal-col-head[tabindex="0"]'),
  );
  const [pos, setPos] = useState<{ left: number; top: number }>({ left: at.x, top: at.y });
  const calendar = calendars.find((c) => c.guid === draft.calendarGuid);
  const editable = !draft.guid || (calendar?.writable ?? false);
  const choices = calendars.filter((c) => c.writable || c.guid === draft.calendarGuid);

  // Beside what was clicked, and always wholly on screen.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const gap = 12;
    let left = at.x + gap;
    if (left + width > window.innerWidth - 8) left = (at.left ?? at.x) - width - gap;
    // Neither side free — a block as wide as Day's column — so over its far end, clear of
    // its title, which begins at the other.
    if (left < 8) left = Math.max(8, window.innerWidth - width - 8);
    const top = Math.min(Math.max(8, at.y - 24), window.innerHeight - height - 8);
    setPos({ left, top });
  }, [at]);

  const live = useActiveRef();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!live.current) return;
      if (e.key === "Escape") return onClose();
      // Up/Down: fifteen minutes in Day and Week, a week in Month. Left/Right: a day.
      if (e.ctrlKey && e.altKey && onNudge && editable && e.key.startsWith("Arrow")) {
        // A day is a calendar day on the event's clock, never a fixed 86,400 seconds.
        const week = view === "month" || draft.allDay;
        const by = {
          ArrowUp: week ? { days: -7 } : { minutes: -15 },
          ArrowDown: week ? { days: 7 } : { minutes: 15 },
          ArrowLeft: { days: -1 },
          ArrowRight: { days: 1 },
        }[e.key];
        if (by) {
          e.preventDefault();
          onNudge(by);
        }
        return;
      }
      // Delete, as in Calendar: an open event goes when Delete is pressed outside its fields —
      // and Undo brings it back, since iCloud keeps no bin.
      const typing = (e.target as HTMLElement | null)?.closest("input, textarea, select, [contenteditable]");
      if ((e.key === "Delete" || e.key === "Backspace") && !typing && (onDeleteKey ?? onDelete) && editable) {
        e.preventDefault();
        (onDeleteKey ?? onDelete)?.();
      }
    };
    const onDown = (e: PointerEvent) => {
      if (!box.current || box.current.contains(e.target as Node) || inMenu(e.target)) return;
      // Calendar's question about a repeating event is this popover's own, asked on its Save.
      if ((e.target as HTMLElement).closest?.(".modal-veil")) return;
      // A press on the event this is open for is not a press elsewhere: it keeps it open.
      if (draft.guid && (e.target as HTMLElement).closest?.(`[data-event="${CSS.escape(draft.guid)}"]`)) return;
      onDismiss();
    };
    window.addEventListener("keydown", onKey);
    // A press, not a mouse-down: the timeline cancels its presses' default to keep text from
    // being selected, and a cancelled pointer press sends no mouse-down — a click on empty time
    // left the popover open. Caught on the way down, before an event block stops it: a press
    // on another event puts this one away — keeping what was typed — before that one opens.
    window.addEventListener("pointerdown", onDown, true);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("pointerdown", onDown, true);
    };
  }, [onClose, onDismiss, onDelete, onDeleteKey, onNudge, editable, view, draft.allDay, draft.guid]);

  const set = (patch: Partial<Draft>) => onChange({ ...draft, ...patch });

  // A holiday, a birthday, a subscribed feed: nothing to change, so Calendar shows what it
  // is — not a form of greyed fields with a note saying they cannot be used.
  if (!editable) {
    const start = dateOf(draft.startsAt);
    // An all-day event's last day is the one before its end midnight — a second before it, not a
    // fixed 86,400 seconds, which across the clock's spring change named the day before.
    const end = dateOf(Math.max(draft.startsAt, draft.endsAt - (draft.allDay ? 1 : 0)));
    const day = (d: Date) => dateText(d, { weekday: "long", day: "numeric", month: "long", year: "numeric" });
    const time = (d: Date) => timeText(d, { hour: "numeric", minute: "2-digit" });
    const sameDay = start.toDateString() === end.toDateString();
    const when = draft.allDay
      ? sameDay ? day(start) : `${day(start)} – ${day(end)}`
      : sameDay ? `${day(start)}, ${time(start)} – ${time(end)}` : `${day(start)}, ${time(start)} – ${day(end)}, ${time(end)}`;
    return (
      <Portal>
      <form
        ref={box}
        className="cx-popover is-info"
        role="dialog"
        aria-label={draft.title || "Event"}
        style={{ left: pos.left, top: pos.top, "--cal": tint(calendar) } as React.CSSProperties}
        onSubmit={(e) => { e.preventDefault(); onClose(); }}
      >
        <h3 className="cx-info-title">{draft.title}</h3>
        <JoinButton event={draft} />
        {draft.location && (
          <p className="cx-info-line cx-info-place">
            {draft.location}
            <a className="cx-maps" href={mapsHref(draft.location)} title="Open in Maps">Open in Maps</a>
          </p>
        )}
        <p className="cx-info-line">{when}</p>
        <p className="cx-info-cal"><span className="cx-info-dot" aria-hidden="true" />{calendar?.title}</p>
        {/* An invitation's link is the sender's to choose: only a kind Skykeep opens is a link
            (lib/links.ts); any other is its text. */}
        {draft.url && (outsideHref(draft.url) ? (
          <a className="cx-info-url" href={outsideHref(draft.url)!} target="_blank" rel="noreferrer">{draft.url}</a>
        ) : (
          <p className="cx-info-url" title={REFUSED_LINK}>{draft.url}</p>
        ))}
        <Guests guid={draft.guid} />
        {draft.notes && <p className="cx-info-notes">{draft.notes}</p>}
      </form>
      </Portal>
    );
  }

  return (
    <Portal>
    <form
      ref={box}
      className="cx-popover"
      role="dialog"
      aria-label={draft.guid ? draft.title || "Event" : "New event"}
      style={{ left: pos.left, top: pos.top, "--cal": tint(calendar) } as React.CSSProperties}
      onSubmit={(e) => {
        e.preventDefault();
        if (editable) onSave();
      }}
    >
      <input
        className="cx-pop-title"
        value={draft.title}
        placeholder="New Event"
        aria-label="Title"
        autoFocus
        readOnly={!editable}
        onChange={(e) => set({ title: e.target.value })}
      />
      <JoinButton event={draft} />
      <span className="cx-pop-place">
        <input
          className="cx-pop-line"
          value={draft.location}
          placeholder="Add Location"
          aria-label="Location"
          readOnly={!editable}
          onChange={(e) => set({ location: e.target.value })}
        />
        {draft.location.trim() && (
          <a className="cx-pop-link" href={mapsHref(draft.location.trim())} aria-label="Open in Maps" title="Open in Maps">
            <MapPin size={14} strokeWidth={2} aria-hidden="true" />
          </a>
        )}
      </span>
      <div className="cx-pop-rows">
        <label className="cx-pop-row">
          <span>all-day</span>
          <input
            type="checkbox"
            checked={draft.allDay}
            disabled={!editable}
            onChange={(e) => {
              const allDay = e.target.checked;
              const day = midnight(dateOf(draft.startsAt));
              set(
                allDay
                  ? { allDay, startsAt: seconds(day), endsAt: seconds(addDays(day, 1)) }
                  : { allDay, startsAt: onClockOf(day, 9 * 60), endsAt: onClockOf(day, 10 * 60) }
              );
            }}
          />
        </label>
        <div className="cx-pop-row">
          <span>starts</span>
          <When
            at={draft.startsAt}
            allDay={draft.allDay}
            editable={editable}
            label="Starts"
            zone={zoneOf(draft)}
            // Moving the start keeps the length, as Calendar does — an all-day event's in days:
            // kept in seconds, one moved onto a day the clock changes ran an hour into the next.
            onChange={(startsAt) => set({
              startsAt,
              endsAt: draft.allDay
                ? addDaysIn(ZONE, startsAt, Math.max(1, Math.round((draft.endsAt - draft.startsAt) / 86_400)))
                : startsAt + (draft.endsAt - draft.startsAt),
            })}
          />
        </div>
        <div className="cx-pop-row">
          <span>ends</span>
          <When
            at={draft.allDay ? draft.endsAt - 1 : draft.endsAt}
            allDay={draft.allDay}
            editable={editable}
            label="Ends"
            zone={zoneOf(draft)}
            onChange={(end) =>
              set({ endsAt: draft.allDay ? seconds(addDays(midnight(dateOf(end)), 1)) : Math.max(end, draft.startsAt + 300) })
            }
          />
        </div>
        {!draft.allDay && (
          <div className="cx-pop-row cal-field">
            <span>time zone</span>
            <ZoneField
              zone={zoneOf(draft)}
              at={draft.startsAt}
              editable={editable}
              onChange={(z) => {
                const from = zoneOf(draft);
                set({ timezone: z, startsAt: sameClockIn(from, z, draft.startsAt), endsAt: sameClockIn(from, z, draft.endsAt) });
              }}
            />
          </div>
        )}
        <div className="cx-pop-row cal-field">
          <span>calendar</span>
          <Picker
            label="Calendar"
            value={draft.calendarGuid}
            disabled={!editable || choices.length < 2}
            onChange={(guid) => set({ calendarGuid: guid })}
            options={choices.map((c) => ({ value: c.guid, label: c.title }))}
          />
        </div>
        {/* Alert, and a second once there is a first — icloud.com's own pair and presets. */}
        {[0, 1].map((n) => {
          const alerts = draft.alerts;
          if (n === 1 && !(alerts && alerts.length >= 1)) return null;
          const presets = draft.allDay ? ALL_DAY_ALERTS : ALERTS;
          const current = alerts?.[n];
          const options = [
            { value: "none", label: "None" },
            ...presets.map((p) => ({ value: String(p.minutes), label: p.label })),
            ...(current !== undefined && !presets.some((p) => p.minutes === current)
              ? [{ value: String(current), label: alertLabel(current) }]
              : []),
          ];
          return (
            <div className="cx-pop-row cal-field" key={`alert${n}`}>
              <span>{n === 0 ? "alert" : "2nd alert"}</span>
              <Picker
                label={n === 0 ? "Alert" : "Second alert"}
                value={alerts === undefined ? "loading" : current === undefined ? "none" : String(current)}
                disabled={!editable || alerts === undefined}
                onChange={(v) => {
                  const next = [...(alerts ?? [])];
                  if (v === "none") next.splice(n, 1);
                  else next[n] = Number(v);
                  // Any beyond the two shown (made on a Mac) are kept, never dropped by an edit here.
                  set({ alerts: next, alertsChanged: true, alertsDefault: false });
                }}
                options={alerts === undefined ? [{ value: "loading", label: "Loading" }] : options}
              />
            </div>
          );
        })}
        {/* Repeat, icloud.com's simple choices; a schedule it has no name for is shown as is. */}
        <div className="cx-pop-row cal-field">
          <span>repeat</span>
          <Picker
            label="Repeat"
            value={draft.repeat === undefined ? "loading" : repeatKey(draft.repeat)}
            disabled={!editable || draft.repeat === undefined}
            onChange={(v) => set({ repeat: v === "never" ? null : { freq: v as EventRepeat["freq"], interval: 1, count: null }, repeatChanged: true })}
            options={
              draft.repeat === undefined
                ? [{ value: "loading", label: "Loading" }]
                : [
                    ...REPEATS,
                    ...(repeatKey(draft.repeat) === "custom" ? [{ value: "custom", label: repeatText(draft.repeat) }] : []),
                  ]
            }
          />
        </div>
      </div>
      <span className="cx-pop-place">
        <input
          className="cx-pop-line"
          type="url"
          value={draft.url ?? ""}
          placeholder="Add URL"
          aria-label="URL"
          readOnly={!editable}
          onChange={(e) => set({ url: e.target.value || null })}
        />
        {draft.url && /^(https?:\/\/|mailto:)/i.test(draft.url.trim()) && outsideHref(draft.url) && (
          <a className="cx-pop-link is-text" href={outsideHref(draft.url)!} target="_blank" rel="noreferrer">Open</a>
        )}
      </span>
      <Guests guid={draft.guid} />
      <textarea
        className="cx-pop-notes"
        value={draft.notes}
        placeholder="Add Notes"
        aria-label="Notes"
        rows={2}
        readOnly={!editable}
        onChange={(e) => set({ notes: e.target.value })}
      />
      <div className="cx-pop-actions">
        <span>
          {onDelete && editable && (
            <button type="button" className="link danger cx-pop-delete" onClick={onDelete}>
              Delete
            </button>
          )}
          <Waiting shown={waiting} />
        </span>
        <span>
          <button type="button" className="quiet" onClick={onClose}>
            {editable ? "Cancel" : "Close"}
          </button>
          {editable && (
            <button type="submit" className="primary">
              {draft.guid ? "Save" : "Add"}
            </button>
          )}
        </span>
      </div>
    </form>
    </Portal>
  );
}

/**
 * The day at a glance, beside Day's timeline — as Calendar sets it: the date, then what fills
 * it, all-day first and the rest in time order. A wide window gave the timeline the whole
 * width and left most of it empty; this is what Calendar puts there.
 */
const DayPane = memo(function DayPane({
  day,
  events,
  reminders,
  calendars,
  onOpen,
  onMenu,
  week,
  children,
}: {
  day: Date;
  events: CalendarEvent[];
  reminders: Reminder[];
  calendars: Map<string, Calendar>;
  onOpen: (e: CalendarEvent, at: Anchor) => void;
  onMenu: (e: CalendarEvent, at: { x: number; y: number }) => void;
  /** The week's number, when week numbers are shown. */
  week: number | null;
  children?: React.ReactNode;
}) {
  const listColour = useListColour();
  const showReminder = useShowReminder();
  const from = seconds(day);
  const to = seconds(addDays(day, 1));
  const today = events.filter((e) => e.startsAt < to && e.endsAt > from);
  const allDay = today.filter((e) => spansDays(e));
  const timed = today.filter((e) => !spansDays(e)).sort((a, b) => a.startsAt - b.startsAt);
  const due = reminders
    .filter((r) => r.dueAt !== null && r.dueAt >= from && r.dueAt < to && !r.completedAt)
    .sort((a, b) => (a.dueAt ?? 0) - (b.dueAt ?? 0));
  const row = (e: CalendarEvent, when: string) => (
    <li key={e.guid}>
      <button
        className="cx-pane-event"
        data-event={e.guid}
        style={{ "--cal": tint(calendars.get(e.calendarGuid)) } as React.CSSProperties}
        onClick={(ev) => onOpen(e, pointOf(ev.currentTarget))}
        onContextMenu={(ev) => {
          ev.preventDefault();
          onMenu(e, { x: ev.clientX, y: ev.clientY });
        }}
      >
        <span className="cx-pane-title">{e.title}</span>
        <span className="cx-pane-when">{when}</span>
        {e.location && <span className="cx-pane-where">{e.location}</span>}
      </button>
    </li>
  );
  const nothing = allDay.length + timed.length + due.length === 0;
  return (
    <aside className="cx-pane" aria-label={dateText(day, { dateStyle: "full" })}>
      {children && <div className="cx-pane-month">{children}</div>}
      <h2 className="cx-pane-day">{dateText(day, { weekday: "long" })}</h2>
      <p className="cx-pane-date">
        {dateText(day, { day: "numeric", month: "long", year: "numeric" })}
        {week !== null && <span className="cx-pane-week">Week {week}</span>}
      </p>
      {nothing ? (
        <p className="cx-pane-empty">Nothing on this day.</p>
      ) : (
        <ul className="cx-pane-list">
          {allDay.map((e) => row(e, "all-day"))}
          {timed.map((e) => row(e, `${clock(e.startsAt)} – ${clock(e.endsAt)}`))}
          {due.map((r) => (
            <li key={r.guid}>
              <div className="cx-pane-reminder" style={listColour(r)} {...showReminder(r)}>
                <Ring reminder={r} quiet />
                <span className="cx-pane-title">{r.title}</span>
                <span className="cx-pane-when">{r.allDay ? "all-day" : clock(r.dueAt ?? 0)}</span>
              </div>
            </li>
          ))}
        </ul>
      )}
    </aside>
  );
});

/** Apple's calendar colours, in icloud.com's order (calendar bundle `r2`). */
const CALENDAR_COLOURS: [string, string][] = [
  ["#ff2d55", "Red"],
  ["#ff9500", "Orange"],
  ["#ffcc00", "Yellow"],
  ["#65db39", "Green"],
  ["#34aadc", "Blue"],
  ["#cc73e1", "Purple"],
  ["#a2845e", "Brown"],
];

/** A calendar's name being typed: Return or leaving keeps it, Escape leaves it as it was. */
function CalendarName({ initial, label, onDone, onCancel }: {
  initial: string;
  label: string;
  onDone: (name: string) => void;
  onCancel: () => void;
}) {
  const settled = useRef(false);
  return (
    <input
      className="field cx-cal-rename"
      defaultValue={initial}
      aria-label={label}
      autoFocus
      onFocus={(e) => e.currentTarget.select()}
      onBlur={(e) => {
        if (settled.current) return;
        settled.current = true;
        onDone(e.currentTarget.value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") {
          settled.current = true;
          onCancel();
        }
      }}
    />
  );
}

/** `text` with the words of `query` marked, as Calendar marks what a search matched. */
function Marked({ text, query }: { text: string; query: string }) {
  const words = query.trim().split(/\s+/).filter(Boolean).map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  if (!words.length) return <>{text}</>;
  const parts = text.split(new RegExp(`(${words.join("|")})`, "gi"));
  return <>{parts.map((p, i) => (i % 2 ? <mark key={i} className="cx-mark">{p}</mark> : <Fragment key={i}>{p}</Fragment>))}</>;
}

/**
 * Quick Event, under the + as Calendar's is: a line typed as it would be said, read here as it
 * is typed, and shown as the event it will make before Return makes it.
 */
function QuickEvent({ calendar, onAdd, onClose }: {
  calendar: Calendar | null;
  onAdd: (text: string, details: boolean, at: Anchor) => void;
  onClose: () => void;
}) {
  const [text, setText] = useState("");
  const box = useRef<HTMLFormElement | null>(null);
  const q = useMemo(() => (text.trim() ? parseQuick(text) : null), [text]);
  const at = (): Anchor => {
    const r = box.current?.getBoundingClientRect();
    return r ? { x: r.right - 300, y: r.top + 8, left: r.left } : { x: 40, y: 80 };
  };
  const start = q ? dateOf(q.startsAt) : null;
  const lastDay = q ? dateOf(q.allDay ? q.endsAt - 1 : q.endsAt) : null;
  const sameDayEnd = start && lastDay ? sameDay(start, lastDay) : true;
  const day = (d: Date) => dateText(d, { weekday: "long", day: "numeric", month: "long" });
  return (
    <form
      ref={box}
      className="popover cx-quick"
      role="dialog"
      aria-label="Quick Event"
      onSubmit={(e) => {
        e.preventDefault();
        onAdd(text, false, at());
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose();
        }
      }}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) onClose();
      }}
    >
      <input
        className="field cx-quick-field"
        aria-label="Event, as you would say it"
        placeholder="Lunch with Asha tomorrow at 1pm"
        autoFocus
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.shiftKey || e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            onAdd(text, true, at());
          }
        }}
      />
      {q && start && lastDay && (
        <div className="cx-quick-preview" aria-live="polite" style={{ "--cal": tint(calendar ?? undefined) } as React.CSSProperties}>
          <p className="cx-quick-title">{q.title || "New Event"}</p>
          <p className="cx-quick-when">
            <span>{day(start)}</span>
            <span className="cx-quick-time">
              {q.allDay
                ? sameDayEnd ? "All day" : `All day, until ${day(lastDay)}`
                : `${clock(q.startsAt)} – ${clock(q.endsAt)}${sameDayEnd ? "" : `, ${day(lastDay)}`}`}
            </span>
          </p>
          {(q.repeat || q.location) && (
            <p className="cx-quick-more">
              {q.repeat && (
                <span className="cx-quick-chip"><Repeat size={12} strokeWidth={2.2} aria-hidden="true" />{repeatText(q.repeat)}</span>
              )}
              {q.location && (
                <span className="cx-quick-chip"><MapPin size={12} strokeWidth={2.2} aria-hidden="true" />{q.location}</span>
              )}
            </p>
          )}
          <p className="cx-quick-cal"><span className="cx-dot" aria-hidden="true" />{calendar?.title}</p>
        </div>
      )}
      <div className="cx-quick-actions">
        <button type="button" className="quiet" onClick={() => onAdd(text, true, at())}>Details</button>
        <button type="submit" className="primary" disabled={!q?.title}>Add</button>
      </div>
    </form>
  );
}

/**
 * New Calendar Subscription: an address, fetched once now so a wrong one is said here, then
 * kept and refreshed on this computer. Read-only, as a subscription is.
 */
function SubscribeForm({ onDone, onCancel }: {
  onDone: (sub: CalendarSubscription) => void;
  onCancel: () => void;
}) {
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  return (
    <form
      className="cx-subscribe"
      aria-label="New Calendar Subscription"
      onSubmit={(e) => {
        e.preventDefault();
        if (!url.trim() || busy) return;
        setBusy(true);
        setProblem(null);
        calendarFeedsApi.subscribe(url.trim()).then(
          (sub) => onDone(sub),
          (err) => {
            setBusy(false);
            setProblem(messageOf(err));
          },
        );
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onCancel();
        }
      }}
    >
      <label className="cx-subscribe-label" htmlFor="cx-subscribe-url">Subscribe to a calendar</label>
      <input
        id="cx-subscribe-url"
        className="field"
        type="url"
        inputMode="url"
        placeholder="webcal:// or https:// address"
        autoFocus
        spellCheck={false}
        value={url}
        disabled={busy}
        onChange={(e) => { setUrl(e.target.value); setProblem(null); }}
      />
      {problem ? (
        <p className="cx-subscribe-note is-problem" role="alert">{problem}</p>
      ) : (
        <p className="cx-subscribe-note">Skykeep reads it from that address and keeps it up to date.</p>
      )}
      <div className="cx-subscribe-actions">
        <button type="button" className="quiet" onClick={onCancel}>Cancel</button>
        <button type="submit" className="primary" disabled={!url.trim() || busy} aria-busy={busy}>Subscribe</button>
      </div>
    </form>
  );
}
