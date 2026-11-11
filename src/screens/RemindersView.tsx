import { Fragment, memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AppFoot } from "../components/AppFoot";
import { refusedLink, outsideHref } from "../lib/links";
import { composing } from "../lib/keys";
import { fadeIn, fadeOut } from "../lib/fade";
import { overlayOpen, useActiveRef } from "../lib/screen";
import { useModalFocus } from "../lib/focus";
import { listen } from "@tauri-apps/api/event";
import { usePending } from "../lib/pending";
import { Unsent, Waiting } from "../components/Unsent";
import { useOutbox } from "../lib/outbox";
import { useErrorToast } from "../components/Toast";
import { useConfirm } from "../components/Confirm";
import { EmptyState } from "../components/EmptyState";
import { SearchField } from "../components/SearchField";
import { PaneTitle } from "../components/PaneTitle";
import { Portal } from "../components/Portal";
import { TimeField } from "../components/TimeField";
import { AlarmClock, ChevronLeft, ChevronRight, Link2, ListFilter, MapPin, MessageCircle, X } from "lucide-react";
import {
  api,
  MOST_AT_ONCE,
  messageOf,
  type Reminder,
  type ReminderGroup,
  type ReminderList,
  type ReminderSection,
  type ReminderCounts,
  type SmartList,
  remindersApi,
  reminderImageSrc,
  type ReminderDetail,
  type ReminderMore,
  type ReminderMarks,
  type RepeatRule,
  type ReminderSmartList,
  type TagCount,
  REMINDER_URGENT,
  type UrgentRead,
} from "../lib/ipc";
import { Arriving } from "../components/EmptyState";
import { useArriving, useShownRevision } from "../lib/sync";
import { Picker } from "../components/Picker";
import { RowMenu } from "../components/RowMenu";
import { Glyph, emblemGlyph, type GlyphName } from "../components/Glyph";
import { dateText, dayOf, timeText, useClock } from "../lib/when";
import { parseReminder, understood, type QuickReminder } from "./reminderQuick.ts";
import { useReveal } from "../lib/reveal";
import { instantIn, wallIn, zoneName, type Wall } from "./calendar/model";
import { keepSame, same } from "../lib/same";
import { useArrivals } from "../lib/arrival";
import { carryLabel } from "../lib/dragPicture";

/** The built-in views, in iCloud's order, each with its glyph and Skykeep's colour for it (the
 *  same as the Android app's). Each carries white glyphs at 3.5:1 or better. */
const SMART: { kind: SmartList; title: string; glyph: GlyphName; colour: string }[] = [
  { kind: "today", title: "Today", glyph: "today", colour: "#2A82E0" },
  { kind: "scheduled", title: "Scheduled", glyph: "scheduled", colour: "#D84A3F" },
  { kind: "all", title: "All", glyph: "all", colour: "#7A8796" },
  { kind: "flagged", title: "Flagged", glyph: "flagged", colour: "#B86E00" },
  { kind: "completed", title: "Completed", glyph: "completed", colour: "#778392" },
];

/** A list's own colour, or Skykeep's blue for one that has none. */
const colourOf = (list: ReminderList | undefined | null) =>
  list?.colour && /^#[0-9a-f]{6}$/i.test(list.colour) ? list.colour : "var(--sk-blue)";

/** Groups the viewer has folded shut, remembered on this computer. */
const FOLDED_KEY = "quince.reminders.folded";
function readFolded(): Set<string> {
  try {
    const raw = window.localStorage.getItem(FOLDED_KEY);
    return new Set(raw ? (JSON.parse(raw) as string[]) : []);
  } catch {
    return new Set();
  }
}

/** Reminders whose subtasks are hidden, remembered on this computer, as the Mac keeps them. */
const HIDDEN_SUBTASKS_KEY = "quince.reminders.hiddenSubtasks";
function readHiddenSubtasks(): Set<string> {
  try {
    const raw = window.localStorage.getItem(HIDDEN_SUBTASKS_KEY);
    return new Set(raw ? (JSON.parse(raw) as string[]) : []);
  } catch {
    return new Set();
  }
}
function keepHiddenSubtasks(hidden: Set<string>) {
  try {
    window.localStorage.setItem(HIDDEN_SUBTASKS_KEY, JSON.stringify([...hidden]));
  } catch {
    // Without storage they still fold; it is forgotten on restart.
  }
}

/** End of today in the viewer's own timezone — the interface knows it, the backend doesn't. */
function endOfToday(): number {
  const d = new Date();
  return Math.floor(
    new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1).getTime() / 1000,
  );
}

/**
 * How many days apart two moments are, counted on a calendar rather than in milliseconds.
 *
 * Dividing by 86,400,000 is wrong on the two days a year a clock moves: a day is then
 * twenty-three hours or twenty-five, and something due tomorrow evening comes out as today.
 * Counting dates is right wherever the viewer is, including the places that never change.
 */
function daysApart(from: Date, to: Date): number {
  const noon = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate(), 12).getTime();
  return Math.round((noon(to) - noon(from)) / 86_400_000);
}

/**
 * When a reminder is due, said the way Apple says it.
 *
 * `dated` means the heading above already names the day — under "Tomorrow", Apple writes
 * "9:30 PM" and not "Tomorrow 9:30 PM". Saying it twice is the sort of thing that makes a
 * screen read as almost-right.
 */
/** A repeat in words, as the phone's Repeat row says it: "Daily", "Every 2 Weeks". */
function repeatWords(r: { frequency: number; interval: number }): string {
  const unit = ["Day", "Week", "Month", "Year", "Hour"][r.frequency] ?? "Day";
  if (r.interval === 1) return ["Daily", "Weekly", "Monthly", "Yearly", "Hourly"][r.frequency] ?? "Daily";
  return `Every ${r.interval} ${unit}s`;
}

function dueLabel(seconds: number, dated = false, allDay = false): { text: string; overdue: boolean } {
  const due = new Date(seconds * 1000);
  const now = new Date();
  const days = daysApart(now, due);
  // A day has no time to show, and is overdue only once the day itself has passed.
  if (allDay) {
    if (dated || days === 0) return { text: dated ? "" : "Today", overdue: false };
    if (days === 1) return { text: "Tomorrow", overdue: false };
    return { text: dateText(due, { day: "numeric", month: "short" }), overdue: days < 0 };
  }
  const time = timeText(due, {
    hour: "numeric",
    minute: "2-digit",
  });
  if (dated) {
    return { text: time, overdue: days < 0 || (days === 0 && due.getTime() < now.getTime()) };
  }
  if (days < 0)
    return {
      text: dateText(due, {
        day: "numeric",
        month: "short",
      }),
      overdue: true,
    };
  if (days === 0)
    return { text: `Today ${time}`, overdue: due.getTime() < now.getTime() };
  if (days === 1) return { text: `Tomorrow ${time}`, overdue: false };
  return {
    text: dateText(due, { day: "numeric", month: "short" }),
    overdue: false,
  };
}

/** Apple's scale, and the marks its own app puts before a title. */
const PRIORITIES = [
  { value: 0, label: "None", mark: "" },
  { value: 1, label: "High", mark: "!!!" },
  { value: 5, label: "Medium", mark: "!!" },
  { value: 9, label: "Low", mark: "!" },
] as const;

const priorityMark = (priority: number) =>
  PRIORITIES.find((p) => p.value === priority)?.mark ?? "";

/** Apple's frequencies by number: 0 daily, 1 weekly, 2 monthly, 3 yearly, 4 hourly. */
const REPEAT_UNITS = ["day", "week", "month", "year", "hour"] as const;

/**
 * The iPhone's Repeat menu, in its order. Each preset is a frequency and an interval, which
 * is how the phone writes them; Weekdays and Weekends are weekly with chosen days (1 Sunday …
 * 7 Saturday), as the phone writes those.
 */
const WEEKDAYS = [2, 3, 4, 5, 6];
const WEEKENDS = [1, 7];
const REPEATS: { key: string; label: string; repeat: { frequency: number; interval: number } | null; days?: number[]; divider?: boolean; disabled?: boolean }[] = [
  { key: "never", label: "Never", repeat: null },
  { key: "4:1", label: "Hourly", repeat: { frequency: 4, interval: 1 }, divider: true },
  { key: "0:1", label: "Daily", repeat: { frequency: 0, interval: 1 } },
  { key: "weekdays", label: "Weekdays", repeat: { frequency: 1, interval: 1 }, days: WEEKDAYS },
  { key: "weekends", label: "Weekends", repeat: { frequency: 1, interval: 1 }, days: WEEKENDS },
  { key: "1:1", label: "Weekly", repeat: { frequency: 1, interval: 1 } },
  { key: "1:2", label: "Biweekly", repeat: { frequency: 1, interval: 2 } },
  { key: "2:1", label: "Monthly", repeat: { frequency: 2, interval: 1 } },
  { key: "2:3", label: "Every 3 Months", repeat: { frequency: 2, interval: 3 } },
  { key: "2:6", label: "Every 6 Months", repeat: { frequency: 2, interval: 6 } },
  { key: "3:1", label: "Yearly", repeat: { frequency: 3, interval: 1 } },
  { key: "custom", label: "Custom", repeat: null, divider: true },
];
/** The Custom screen's Frequency menu, in the phone's order. */
const FREQUENCIES = [
  { value: "4", label: "Hourly" },
  { value: "0", label: "Daily" },
  { value: "1", label: "Weekly" },
  { value: "2", label: "Monthly" },
  { value: "3", label: "Yearly" },
];

const sameDays = (a: number[], b: number[]) => a.length === b.length && a.every((d, i) => d === b[i]);

/** Which choice a reminder's rule is: a preset — Weekdays and Weekends by their days — or
 *  Custom for any other schedule. */
function repeatKey(r: Reminder, rule?: RepeatRule | null, days: number[] = rule?.daysOfWeek ?? []): string {
  if (r.recurFrequency === null || r.recurFrequency === undefined) return "never";
  if (rule && rule.other.length > 0) return "custom";
  const every = r.recurInterval ?? 1;
  if (days.length > 0) {
    if (r.recurFrequency !== 1 || every !== 1) return "custom";
    return sameDays(days, WEEKDAYS) ? "weekdays" : sameDays(days, WEEKENDS) ? "weekends" : "custom";
  }
  const key = `${r.recurFrequency}:${every}`;
  return REPEATS.some((option) => option.key === key && option.repeat && !option.days) ? key : "custom";
}

/** "Every day", "Every 2 weeks" — or null when this happens only once. */
function repeatLabel(r: Reminder): string | null {
  if (r.recurFrequency === null || r.recurFrequency === undefined) return null;
  const unit = REPEAT_UNITS[r.recurFrequency];
  if (!unit) return "Repeats";
  const every = r.recurInterval ?? 1;
  return every > 1 ? `Every ${every} ${unit}s` : `Every ${unit}`;
}

/**
 * The word Apple puts beside the repeat arrow: Daily, Weekly, Monthly, Yearly.
 *
 * The arrow alone says something repeats without saying how often, which is the one thing
 * worth knowing about it at a glance.
 */
function repeatWord(r: Reminder, days: number[] = []): string | null {
  if (r.recurFrequency === null || r.recurFrequency === undefined) return null;
  if (r.recurFrequency === 1 && (r.recurInterval ?? 1) === 1 && days.length > 0) {
    if (sameDays(days, WEEKDAYS)) return "Weekdays";
    if (sameDays(days, WEEKENDS)) return "Weekends";
    return days.map((d) => WEEKDAY_SHORT[d - 1]).join(", ");
  }
  if ((r.recurInterval ?? 1) > 1) return repeatLabel(r);
  return ["Daily", "Weekly", "Monthly", "Yearly", "Hourly"][r.recurFrequency] ?? "Repeats";
}

/**
 * An all-day date as Apple keeps it is that day's midnight in UTC. Shown and grouped here at
 * the same day's local midnight — read as a moment, the 17th's UTC midnight is the evening of
 * the 16th west of Greenwich, and a day reminder would drift into the day before.
 */
const dayToLocal = (utc: number) => {
  const d = new Date(utc * 1000);
  return Math.floor(new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()).getTime() / 1000);
};
const dayToUtc = (value: string) => {
  const [y, m, d] = value.split("-").map(Number);
  return Math.floor(Date.UTC(y, m - 1, d) / 1000);
};
/**
 * The backend's answer when a reminder acted on is not in the library at all (REMINDER_GONE in
 * main.rs) — gone from iCloud and synced away. Answered with a quiet re-read, never a notice.
 */
const REMINDER_GONE = "reminder-gone";
/** A row shown for a reminder still being added, before Apple has given it an id. */
const STAND_IN = "pending:";

const asShown = (r: Reminder): Reminder =>
  r.allDay && r.dueAt !== null ? { ...r, dueAt: dayToLocal(r.dueAt) } : r;


/**
 * Tags, links, early alerts and sections can be changed from here: on since each was seen on
 * his iPhone as written (2026-10-22). Mirrors `REMINDER_DETAIL_WRITES` in
 * crates/quince-icloud/src/reminders_details.rs: turn both on together.
 */
export const REMINDER_DETAIL_WRITES = true;

/** Apple's early-alert units, by its numbering. */
const ALERT_UNITS = ["minute", "hour", "day", "week", "month"] as const;
/** "30 minutes before", "1 day before" — the phone's Early Reminder words. */
function earlyText(count: number, unit: number): string {
  const word = ALERT_UNITS[unit] ?? "minute";
  return `${count} ${word}${count === 1 ? "" : "s"} before`;
}
/** What the phone's Early Reminder menu offers, as [count, unit]. */
const EARLY_CHOICES: [number, number][] = [
  [5, 0], [15, 0], [30, 0], [1, 1], [2, 1], [1, 2], [2, 2], [1, 3], [1, 4],
];

/** This computer's own zone. */
const LOCAL_ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
/** "11:00 AM GMT-4": a time on another zone's clock, as the phone writes it in a row. */
function zonedClock(zone: string, s: number): string {
  try {
    const time = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit", timeZone: zone }).format(new Date(s * 1000));
    return `${time} ${zoneName(zone, s).offset.replace("−", "-")}`;
  } catch {
    return "";
  }
}
/** A link as the phone's chip shows it: its site, without "www.". */
const hostOf = (url: string) => {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url.replace(/^https?:\/\//, "").replace(/\/.*$/, "");
  }
};

/** The zone a reminder's time is read in: its own, or this computer's. */
const zoneOf = (r: Reminder) => r.timezone || LOCAL_ZONE;
/** This computer's clock reading at `s`. */
const localWall = (s: number): Wall => {
  const d = new Date(s * 1000);
  return { y: d.getFullYear(), m: d.getMonth() + 1, d: d.getDate(), h: d.getHours(), min: d.getMinutes() };
};
/** "2026-10-22" and "11:00" on the clock of `zone`, as an instant. */
function timedInstant(day: string, time: string, zone: string): number {
  const [y, m, d] = day.split("-").map(Number);
  const [h, min] = time.split(":").map(Number);
  return instantIn(zone, { y, m, d, h, min });
}

/** A smart list's own colour, or Skykeep's grey for one that has none. */
const smartColour = (s: ReminderSmartList) =>
  s.colour && /^#[0-9a-f]{6}$/i.test(s.colour) ? s.colour : "var(--sk-gray)";

type Selection =
  | { type: "smart"; kind: SmartList }
  | { type: "list"; guid: string }
  | { type: "tag"; name: string }
  | { type: "custom"; guid: string };

export function RemindersView() {
  const [lists, setLists] = useState<ReminderList[]>([]);
  /** Each list by its id: every row looks its own up, and a search through the array per row
   *  was a thousand walks of it per draw. */
  const listById = useMemo(() => new Map(lists.map((l) => [l.guid, l])), [lists]);
  const focusedBeforePress = useRef<Element | null>(null);
  /** A reminder's menu, from a right-click on its row. */
  const [rowMenu, setRowMenu] = useState<{ reminder: Reminder; at: { x: number; y: number; alignRight: boolean } } | null>(null);
  /** A list's menu, from its ⋯ or a right-click. */
  const [listMenu, setListMenu] = useState<{ list: ReminderList; at: { x: number; y: number; alignRight: boolean } } | null>(null);
  /** The list whose List Details sheet is open. */
  const [infoFor, setInfoFor] = useState<ReminderList | null>(null);
  const [groups, setGroups] = useState<ReminderGroup[]>([]);
  const [folded, setFolded] = useState<Set<string>>(readFolded);
  /** Reminders whose subtasks are folded away under them (the arrow beside one, Ctrl+E). */
  const [hiddenSubtasks, setHiddenSubtasks] = useState<Set<string>>(readHiddenSubtasks);
  const foldSubtasks = useCallback((guid: string) => {
    setHiddenSubtasks((was) => {
      const next = new Set(was);
      if (next.has(guid)) next.delete(guid);
      else next.add(guid);
      keepHiddenSubtasks(next);
      return next;
    });
  }, []);
  const toggleFolded = useCallback((guid: string) => {
    setFolded((was) => {
      const next = new Set(was);
      if (next.has(guid)) next.delete(guid);
      else next.add(guid);
      try {
        window.localStorage.setItem(FOLDED_KEY, JSON.stringify([...next]));
      } catch {
        // Without storage the fold still works; it is forgotten on restart.
      }
      return next;
    });
  }, []);
  /**
   * What was picked, and what the pane shows. The sidebar answers a click at once from
   * `picked`; the pane moves to it only when the picked view's reminders and headings have
   * arrived, and takes all of them in one draw. Moving the heading first left the new
   * list's title over the old list's reminders for a moment — the flicker on every switch.
   */
  const [picked, setSelection] = useState<Selection>({
    type: "smart",
    kind: "today",
  });
  const [selection, setShown] = useState<Selection>(picked);
  // A reminder found by the search across everything: its list opened, its details shown,
  // and its row brought into view once the list has come.
  const revealing = useRef<string | null>(null);
  useReveal("reminders", (r) => {
    if (r.within) setSelection({ type: "list", guid: r.within });
    revealing.current = r.id;
    setOpenGuid(r.id);
  });
  const [items, setItems] = useState<Reminder[]>([]);
  // Info belongs to the view it was opened in; another view starts with it closed — unless
  // the search across everything is opening this very reminder.
  useEffect(() => {
    if (!revealing.current) setOpenGuid(null);
  }, [picked]);
  useEffect(() => {
    const id = revealing.current;
    if (!id || !items.some((r) => r.guid === id)) return;
    revealing.current = null;
    requestAnimationFrame(() =>
      document.querySelector(`.rx-row [aria-label="Rename ${CSS.escape(items.find((r) => r.guid === id)?.title ?? "")}"]`)
        ?.closest(".rx-row")
        ?.scrollIntoView({ block: "center" }),
    );
  }, [items]);
  const [showDone, setShowDone] = useState(false);
  const [syncing, setSyncing] = useState(false);
  /** Every reminder with a write still on its way — a set, since several can be at once. */
  const [openGuid, setOpenGuid] = useState<string | null>(null);
  /** The reminder open now — read after an await, when the one open before it may have changed. */
  const openRef = useRef<string | null>(null);
  openRef.current = openGuid;
  const [newList, setNewList] = useState("");
  const [addingList, setAddingList] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useErrorToast(error, () => setError(null));
  const [confirm, confirmSheet] = useConfirm();
  // Held while another screen is in front, and read once on coming back (lib/sync).
  const revision = useShownRevision("reminders");
  const arriving = useArriving("reminders");
  /** What each sidebar entry is holding. Apple puts a number on every one. */
  const [counts, setCounts] = useState<ReminderCounts | null>(null);
  /** The headings inside the list that is open, when a real list is open. */
  const [listSections, setListSections] = useState<ReminderSection[]>([]);
  /** Every tag on the account, with how many reminders still to do carry each. */
  const [tags, setTags] = useState<TagCount[]>([]);
  /** Each reminder's tags, by its id — for the chips on its row. */
  const [tagsOf, setTagsOf] = useState<Map<string, string[]>>(() => new Map());
  /** Smart lists made by hand on the phone or Mac, and how many each holds now. */
  const [customLists, setCustomLists] = useState<ReminderSmartList[]>([]);
  const [customCounts, setCustomCounts] = useState<Map<string, number>>(() => new Map());
  /** Rules of the open smart list this app does not read yet. */
  const [unknownRules, setUnknownRules] = useState<string[]>([]);
  /** The open reminder's tags, link and early alerts. */
  const [detailOf, setDetailOf] = useState<ReminderDetail | null>(null);
  /** The rest of the open reminder's Details: its repeat whole, zone, Urgent, place, person. */
  const [moreOf, setMoreOf] = useState<ReminderMore | null>(null);
  /** Each row's marks beside its date, by reminder: Weekdays, Urgent, a place, a person, a link. */
  const [marks, setMarks] = useState<Map<string, ReminderMarks>>(() => new Map());
  /** Repeating reminders whose next tick is their last: past End Repeat, ticked means done. */
  const [ending, setEnding] = useState<Set<string>>(() => new Set());
  /** A repeating reminder that was just ticked forward, and where it landed. */
  const [moved, setMoved] = useState<{ guid: string; text: string } | null>(
    null,
  );

  const inList = selection.type === "list";
  const listGuid = inList ? selection.guid : null;
  /** The built-in view open, if one is. */
  const smartKind = selection.type === "smart" ? selection.kind : null;
  /** A tag or a hand-made smart list: views gathered from every list, with nowhere to add. */
  const gathered = selection.type === "tag" || selection.type === "custom";
  const openCustom = selection.type === "custom" ? (customLists.find((x) => x.guid === selection.guid) ?? null) : null;

  const readLists = useCallback(async () => {
    const [found, tally, folders] = await Promise.all([
      api.reminderLists(),
      api.reminderCounts(endOfToday()),
      // An older library without groups is still a working one.
      api.reminderGroups().catch(() => [] as ReminderGroup[]),
    ]);
    setLists(Array.isArray(found) ? found : []);
    setGroups(Array.isArray(folders) ? folders : []);
    setCounts(tally);
    setLoaded(true);
    // Tags and smart lists after: an older library without them is still a working one.
    void readExtras().catch(() => {});
  }, []);

  /**
   * Tags with each one's reminders, and custom smart lists with what each holds. Three reads,
   * whatever the account holds: this asked for every tag's reminders and every smart list's
   * whole, one call each, only to know which tags each row carries and how many each list has.
   */
  const readExtras = async () => {
    const [allTags, smart, extras] = await Promise.all([
      remindersApi.tags().catch(() => [] as TagCount[]),
      remindersApi.smartLists().catch(() => [] as ReminderSmartList[]),
      remindersApi.extras().catch(() => null),
    ]);
    const named = (Array.isArray(allTags) ? allTags : []).filter((t) => t.name);
    const custom = (Array.isArray(smart) ? smart : []).filter((x) => x.kind === "custom");
    setTags(named);
    // Each row's tags kept as the array it had when they are the same: a new one each read drew
    // every row with a tag again for any change at all (lib/same).
    setTagsOf((was) => {
      const next = new Map(Object.entries(extras?.tagsByReminder ?? {}).map(([k, v]) => {
        const had = was.get(k);
        return [k, had && same(had, v) ? had : v] as const;
      }));
      return next.size === was.size && [...next].every(([k, v]) => was.get(k) === v) ? was : next;
    });
    setCustomLists(custom);
    setCustomCounts(new Map(custom.map((x) => [x.guid, extras?.customCounts?.[x.guid] ?? 0])));
  };

  // A list's headings are read with its reminders (readItems), so they arrive together.

  /** What is typed in the search field. While it holds anything, the pane is the results. */
  const [query, setQuery] = useState("");
  const searching = query.trim().length > 0;
  /**
   * What is searched for: the field once typing pauses for 120 ms, as Calendar's search waits.
   * Each key ran a search and the rows' marks again; a field emptied is taken at once.
   */
  const [settled, setSettled] = useState("");
  useEffect(() => {
    if (!query.trim()) {
      setSettled("");
      return undefined;
    }
    const timer = window.setTimeout(() => setSettled(query), 120);
    return () => window.clearTimeout(timer);
  }, [query]);
  const sought = searching ? settled : "";
  const seeking = sought.trim().length > 0;
  /**
   * In a narrow window one column shows at a time, as on iPhone: the lists, or the list that
   * was picked, with a way back. Searching shows the results under the search field.
   */
  const [detail, setDetail] = useState(false);

  const { hold, over } = usePending<Reminder>();
  /** Changes kept here and not yet at Apple, and any Apple turned down. */
  const outbox = useOutbox("reminders");
  // A reminder added on the phone, landing in the list on screen, fades in (lib/arrival).
  const mainRef = useRef<HTMLElement | null>(null);
  const itemIds = useMemo(() => items.map((r) => r.guid), [items]);
  useArrivals(mainRef, itemIds, `${JSON.stringify(selection)}\u0000${showDone}\u0000${sought.trim()}`, outbox.waiting.size > 0 || itemIds.some((g) => g.startsWith(STAND_IN)));
  // When each timed reminder here falls due, and the time moved on to each of them and to each
  // midnight (lib/when): a row is told only when its label or its red can change.
  const dueMoments = useMemo(() => items.flatMap((r) => (r.dueAt !== null && !r.allDay ? [r.dueAt * 1000] : [])), [items]);
  const clock = useClock(dueMoments);
  const today = dayOf(clock);
  /** Each read is numbered: one overtaken by a later pick is dropped, never drawn late. */
  const reads = useRef(0);
  const readItems = useCallback(async (): Promise<Reminder[]> => {
    const sel = picked;
    const seq = ++reads.current;
    // Search reaches every list, as Apple's does, whatever is open in the sidebar.
    let unknown: string[] = [];
    const [found, sections] = await Promise.all([
      seeking
        ? api.remindersSearch(sought)
        : sel.type === "smart"
          ? api.remindersSmart(sel.kind, endOfToday())
          : sel.type === "tag"
            ? remindersApi.tagged(sel.name)
            : sel.type === "custom"
              ? remindersApi.inSmartList(sel.guid).then((c) => {
                  unknown = Array.isArray(c.unknown) ? c.unknown : [];
                  return c.reminders;
                })
              : api.remindersIn(sel.guid, showDone),
      sel.type === "list"
        ? api.reminderSections(sel.guid).catch(() => [] as ReminderSection[])
        : Promise.resolve([] as ReminderSection[]),
    ]);
    const fresh = over((Array.isArray(found) ? found : []).map(asShown));
    if (seq !== reads.current) return fresh;
    // The view, its reminders and its headings, together.
    setShown(sel);
    // Each reminder unchanged keeps its row as it was (lib/same).
    setItems((was) => keepSame(was, fresh, (r) => r.guid));
    setListSections(Array.isArray(sections) ? sections : []);
    setUnknownRules(unknown);
    return fresh;
  }, [picked, showDone, sought, seeking, over]);

  // Changes go to Apple behind the window (the outbox): once a batch has landed the library
  // holds Apple's own version, and a change Apple refused has been put back — said here, why.
  // Through a ref, so the listeners are set up once: with `readItems` in their effect's deps,
  // every key typed into the search took both down and set them up again.
  const readItemsRef = useRef(readItems);
  readItemsRef.current = readItems;
  useEffect(() => {
    const offs = [
      listen("quince://reminders-synced", () => void readItemsRef.current().catch(() => {})),
      // Said by the refused change's own notice (Unsent), which keeps what it held.
      listen<{ service: string; message: string }>("quince://outbox-refused", (e) => {
        if (e.payload.service !== "reminders") return;
        void readItemsRef.current().catch(() => {});
      }),
      // Urgent read from Apple behind the marks' answer, merged in as it comes.
      listen<UrgentRead[]>(REMINDER_URGENT, (e) => {
        const read = Array.isArray(e.payload) ? e.payload : [];
        if (read.length) setMarks((was) => withUrgent(was, read));
      }),
    ];
    return () => offs.forEach((off) => void off.then((f) => f()).catch(() => {}));
  }, []);

  // Both reads come from the library. `revision` ticks when the shared refresh finishes —
  // pressed, or on the timer — so a reminder ticked on the phone appears here without this
  // screen ever touching the network.
  useEffect(() => {
    let cancelled = false;
    readLists().catch((e) => {
      if (!cancelled) {
        setError(messageOf(e));
        setLoaded(true);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [readLists, revision]);

  useEffect(() => {
    readItems().catch((e) => setError(messageOf(e)));
    // `outbox.tick`: a change sent, or turned down, changes what the library holds.
  }, [readItems, revision, outbox.tick]);

  useEffect(() => {
    if (!moved) return undefined;
    const timer = window.setTimeout(() => setMoved(null), 2600);
    return () => window.clearTimeout(timer);
  }, [moved]);

  const patch = (guid: string, change: Partial<Reminder>) => (rows: Reminder[]) =>
    rows.map((x) => (x.guid === guid ? { ...x, ...change } : x));

  /**
   * Every write goes through here. What it will do shows at once — a tick, a flag, a row
   * gone — the way the phone answers a tap; then it goes to Apple and the list is re-read,
   * so what stays on screen is what Apple kept. Waiting on the network before showing the
   * tap made every add, tick and delete feel a second behind. A refusal re-reads, which
   * puts the row back as it was, and says why.
   */
  const queue = useRef(new Map<string, Promise<void>>());
  const write = useCallback(
    async (
      reminder: Reminder,
      run: () => Promise<void>,
      predict?: (rows: Reminder[]) => Reminder[],
    ): Promise<Reminder[] | null> => {
      // A stand-in has no id at Apple yet, so there is nothing to send: a tick or an edit on it
      // was refused as "no longer in the local library". It is replaced by the real row the
      // moment the add lands, and that one takes changes.
      if (reminder.guid.startsWith(STAND_IN)) return null;
      setError(null);
      if (predict) setItems((rows) => predict(rows));
      // Kept over any refresh until Apple has it; let go before the read that confirms it.
      const release = predict ? hold(predict) : () => {};
      // One reminder's changes reach Apple in the order they were made, each after the last —
      // so nothing on the row is ever disabled while one is on its way. (Disabled, the title
      // greyed out for the whole round trip, and a ticked reminder looked stuck.)
      const before = queue.current.get(reminder.guid) ?? Promise.resolve();
      let done = () => {};
      const mine = new Promise<void>((r) => (done = r));
      queue.current.set(reminder.guid, mine);
      try {
        await before;
        await run();
        release();
        return await readItemsRef.current();
      } catch (e) {
        release();
        // Not in the library at all — deleted on another device and synced away. Nothing to
        // say about it: the list is read again and the row goes, as on the phone.
        if (messageOf(e) !== REMINDER_GONE) setError(messageOf(e));
        readItemsRef.current().catch(() => {});
        return null;
      } finally {
        done();
        if (queue.current.get(reminder.guid) === mine) queue.current.delete(reminder.guid);
        release();
      }
    },
    [hold],
  );

  /**
   * Ticking a repeating reminder never closes it — Apple rolls it to the next occurrence.
   * The row therefore comes back unticked with a later date, which looks exactly like
   * nothing happened, so the new date is called out for a moment.
   */
  /**
   * A ticked reminder leaves as on the phone: the circle fills at once, the row stays a
   * moment (so a mistaken tick can be undone), then fades and goes, all on the screen's own
   * clock, never waiting for Apple. Apple's answer only matters if it refuses, which brings
   * the row back with the reason.
   */
  const [leaving, setLeaving] = useState<Set<string>>(() => new Set());
  const [departed, setDeparted] = useState<Set<string>>(() => new Set());
  const departure = useRef(new Map<string, number[]>());
  const stay = (guid: string) => {
    for (const t of departure.current.get(guid) ?? []) window.clearTimeout(t);
    departure.current.delete(guid);
    const without = (was: Set<string>) => {
      if (!was.has(guid)) return was;
      const next = new Set(was);
      next.delete(guid);
      return next;
    };
    setLeaving(without);
    setDeparted(without);
  };
  const depart = (guid: string) => {
    stay(guid);
    // A moment to see the tick land (and undo a mistaken one), then the row fades and goes.
    // Opacity only, drawn by the page frame by frame (lib/fade): a height fold, or anything
    // handed to the compositor, flickered under WebKitGTK on Hyprland.
    departure.current.set(guid, [
      window.setTimeout(() => {
        setLeaving((was) => new Set(was).add(guid));
        const row = document.querySelector<HTMLElement>(`.rx-row[data-guid="${CSS.escape(guid)}"]`);
        // A row ticked from the keyboard hands the keys on to the one below it (or above),
        // rather than dropping them on the page as it goes.
        const keys = row?.contains(document.activeElement) ?? false;
        const rowsNow = [...document.querySelectorAll<HTMLElement>(".rx-rows .rx-row")];
        const at = row ? rowsNow.indexOf(row) : -1;
        const heir = keys ? (rowsNow.slice(at + 1).find((el) => !el.classList.contains("is-leaving")) ?? rowsNow[at - 1]) : null;
        const heirGuid = heir?.dataset.guid ?? null;
        const gone = () => {
          setDeparted((was) => new Set(was).add(guid));
          if (heirGuid) {
            requestAnimationFrame(() => {
              const active = document.activeElement;
              if (active && active !== document.body && document.contains(active)) return;
              document.querySelector<HTMLElement>(`.rx-row[data-guid="${CSS.escape(heirGuid)}"]`)?.focus();
            });
          }
        };
        if (!row) gone();
        else fadeOut(row, gone, 160);
      }, 600),
    ]);
  };
  useEffect(() => {
    const timers = departure.current;
    return () => timers.forEach((ts) => ts.forEach((t) => window.clearTimeout(t)));
  }, []);

  /** Each reminder's ticks, sent to Apple one after another, in the order they were made. */
  const ticks = useRef(new Map<string, Promise<unknown>>());
  /**
   * A repeating reminder ticked: shown ticked for a moment — the circle filled — before it
   * rolls on to its next date, as on the Mac; and while it does, a second click on it is not
   * taken. Rolled on at once, the row came back unticked as if nothing had happened, a click
   * again ticked the next day too, and a daily reminder skipped a day (one ticked twice three
   * seconds apart came out due the day after tomorrow).
   */
  const [settling, setSettling] = useState<Set<string>>(() => new Set());
  // Read by the click itself: a double-click's second click comes before the redraw.
  const held = useRef(new Set<string>());
  const SETTLE_MS = 1500;
  /** Whether ticking it moves it on: a repeat, unless this is the last time before its end. */
  const rollsOn = (r: Reminder) => repeatLabel(r) !== null && !ending.has(r.guid);
  const toggle = (r: Reminder) => {
    if (r.guid.startsWith(STAND_IN)) return;
    const wasOpen = !r.completedAt;
    const repeating = rollsOn(r);
    if (repeating) {
      // Not taken while it settles, nor until Apple has the tick before; once it shows open
      // again with its next date, a tick counts.
      if (held.current.has(r.guid) || ticks.current.has(r.guid)) return;
      if (wasOpen) {
        held.current.add(r.guid);
        setSettling((was) => new Set(was).add(r.guid));
        window.setTimeout(() => {
          held.current.delete(r.guid);
          setSettling((was) => {
            const next = new Set(was);
            next.delete(r.guid);
            return next;
          });
        }, SETTLE_MS);
      }
    }
    // Shown at once, every time, even while an earlier tick of this one is on its way — a
    // repeating one too: its circle fills now, and it rolls on to its next date when Apple
    // has made it (waiting for that, a tick on one showed nothing at all for a second or two).
    setItems(patch(r.guid, { completedAt: wasOpen ? Math.floor(Date.now() / 1000) : null }));
    // Only where completed reminders are hidden, and never for a repeating one, which rolls on.
    // Its last time before End Repeat is a completion too, as on the phone.
    // In Completed, a reminder unticked is no longer completed, and leaves it the same way.
    if ((wasOpen && !showDone && !repeating) || (!wasOpen && smartKind === "completed")) depart(r.guid);
    else stay(r.guid);
    const before = ticks.current.get(r.guid) ?? Promise.resolve();
    const next = before.then(() => send(r, wasOpen));
    ticks.current.set(r.guid, next);
    void next.finally(() => {
      if (ticks.current.get(r.guid) === next) ticks.current.delete(r.guid);
    });
  };
  const send = async (r: Reminder, wasOpen: boolean) => {
    const fresh = await write(
      r,
      () => api.reminderSetDone(r.guid, r.listGuid, wasOpen),
      // Held over any refresh until Apple has it. A repeating one then comes back from Apple
      // rolled on, open again with its next date.
      patch(r.guid, { completedAt: wasOpen ? Math.floor(Date.now() / 1000) : null }),
    );
    if (!fresh) stay(r.guid);
    if (!fresh || !wasOpen || !rollsOn(r)) return;
    const after = fresh.find((x) => x.guid === r.guid);
    if (after && after.dueAt !== null && after.dueAt !== r.dueAt) {
      // A day's reminder moves to a day: said without the time it does not have.
      setMoved({ guid: r.guid, text: dueLabel(after.dueAt, false, after.allDay).text });
    }
  };
  const flag = (r: Reminder) =>
    write(
      r,
      () => api.reminderEdit(r.guid, r.listGuid, { flagged: !r.flagged }),
      patch(r.guid, { flagged: !r.flagged }),
    );
  const remove = (r: Reminder) =>
    write(
      r,
      () => api.reminderDelete(r.guid, r.listGuid),
      // A subtask goes with its parent, as on the phone.
      (rows) => rows.filter((x) => x.guid !== r.guid && x.parentGuid !== r.guid),
    );
  const rename = (r: Reminder, title: string) =>
    title.trim() && title.trim() !== r.title
      ? write(
          r,
          () => api.reminderEdit(r.guid, r.listGuid, { title: title.trim() }),
          patch(r.guid, { title: title.trim() }),
        )
      : Promise.resolve();
  /** A date, and whether it has a time — the phone's Date and Time switches. */
  const setDue = (r: Reminder, day: string, time: string | null) => {
    const allDay = Boolean(day) && time === null;
    // A time is read on the clock of the zone it is kept in, as the phone reads it: 11:00 in
    // New York stays 11:00 in New York, whatever this computer's own clock says.
    const dueAt = !day ? null : allDay ? dayToUtc(day) : timedInstant(day, time ?? "09:00", zoneOf(r));
    return write(
      r,
      () => api.reminderEdit(r.guid, r.listGuid, { dueAt, allDay }),
      patch(r.guid, { dueAt: allDay && dueAt !== null ? dayToLocal(dueAt) : dueAt, allDay }),
    );
  };
  const setNotes = (r: Reminder, notes: string) => {
    // Only a real change is sent: leaving the field as it was sent the notes as shown, which
    // could be older than an edit made on the phone that had not arrived yet — and replace it.
    if ((notes.trim() || null) === (r.notes?.trim() || null)) return;
    return write(r, () =>
      api.reminderEdit(r.guid, r.listGuid, { notes: notes.trim() || null }),
    );
  };
  /** A repeat, or none, and when it ends — shown at once, then sent. Changing how often keeps
   *  the end date, as the phone does. */
  const setRepeat = (r: Reminder, repeat: { frequency: number; interval: number } | null, endAt: number | null, days: number[] = []) => {
    const daysOfWeek = repeat?.frequency === 1 ? days : [];
    setMoreOf((m) =>
      m && m.guid === r.guid
        ? { ...m, repeat: repeat ? { frequency: repeat.frequency, interval: repeat.interval, endAt, occurrenceCount: 0, daysOfWeek, other: [] } : null }
        : m,
    );
    setMarks((was) => {
      const next = new Map(was);
      const at = next.get(r.guid) ?? { guid: r.guid, daysOfWeek: [], urgent: false, location: null, messaging: null, url: null };
      next.set(r.guid, { ...at, daysOfWeek });
      return next;
    });
    return write(
      r,
      () => api.reminderSetRepeat(r.guid, r.listGuid, repeat, repeat ? endAt : null, daysOfWeek),
      patch(r.guid, { recurFrequency: repeat?.frequency ?? null, recurInterval: repeat?.interval ?? null }),
    );
  };
  const setPriority = (r: Reminder, priority: number) =>
    write(r, () => api.reminderEdit(r.guid, r.listGuid, { priority }));
  /** Nest a reminder under the one above it, the way Tab does in Apple's app. */
  const indent = (r: Reminder) => {
    const flat = threaded.map((t) => t.reminder);
    const index = flat.findIndex((x) => x.guid === r.guid);
    // Only under something above it, and never under another subtask: Apple nests one deep.
    const above = flat
      .slice(0, index)
      .reverse()
      .find((x) => !x.parentGuid);
    // One with subtasks of its own stays where it is: nested, they would be two deep, which
    // Apple does not keep, and they vanished from the list.
    if (!above || flat.some((x) => x.parentGuid === r.guid)) return Promise.resolve(null);
    return write(
      r,
      () => api.reminderEdit(r.guid, r.listGuid, { parentGuid: above.guid }),
      patch(r.guid, { parentGuid: above.guid }),
    );
  };

  /** The open reminder's tags, link and early alerts, read again. */
  const readDetail = useCallback(async (guid: string | null) => {
    if (!guid || guid.startsWith(STAND_IN)) {
      setDetailOf(null);
      return;
    }
    try {
      const found = await remindersApi.detail(guid);
      setDetailOf((was) => (found && found.guid === guid ? found : was?.guid === guid ? was : null));
    } catch {
      // An older library without details: the panel shows what the reminder itself holds.
      setDetailOf(null);
    }
  }, []);
  useEffect(() => {
    void readDetail(openGuid);
  }, [openGuid, readDetail, revision, outbox.tick]);
  useEffect(() => {
    let live = true;
    remindersApi
      .ending()
      .then((guids) => live && setEnding(new Set(guids)))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [revision, outbox.tick, items.length, moreOf?.repeat?.endAt]);
  // The rows' marks, read again when the rows or Apple's copy change.
  const markKey = useMemo(() => items.map((r) => r.guid).join(","), [items]);
  useEffect(() => {
    const guids = items.filter((r) => !r.guid.startsWith(STAND_IN)).map((r) => r.guid);
    if (!guids.length) return;
    let live = true;
    remindersApi
      .marks(guids)
      .then((found) => live && setMarks(new Map(found.map((m) => [m.guid, m]))))
      .catch(() => {});
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [markKey, revision, outbox.tick]);
  /** The rest of the open reminder's Details, read again with it. */
  useEffect(() => {
    const guid = openGuid;
    if (!guid || guid.startsWith(STAND_IN)) {
      setMoreOf(null);
      return;
    }
    let live = true;
    remindersApi
      .more(guid)
      .then((found) => live && setMoreOf(found))
      .catch(() => live && setMoreOf((was) => (was?.guid === guid ? was : null)));
    return () => {
      live = false;
    };
  }, [openGuid, revision, outbox.tick]);
  // Tags and smart lists hold what is still to do, so a change sent or ticked moves them.
  useEffect(() => {
    void readExtras().catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [outbox.tick]);

  const openTag = (name: string) => {
    setQuery("");
    setSelection({ type: "tag", name });
    setDetail(true);
  };

  /**
   * A change to what sits beside a reminder: shown at once in its row and its info, sent to
   * Apple, then the details read again — Apple's copy on success, the old one on a refusal.
   */
  const detailWrite = async (r: Reminder, run: () => Promise<void>, predict?: (d: ReminderDetail) => ReminderDetail) => {
    if (!REMINDER_DETAIL_WRITES || r.guid.startsWith(STAND_IN)) return;
    if (predict) {
      setDetailOf((d) => (d && d.guid === r.guid ? predict(d) : d));
      setTagsOf((m) => {
        const base = { guid: r.guid, tags: m.get(r.guid) ?? [], url: null, earlyAlerts: [], otherAttachments: 0, images: [] };
        const next = new Map(m);
        next.set(r.guid, predict(base).tags);
        return next;
      });
    }
    await write(r, run);
    // The reminder open after the write, not the one open when it began: a write takes a few
    // seconds, and re-reading the earlier one showed its details on whatever was opened since.
    await Promise.all([readDetail(openRef.current), readExtras().catch(() => {})]);
  };
  const tagAdd = (r: Reminder, raw: string) => {
    const name = raw.trim().replace(/^#+/, "").replace(/\s+/g, "");
    if (!name) return Promise.resolve();
    return detailWrite(r, () => remindersApi.tagAdd(r.guid, name), (d) =>
      d.tags.some((t) => t.toLowerCase() === name.toLowerCase()) ? d : { ...d, tags: [...d.tags, name] },
    );
  };
  const tagRemove = (r: Reminder, name: string) =>
    detailWrite(r, () => remindersApi.tagRemove(r.guid, name), (d) => ({ ...d, tags: d.tags.filter((t) => t !== name) }));
  const setUrl = (r: Reminder, url: string | null) =>
    detailWrite(r, () => remindersApi.urlSet(r.guid, url), (d) => ({ ...d, url }));
  const setEarly = (r: Reminder, alert: [number, number] | null) =>
    detailWrite(r, () => remindersApi.earlyAlertSet(r.guid, alert?.[0] ?? null, alert?.[1] ?? null), (d) => ({
      ...d,
      earlyAlerts: alert ? [{ id: "pending", count: alert[0], unit: alert[1] }] : [],
    }));

  /** The section of its list a reminder sits in, when that section is still there. */
  const sectionOf = (r: Reminder) =>
    r.sectionGuid && listSections.some((x) => x.guid === r.sectionGuid) ? r.sectionGuid : null;
  const moveToSection = (r: Reminder, section: string | null) =>
    !REMINDER_DETAIL_WRITES || section === sectionOf(r)
      ? Promise.resolve(null)
      : write(r, () => remindersApi.sectionMove(r.listGuid, r.guid, section), patch(r.guid, { sectionGuid: section }));
  /** A new section, named in place at the foot of the list. */
  const [namingSection, setNamingSection] = useState<string | null>(null);
  const createSection = async (title: string) => {
    if (!REMINDER_DETAIL_WRITES || !listGuid || !title.trim()) return;
    const temp: ReminderSection = { guid: `${STAND_IN}section`, listGuid, title: title.trim(), createdAt: null };
    setListSections((was) => [...was, temp]);
    try {
      await remindersApi.sectionCreate(listGuid, title.trim());
    } catch (e) {
      setError(messageOf(e));
    }
    await readItemsRef.current().catch(() => {});
  };
  const renameSection = async (section: ReminderSection, title: string) => {
    if (!REMINDER_DETAIL_WRITES || !title.trim() || title.trim() === section.title) return;
    setListSections((was) => was.map((x) => (x.guid === section.guid ? { ...x, title: title.trim() } : x)));
    try {
      await remindersApi.sectionRename(section.guid, title.trim());
    } catch (e) {
      setError(messageOf(e));
    }
    await readItemsRef.current().catch(() => {});
  };

  /** Deleting asks first: there is no bringing a reminder back from here. */
  const askRemove = async (r: Reminder) => {
    const subtasks = items.filter((x) => x.parentGuid === r.guid).length;
    const sure = await confirm({
      title: `Delete \u201c${r.title || "New Reminder"}\u201d?`,
      body: subtasks
        ? `Its ${subtasks === 1 ? "subtask goes" : `${subtasks} subtasks go`} with it, on all your devices.`
        : "It goes from all your devices.",
      action: "Delete",
      danger: true,
      symbol: "trash",
    });
    if (!sure) return;
    if (openGuid === r.guid) setOpenGuid(null);
    // The focus moves on to the row below, or above, so the keys carry on from there.
    const rowsNow = [...document.querySelectorAll<HTMLElement>(".rx-rows .rx-row")];
    const at = rowsNow.findIndex((el) => el.dataset.guid === r.guid);
    const neighbour = rowsNow[at + 1] ?? rowsNow[at - 1];
    const next = neighbour?.dataset.guid ?? null;
    // The row goes at once (remove shows it before Apple answers), and the keys with it.
    void remove(r);
    if (next) {
      requestAnimationFrame(() =>
        document.querySelector<HTMLElement>(`.rx-row[data-guid="${CSS.escape(next)}"]`)?.focus(),
      );
    }
  };

  /**
   * The keys Reminders on the Mac answers, with Ctrl for Command: Ctrl+N a new reminder,
   * Ctrl+Shift+N a new list, Ctrl+F the search, Ctrl+I the selected reminder's info. For the
   * selected reminder — its row, or its title being typed in — Ctrl+Shift+F flags it,
   * Ctrl+Shift+C completes it and Ctrl+T makes it due today; Ctrl+Shift+H shows or hides a
   * list's completed reminders; Ctrl+E shows every subtask and Ctrl+Shift+E hides them. On a
   * row: Up and Down move, Space ticks it, Return edits it,
   * Delete deletes it (asking first).
   */
  const live = useActiveRef();
  const searchRef = useRef<HTMLInputElement>(null);
  /** Ctrl+N from a view with nowhere to add: the default list opens, and its field takes it. */
  const [wantAdd, setWantAdd] = useState(0);
  useEffect(() => {
    if (!wantAdd) return;
    const field = document.querySelector<HTMLInputElement>(".rx-add input");
    if (!field) return;
    field.focus();
    field.scrollIntoView({ block: "nearest" });
    setWantAdd(0);
  }, [wantAdd, selection, items]);
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e: KeyboardEvent) => {
    if (!live.current || e.defaultPrevented) return;
    // A menu, a picker's list or a sheet over the screen has the keys to itself — not another
    // screen's own list (Photos' grid stays mounted once visited), which overlayOpen skips.
    if (overlayOpen()) return;
    const target = e.target as HTMLElement | null;
    const typing = !!target?.closest?.("input, textarea, select, [contenteditable]");
    const command = (e.ctrlKey || e.metaKey) && !e.altKey;
    const k = e.key.toLowerCase();
    if (command && k === "n") {
      e.preventDefault();
      if (e.shiftKey) {
        setDetail(false);
        setAddingList(true);
        document.querySelector<HTMLInputElement>(".rx-new-list input")?.focus();
        return;
      }
      if (!targetListGuid) return;
      setDetail(true);
      if (searching) setQuery("");
      if (!canAdd) setSelection({ type: "list", guid: targetListGuid });
      setWantAdd((n) => n + 1);
      return;
    }
    if (command && !e.shiftKey && k === "f") {
      e.preventDefault();
      searchRef.current?.focus();
      searchRef.current?.select();
      return;
    }
    // The reminder the keys act on: the row the focus is in, or the one whose details are open.
    const chosen = () => {
      const row = (document.activeElement as HTMLElement | null)?.closest?.<HTMLElement>(".rx-row");
      // In a row with no reminder yet (the next one being typed), there is none to act on.
      const guid = row ? (row.dataset.guid ?? null) : openGuid;
      return guid ? (items.find((r) => r.guid === guid) ?? null) : null;
    };
    if (command && e.shiftKey && (k === "f" || k === "c")) {
      e.preventDefault();
      if (e.repeat) return;
      const r = chosen();
      if (!r || r.guid.startsWith(STAND_IN)) return;
      if (k === "f") void flag(r);
      else toggle(r);
      return;
    }
    if (command && k === "e") {
      // Show All Subtasks (Ctrl+E) and Hide All Subtasks (Ctrl+Shift+E), as the View menu has them.
      e.preventDefault();
      const next = e.shiftKey ? new Set([...hiddenSubtasks, ...kidsOf.keys()]) : new Set<string>();
      setHiddenSubtasks(next);
      keepHiddenSubtasks(next);
      return;
    }
    if (command && e.shiftKey && k === "h") {
      e.preventDefault();
      if (inList) setShowDone((v) => !v);
      return;
    }
    if (command && !e.shiftKey && k === "t") {
      // Due Today: the day moves to today; a time it had is kept, on its own zone's clock.
      e.preventDefault();
      if (e.repeat) return;
      const r = chosen();
      if (!r || r.guid.startsWith(STAND_IN)) return;
      const now = new Date();
      const pad2 = (n: number) => String(n).padStart(2, "0");
      const day = `${now.getFullYear()}-${pad2(now.getMonth() + 1)}-${pad2(now.getDate())}`;
      const w = r.dueAt !== null && !r.allDay ? wallIn(zoneOf(r), r.dueAt) : null;
      void setDue(r, day, w ? `${pad2(w.h)}:${pad2(w.min)}` : null);
      return;
    }
    if (command && !e.shiftKey && k === "i") {
      e.preventDefault();
      const row = (document.activeElement as HTMLElement | null)?.closest?.<HTMLElement>(".rx-row");
      const guid = row?.dataset.guid ?? openGuid;
      if (!guid) return;
      setOpenGuid(openGuid === guid ? null : guid);
      // The row keeps the keys, so Ctrl+I again closes what it opened.
      if (row && openGuid === guid) row.focus();
      return;
    }
    if (typing || command || e.altKey) return;
    // Arrows from nowhere in particular start at the first (or last) reminder.
    if ((e.key === "ArrowDown" || e.key === "ArrowUp") && !target?.closest?.(".rx-row, .rx-side, button, [role]")) {
      const rows = [...document.querySelectorAll<HTMLElement>(".rx-rows .rx-row")];
      const pick = e.key === "ArrowDown" ? rows[0] : rows[rows.length - 1];
      if (pick) {
        e.preventDefault();
        pick.focus();
      }
    }
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => keys.current(e);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  /** Drag and drop: what is being dragged (state, for its own row's look; a ref, for the rows
   *  it passes over), and the row it would land before — marked on that row's element. */
  const [dragging, setDragging] = useState<string | null>(null);
  const draggingRef = useRef<string | null>(null);
  const dropRow = useRef<HTMLElement | null>(null);
  const markDrop = (row: HTMLElement | null) => {
    if (dropRow.current !== row) dropRow.current?.classList.remove("is-drop-before");
    // Added again on every dragover: a row drawn again mid-drag takes its class from React.
    row?.classList.add("is-drop-before");
    dropRow.current = row;
  };
  const canReorder = inList && !searching;
  /** Put a reminder before another in its list — at once, then to Apple; back if refused. */
  const reorder = (guid: string, before: string) => {
    const r = items.find((x) => x.guid === guid);
    if (!r || !listGuid) return Promise.resolve(null);
    // Dropped among another section's reminders, it goes into that section too, as on the
    // phone: put in order alone, it sprang back under its old heading.
    const target = items.find((x) => x.guid === before);
    const into = target ? sectionOf(target) : sectionOf(r);
    const across = REMINDER_DETAIL_WRITES && into !== sectionOf(r);
    return write(
      r,
      async () => {
        await api.reminderReorder(listGuid, guid, before);
        if (across) await remindersApi.sectionMove(listGuid, guid, into);
      },
      (rows) => {
        const found = rows.find((x) => x.guid === guid);
        if (!found) return rows;
        const moved = across ? { ...found, sectionGuid: into } : found;
        const rest = rows.filter((x) => x.guid !== guid);
        const at = rest.findIndex((x) => x.guid === before);
        return [...rest.slice(0, at < 0 ? rest.length : at), moved, ...rest.slice(at < 0 ? rest.length : at)];
      },
    );
  };

  const outdent = (r: Reminder) =>
    write(
      r,
      () => api.reminderEdit(r.guid, r.listGuid, { parentGuid: null }),
      patch(r.guid, { parentGuid: null }),
    );

  // The reminder goes, and its subtasks with it — the backend moves them after it, as the
  // phone does — at once: out of the list on screen, or re-filed where a view gathers lists.
  const moveToList = (r: Reminder, listGuid: string) =>
    listGuid === r.listGuid
      ? Promise.resolve(null)
      : write(
          r,
          () => api.reminderEdit(r.guid, r.listGuid, { listGuid }),
          (rows) => {
            const moving = (x: Reminder) => x.guid === r.guid || x.parentGuid === r.guid;
            return selection.type === "list"
              ? rows.filter((x) => !moving(x))
              : rows.map((x) => (moving(x) ? { ...x, listGuid } : x));
          },
        );

  /** List writes go to Apple first, then the whole set is re-read. */
  const listWrite = useCallback(
    async (run: () => Promise<void>) => {
      setSyncing(true);
      setError(null);
      try {
        await run();
        await readLists();
        await readItemsRef.current();
      } catch (e) {
        setError(messageOf(e));
      } finally {
        setSyncing(false);
      }
    },
    [readLists, readItems],
  );

  /**
   * The first list on an account that has none.
   *
   * Named the way Apple names the one it makes for a new account, so the two accounts end up
   * looking the same rather than this one carrying a name only this app would have chosen.
   */
  async function makeFirstList() {
    await listWrite(() => api.reminderListCreate("Reminders"));
  }

  async function createList(e: React.FormEvent) {
    e.preventDefault();
    const title = newList.trim();
    if (!title) return;
    await listWrite(() => api.reminderListCreate(title));
    setNewList("");
    setAddingList(false);
  }

  /**
   * A smart list is a view, not somewhere to keep things, so adding from Today or All has
   * to file the reminder in a real list — as Apple's own app does.
   *
   * Which one was previously just "the first one", which is why everything landed in
   * Assignments with no explanation. Apple's default-list setting is a client preference it
   * does not publish anywhere this app can read, so rather than guess at it, the choice is
   * shown and it is yours: whichever list you last added to, changeable right there.
   */
  const [preferredList, setPreferredList] = useState<string | null>(() => {
    try {
      return window.localStorage.getItem("quince.reminders.defaultList");
    } catch {
      return null;
    }
  });

  const chooseList = useCallback((guid: string) => {
    setPreferredList(guid);
    try {
      window.localStorage.setItem("quince.reminders.defaultList", guid);
    } catch {
      // A browser with storage switched off still works; it just forgets the choice.
    }
  }, []);

  const targetListGuid =
    listGuid ??
    (preferredList && lists.some((l) => l.guid === preferredList)
      ? preferredList
      : (lists[0]?.guid ?? null));
  const targetList = lists.find((l) => l.guid === targetListGuid) ?? null;

  /** A line typed at the foot of the view, added; `false` when it could not be. */
  async function add(typed: string, said: QuickReminder | null): Promise<boolean> {
    if (!typed || !targetListGuid) return false;
    const title = said ? said.title : typed;
    setSyncing(true);
    setError(null);
    // What the view implies, as on the phone: typed into Today or Scheduled it is due today,
    // into Flagged it is flagged — so it stays where it was typed, rather than vanishing.
    const kind = selection.type === "smart" ? selection.kind : null;
    const today = new Date();
    const pad2 = (n: number) => String(n).padStart(2, "0");
    const todayKey = `${today.getFullYear()}-${pad2(today.getMonth() + 1)}-${pad2(today.getDate())}`;
    const keyOf = (d: Date) => `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
    const viewImplied: { dueAt?: number; allDay?: boolean; flagged?: boolean } =
      kind === "today" || kind === "scheduled"
        ? { dueAt: dayToUtc(todayKey), allDay: true }
        : kind === "flagged"
          ? { flagged: true }
          : {};
    // A day or time typed wins over the one the view implies; a day is kept as Apple keeps
    // one (that day's midnight in UTC), a time as the moment it is.
    const implied: { dueAt?: number; allDay?: boolean; flagged?: boolean } =
      said?.dueAt != null
        ? {
            ...viewImplied,
            ...(said.allDay
              ? { dueAt: dayToUtc(keyOf(new Date(said.dueAt * 1000))), allDay: true }
              : { dueAt: said.dueAt, allDay: false }),
          }
        : viewImplied;
    // The row is there the moment Return is pressed, and the field is ready for the next.
    const standIn = `${STAND_IN}${Date.now()}`;
    const tagged = selection.type === "tag" ? selection.name : null;
    const shownHere = inList || tagged !== null || kind === "today" || kind === "scheduled" || kind === "flagged" || kind === "all";
    if (shownHere) setItems((rows) => [
      ...rows,
      {
        guid: standIn, listGuid: targetListGuid, title, notes: null,
        dueAt: implied.dueAt === undefined ? null : implied.allDay ? dayToLocal(implied.dueAt) : implied.dueAt,
        allDay: implied.allDay ?? false,
        completedAt: null, priority: said?.priority ?? 0, flagged: implied.flagged ?? false, parentGuid: null,
        recurFrequency: said?.repeat?.frequency ?? null, recurInterval: said?.repeat?.interval ?? null, etag: null,
      } as Reminder,
    ]);
    try {
      const made = await api.reminderCreate(targetListGuid, title, implied);
      // The priority and repeat the line named, through the same edits the detail panel makes.
      if (said && typeof made === "string" && made) {
        if (said.priority) await api.reminderEdit(made, targetListGuid, { priority: said.priority }).catch((e) => setError(messageOf(e)));
        if (said.repeat) await api.reminderSetRepeat(made, targetListGuid, said.repeat).catch((e) => setError(messageOf(e)));
      }
      // Typed into a tag, it carries the tag, as on the phone.
      if (tagged && REMINDER_DETAIL_WRITES && typeof made === "string" && made) {
        await remindersApi.tagAdd(made, tagged).catch((e) => setError(messageOf(e)));
        void readExtras().catch(() => {});
      }
      await readItemsRef.current();
      return true;
    } catch (err) {
      setItems((rows) => rows.filter((r) => r.guid !== standIn));
      setError(messageOf(err));
      return false;
    } finally {
      setSyncing(false);
    }
  }

  /**
   * Return on a reminder in a list begins the next one right below it, as on the Mac: a new
   * row there, its title being typed. A subtask's next is a subtask of the same reminder, and
   * one in a section stays in that section. Nothing is made until something is typed.
   */
  /** The reminder the next one goes below; `was` its stand-in's id, once it has its own. */
  const [insertAfter, setInsertAfter] = useState<(Reminder & { was?: string }) | null>(null);
  const addBelow = async (typed: string, anchor: Reminder & { was?: string }, goOn: boolean) => {
    const line = typed.trim();
    if (!line || !listGuid) return;
    const said = parseReminder(line);
    const q = understood(said) ? said : null;
    const title = q ? q.title : line;
    const parentGuid = anchor.parentGuid;
    const section = sectionOf(anchor);
    const keyOf = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    const implied =
      q?.dueAt != null
        ? q.allDay
          ? { dueAt: dayToUtc(keyOf(new Date(q.dueAt * 1000))), allDay: true }
          : { dueAt: q.dueAt, allDay: false }
        : {};
    // Where it goes in the list's order: before the reminder that follows this one's block in
    // the same section, or last in it when there is none.
    const flat = threaded.map((t) => t.reminder);
    // The anchor by its id, or by its stand-in's while the list has not been read since it was made.
    const isAnchor = (x: Reminder) => x.guid === anchor.guid || (anchor.was !== undefined && x.guid === anchor.was);
    const at = flat.findIndex(isAnchor);
    const next = parentGuid || at < 0
      ? null
      : flat.slice(at + 1).find((x) => !x.parentGuid && !x.completedAt && sectionOf(x) === section && !x.guid.startsWith(STAND_IN)) ?? null;
    const standIn = `${STAND_IN}${Date.now()}`;
    // Shown in its place at once, the field going on below it for the next.
    const shown = {
      guid: standIn, listGuid, title, notes: null,
      dueAt: implied.dueAt === undefined ? null : implied.allDay ? dayToLocal(implied.dueAt) : implied.dueAt,
      allDay: implied.allDay ?? false, completedAt: null, priority: q?.priority ?? 0, flagged: false,
      parentGuid, sectionGuid: section, recurFrequency: q?.repeat?.frequency ?? null,
      recurInterval: q?.repeat?.interval ?? null, etag: null,
    } as Reminder;
    setItems((rows) => {
      const i = rows.findIndex(isAnchor);
      // After the anchor's own subtasks, where the row is drawn.
      let end = i;
      if (!parentGuid) while (end + 1 < rows.length && rows[end + 1].parentGuid === rows[i].guid) end += 1;
      return i < 0 ? [...rows, shown] : [...rows.slice(0, end + 1), shown, ...rows.slice(end + 1)];
    });
    // Return goes on to another below this one; leaving the field ends there.
    setInsertAfter(goOn ? shown : null);
    setError(null);
    try {
      const made = await api.reminderCreate(listGuid, title, implied);
      if (typeof made === "string" && made) {
        // The field below follows the real reminder once it has its id.
        setInsertAfter((was) => (was?.guid === standIn ? { ...shown, guid: made, was: standIn } : was));
        if (next) await api.reminderReorder(listGuid, made, next.guid).catch((e) => setError(messageOf(e)));
        if (section && REMINDER_DETAIL_WRITES) await remindersApi.sectionMove(listGuid, made, section).catch((e) => setError(messageOf(e)));
        if (parentGuid) await api.reminderEdit(made, listGuid, { parentGuid }).catch((e) => setError(messageOf(e)));
        if (q?.priority) await api.reminderEdit(made, listGuid, { priority: q.priority }).catch((e) => setError(messageOf(e)));
        if (q?.repeat) await api.reminderSetRepeat(made, listGuid, q.repeat).catch((e) => setError(messageOf(e)));
      }
      await readItemsRef.current();
    } catch (err) {
      setItems((rows) => rows.filter((r) => r.guid !== standIn));
      setInsertAfter((was) => (was?.guid === standIn ? null : was));
      setError(messageOf(err));
    }
  };

  const heading = inList
    ? (lists.find((l) => l.guid === listGuid)?.title ?? "Reminders")
    : selection.type === "tag"
      ? `#${selection.name}`
      : selection.type === "custom"
        ? (openCustom?.title ?? "Smart List")
        : (SMART.find((s) => s.kind === smartKind)?.title ?? "Reminders");
  const outstanding = items.filter((r) => !r.completedAt).length;

  /**
   * How many this view actually holds, when that is more than it was handed.
   *
   * The counts come from the library rather than from the list on screen, so they stay true
   * even where the list is only the first thousand of them. Saying nothing would be the
   * dishonest part — a list that quietly stops is worse than one that says where it stopped.
   */
  const heldBack = (() => {
    if (items.length < MOST_AT_ONCE || !counts || gathered) return 0;
    const total = inList
      ? (counts.lists.find(([guid]) => guid === listGuid)?.[1] ?? 0)
      : smartKind ? counts[smartKind] : 0;
    return Math.max(0, total - items.length);
  })();

  /**
   * Reminders in Apple's order, with each subtask directly beneath its parent.
   *
   * The library returns a flat list sorted by date and priority, which is right for the
   * top level and wrong for what hangs off it: a subtask sorted away from its parent reads
   * as an unrelated fragment. This re-threads them without re-sorting either level.
   */
  // What is on screen: the list less any ticked reminder that has already left it.
  const shownItems = useMemo(
    () => (departed.size ? items.filter((r) => !departed.has(r.guid)) : items),
    [items, departed],
  );
  const threaded = useMemo(() => {
    const children = new Map<string, Reminder[]>();
    for (const r of shownItems) {
      if (!r.parentGuid) continue;
      const bucket = children.get(r.parentGuid);
      if (bucket) bucket.push(r);
      else children.set(r.parentGuid, [r]);
    }
    const known = new Set(shownItems.map((r) => r.guid));
    const out: { reminder: Reminder; depth: number }[] = [];
    // Open reminders first, in their list's own order; completed ones beneath them, the most
    // recently completed first, as Reminders shows them. In the list's stored order they came
    // out scattered among the open ones, in no order a person could follow.
    const open = shownItems.filter((r) => !r.completedAt);
    const done = shownItems
      .filter((r) => r.completedAt)
      .sort((a, b) => (b.completedAt ?? 0) - (a.completedAt ?? 0));
    for (const r of [...open, ...done]) {
      // A subtask whose parent is not in this view — filtered out, or in another list —
      // has to appear at the top level rather than vanish.
      if (r.parentGuid && known.has(r.parentGuid)) continue;
      out.push({ reminder: r, depth: 0 });
      // Folded away under it, its subtasks are not drawn at all — until it is opened again.
      if (hiddenSubtasks.has(r.guid)) continue;
      for (const child of children.get(r.guid) ?? []) {
        out.push({ reminder: child, depth: 1 });
      }
    }
    return out;
  }, [shownItems, hiddenSubtasks]);
  /** How many subtasks each reminder in view has, for the arrow that folds them. */
  const kidsOf = useMemo(() => {
    const known = new Set(shownItems.map((r) => r.guid));
    const out = new Map<string, number>();
    for (const r of shownItems) if (r.parentGuid && known.has(r.parentGuid)) out.set(r.parentGuid, (out.get(r.parentGuid) ?? 0) + 1);
    return out;
  }, [shownItems]);
  /**
   * The rows arranged the way Apple arranges them.
   *
   * Every one of Apple's smart lists is grouped, and this app showed each of them as one flat
   * run of reminders — which is most of why the screen felt unlike the real thing even when
   * the same reminders were on it. Today breaks by time of day, Scheduled by date, and the
   * ones that gather from everywhere break by the list each reminder came from.
   *
   * A real list is left alone: Apple does not group inside one either.
   */
  /**
   * Whether a row should name the list it came from.
   *
   * Apple shows it wherever the grouping does not already say so: in Today and Scheduled,
   * which gather from everywhere, but not under a list heading where it would repeat.
   */
  const showListName =
    !searching &&
    !inList &&
    !gathered &&
    smartKind !== "all" &&
    smartKind !== "flagged" &&
    smartKind !== "completed";

  const sections = useMemo(() => {
    if (searching) {
      // Results gather from every list, so the list each came from is the grouping — as in
      // All.
      const byList = new Map<string, typeof threaded>();
      for (const row of threaded) {
        const bucket = byList.get(row.reminder.listGuid);
        if (bucket) bucket.push(row);
        else byList.set(row.reminder.listGuid, [row]);
      }
      return [...byList.entries()].map(([guid, rows]) => {
        const list = listById.get(guid);
        return {
          key: guid,
          heading: list?.title ?? "Reminders",
          colour: list?.colour ?? null,
          dated: false,
          rows,
        };
      });
    }
    if (inList) {
      // Inside a real list the grouping is the list's own sections, which is how Apple shows
      // it: reminders outside any section first with no heading, then each section in the
      // order it was made. A reminder placed in a section that is not here — one deleted
      // since — falls back to the loose ones rather than vanishing.
      const known = new Set(listSections.map((s) => s.guid));
      const loose = threaded.filter(
        (row) => !row.reminder.sectionGuid || !known.has(row.reminder.sectionGuid)
      );
      return [
        { key: "loose", heading: null, colour: null, dated: false, rows: loose },
        ...listSections.map((section) => ({
          key: section.guid,
          heading: section.title,
          colour: null,
          dated: false,
          rows: threaded.filter((row) => row.reminder.sectionGuid === section.guid),
        })),
      ];
    }

    const kind = smartKind;
    const now = new Date();
    const listOf = (guid: string) => listById.get(guid);

    if (kind === "today") {
      // Apple's own buckets, each shown only when something is in it: empty "Morning" and
      // "Afternoon" headings above the one reminder due tonight read as missing reminders.
      const buckets: { key: string; heading: string | null; rows: typeof threaded }[] = [
        { key: "loose", heading: null, rows: [] },
        { key: "morning", heading: "Morning", rows: [] },
        { key: "afternoon", heading: "Afternoon", rows: [] },
        { key: "tonight", heading: "Tonight", rows: [] },
      ];
      // A subtask goes where its reminder went, directly under it: filed by its own time, it was
      // drawn nested under whatever reminder ended another part of the day.
      let parentBucket = 0;
      for (const row of threaded) {
        if (row.depth > 0) {
          buckets[parentBucket].rows.push(row);
          continue;
        }
        const due = row.reminder.dueAt;
        // Anything overdue, or with no time at all, sits above the buckets — which is where
        // Apple puts it. A day's reminder has no time either: read as its midnight, it went
        // under Morning, with every one added here.
        if (due === null || row.reminder.allDay || daysApart(now, new Date(due * 1000)) < 0) {
          parentBucket = 0;
        } else {
          const hour = new Date(due * 1000).getHours();
          parentBucket = hour < 12 ? 1 : hour < 17 ? 2 : 3;
        }
        buckets[parentBucket].rows.push(row);
      }
      // Each part of the day in time order, as Today reads; the list's own order put 9:30
      // above 8:30. A subtask keeps its place under its parent.
      // Its subtasks move with it.
      for (const b of buckets.slice(1)) {
        const units: (typeof threaded)[] = [];
        for (const r of b.rows) {
          if (r.depth > 0 && units.length) units[units.length - 1].push(r);
          else units.push([r]);
        }
        units.sort((x, y) => (x[0].reminder.dueAt ?? 0) - (y[0].reminder.dueAt ?? 0));
        b.rows = units.flat();
      }
      return buckets
        .filter((b) => b.rows.length > 0)
        // The buckets are all today, so a row under one needs only its time. The loose rows
        // above are overdue or undated and still have to say which day they mean.
        .map((b) => ({ ...b, colour: null, dated: b.heading !== null }));
    }

    if (kind === "scheduled") {
      const byDay = new Map<number, typeof threaded>();
      const overdue: typeof threaded = [];
      // A subtask goes with its reminder, directly under it, as in Today; one of a reminder not
      // shown here (no date) stands on its own day.
      let parentRows: typeof threaded | null = null;
      for (const row of threaded) {
        if (row.depth > 0 && parentRows) {
          parentRows.push(row);
          continue;
        }
        const due = row.reminder.dueAt;
        if (due === null) {
          parentRows = null;
          continue;
        }
        const days = daysApart(now, new Date(due * 1000));
        if (days < 0) parentRows = overdue;
        else {
          parentRows = byDay.get(days) ?? [];
          byDay.set(days, parentRows);
        }
        parentRows.push(row);
        if (row.depth > 0) parentRows = null;
      }
      const out = overdue.length
        ? [{ key: "overdue", heading: "Overdue", colour: null, dated: false, rows: overdue }]
        : [];
      for (const days of [...byDay.keys()].sort((a, b) => a - b)) {
        const when = new Date(now.getFullYear(), now.getMonth(), now.getDate() + days);
        out.push({
          key: `d${days}`,
          heading:
            days === 0
              ? "Today"
              : days === 1
                ? "Tomorrow"
                : dateText(when, {
                    weekday: "short",
                    day: "numeric",
                    month: "short",
                  }),
          colour: null,
          dated: true,
          rows: byDay.get(days) ?? [],
        });
      }
      return out;
    }

    // All, Flagged and Completed gather from everywhere, so the list each came from is the
    // grouping.
    const byList = new Map<string, typeof threaded>();
    for (const row of threaded) {
      const bucket = byList.get(row.reminder.listGuid);
      if (bucket) bucket.push(row);
      else byList.set(row.reminder.listGuid, [row]);
    }
    return [...byList.entries()]
      .sort(
        ([a], [b]) =>
          (listOf(a)?.sortOrder ?? 0) - (listOf(b)?.sortOrder ?? 0) ||
          (listOf(a)?.title ?? "").localeCompare(listOf(b)?.title ?? ""),
      )
      .map(([guid, rows]) => ({
        key: guid,
        heading: listOf(guid)?.title ?? "Reminders",
        colour: listOf(guid)?.colour ?? null,
        // Grouped by list rather than by day, so a row still has to name its day.
        dated: false,
        rows,
      }));
  }, [threaded, inList, selection, listById, listSections, searching, clock]);

  const countOf = (guid: string) => counts?.lists.find(([g]) => g === guid)?.[1] ?? 0;
  const openList = inList ? (lists.find((l) => l.guid === listGuid) ?? null) : null;
  const smartOpen = smartKind ? SMART.find((x) => x.kind === smartKind) : undefined;
  /** The colour the open view is drawn in: its list's, or its built-in view's. */
  const accent = searching
    ? "var(--text)"
    : openList
      ? colourOf(openList)
      : openCustom
        ? smartColour(openCustom)
        : selection.type === "tag"
          ? "var(--sk-blue)"
          : (smartOpen?.colour ?? "var(--sk-blue)");

  /**
   * The sidebar in Apple's order: a list or group at the top level wherever it was walked,
   * with a group's lists beneath it. Pinned lists are tiles instead, as on the phone.
   */
  const pinned = lists.filter((l) => l.pinned);
  const topLevel = [
    ...lists
      .filter((l) => !l.pinned && !(l.groupGuid && groups.some((g) => g.guid === l.groupGuid)))
      .map((list) => ({ kind: "list" as const, order: list.sortOrder, list })),
    ...groups.map((group) => ({ kind: "group" as const, order: group.sortOrder, group })),
  ].sort((a, b) => a.order - b.order);
  const inGroup = (guid: string) => lists.filter((l) => !l.pinned && l.groupGuid === guid);

  const listRow = (list: ReminderList, nested: boolean) => (
    <div className={`rx-list-row${nested ? " nested" : ""}${listMenu?.list.guid === list.guid ? " is-menu" : ""}`} key={list.guid}>
      {renaming === list.guid ? (
        <input
          className="field rx-rename"
          defaultValue={list.title}
          autoFocus
          aria-label={`Rename ${list.title}`}
          onBlur={(e) => {
            const title = e.target.value.trim();
            setRenaming(null);
            if (title && title !== list.title) {
              void listWrite(() => api.reminderListRename(list.guid, title));
            }
          }}
          onKeyDown={(e) => {
            if (composing(e)) return;
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") {
              e.currentTarget.value = list.title;
              e.currentTarget.blur();
            }
          }}
        />
      ) : (
        <>
          <button
            className="source-row rx-list"
            aria-current={picked.type === "list" && picked.guid === list.guid}
            onClick={() => {
              setQuery("");
              setSelection({ type: "list", guid: list.guid });
              setDetail(true);
            }}
            onDoubleClick={() => setRenaming(list.guid)}
            onContextMenu={(e) => {
              e.preventDefault();
              setListMenu({ list, at: { x: e.clientX, y: e.clientY, alignRight: false } });
            }}
            // A reminder dragged onto another list moves to it, its subtasks with it, as on the
            // Mac. Marked on the element itself while it is over it, as the rows mark a drop.
            onDragOver={(e) => {
              const moving = draggingRef.current ? items.find((x) => x.guid === draggingRef.current) : null;
              if (!moving || moving.listGuid === list.guid) return;
              e.preventDefault();
              e.dataTransfer.dropEffect = "move";
              e.currentTarget.classList.add("is-drop-target");
            }}
            onDragLeave={(e) => e.currentTarget.classList.remove("is-drop-target")}
            onDrop={(e) => {
              e.currentTarget.classList.remove("is-drop-target");
              const moving = draggingRef.current ? items.find((x) => x.guid === draggingRef.current) : null;
              if (!moving || moving.listGuid === list.guid) return;
              e.preventDefault();
              markDrop(null);
              draggingRef.current = null;
              setDragging(null);
              void moveToList(moving, list.guid);
            }}
          >
            <span className="rx-badge" style={{ background: colourOf(list) }}>
              <Glyph name={emblemGlyph(list.emblem)} />
            </span>
            <span className="source-name rx-list-name">{list.title}</span>
            <span className="count">{countOf(list.guid)}</span>
          </button>
          {/* One ⋯ over where the count sits, as Notes' and Contacts' rows have: three
              buttons there covered the list's own name. Right-click opens the same menu. */}
          <button
            className="icon-button small round row-more rx-list-more"
            aria-label={`Actions for ${list.title}`}
            aria-haspopup="menu"
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              setListMenu({ list, at: { x: r.right, y: r.bottom + 4, alignRight: true } });
            }}
          >
            <span aria-hidden="true">•••</span>
          </button>
        </>
      )}
    </div>
  );

  const tile = (
    key: string,
    title: string,
    glyph: GlyphName,
    colour: string,
    count: number,
    current: boolean,
    choose: () => void,
  ) => (
    <button
      key={key}
      className="rx-tile"
      aria-current={current}
      aria-label={`${title}, ${count} reminders`}
      style={{ "--tile": colour } as React.CSSProperties}
      onClick={() => {
        setQuery("");
        choose();
        setDetail(true);
      }}
    >
      <span className="rx-tile-glyph">
        <Glyph name={glyph} />
      </span>
      <span className="rx-tile-count">{count}</span>
      <span className="rx-tile-name">{title}</span>
    </button>
  );

  /** Where a reminder typed at the foot of the view is filed, when that is a choice. */
  // Completed has nothing to add to, as on the Mac: a reminder begins open.
  const canAdd = (!gathered || (selection.type === "tag" && REMINDER_DETAIL_WRITES)) && smartKind !== "completed";
  const addNow = useRef(add);
  addNow.current = add;
  const onAdd = useCallback((typed: string, said: QuickReminder | null) => addNow.current(typed, said), []);
  const addRow = targetListGuid && !searching && canAdd && (
    <AddReminder
      onAdd={onAdd}
      label={inList ? `Add a reminder to ${heading}` : `Add a reminder to ${targetList?.title ?? "a list"}`}
      // From a built-in view, where it lands is a real choice, so it is shown.
      lists={!inList && targetList && lists.length > 1 ? lists : null}
      target={targetListGuid}
      onChooseList={chooseList}
    />
  );

  const rowActions = useRef<RowActions>(null!);
  rowActions.current = {
    toggle, askRemove, rename, indent, outdent, flag, setNotes, setUrl, setDue, setEarly, setRepeat,
    tagAdd, tagRemove, setPriority, moveToList, moveToSection, openTag, setOpenGuid, reorder,
    foldSubtasks,
    next: (r) => {
      // Below it in a list, where its order is the list's; elsewhere, at the foot of the view.
      if (inList && !searching && !r.completedAt) setInsertAfter(r);
      else window.setTimeout(() => document.querySelector<HTMLInputElement>(".rx-add input")?.focus(), 0);
    },
    openMenu: (reminder, at) => setRowMenu({ reminder, at }),
    dragging: () => draggingRef.current,
    startDrag: (guid) => {
      draggingRef.current = guid;
      setDragging(guid);
    },
    endDrag: () => {
      draggingRef.current = null;
      setDragging(null);
      markDrop(null);
    },
    markDrop,
  };

  return (
    <div
      className={`rx${detail && !searching ? " is-detail" : ""}${searching ? " is-searching" : ""}`}
      style={{ "--accent": accent } as React.CSSProperties}
    >
      <aside className="rx-side" aria-label="Reminder lists">
        <SearchField
          ref={searchRef}
          className="rx-search"
          placeholder="Search"
          aria-label="Search reminders"
          value={query}
          onChange={setQuery}
          onKeyDown={(e) => {
            if (composing(e)) return;
            if (e.key === "Escape") setQuery("");
          }}
        />

        <div className="rx-tiles">
          {SMART.map((smart) =>
            tile(
              smart.kind,
              smart.title,
              smart.glyph,
              smart.colour,
              counts?.[smart.kind] ?? 0,
              !searching && picked.type === "smart" && picked.kind === smart.kind,
              () => setSelection({ type: "smart", kind: smart.kind }),
            ),
          )}
          {pinned.map((list) =>
            tile(
              list.guid,
              list.title,
              emblemGlyph(list.emblem),
              colourOf(list),
              countOf(list.guid),
              !searching && picked.type === "list" && picked.guid === list.guid,
              () => setSelection({ type: "list", guid: list.guid }),
            ),
          )}
        </div>

        <p className="side-heading rx-side-heading">My Lists</p>
        <div className="rx-lists">
          {topLevel.map((entry) =>
            entry.kind === "list" ? (
              listRow(entry.list, false)
            ) : (
              <div className="rx-group" key={entry.group.guid}>
                <button
                  className="source-row rx-group-row"
                  aria-expanded={!folded.has(entry.group.guid)}
                  onClick={() => toggleFolded(entry.group.guid)}
                >
                  <span className="rx-group-glyph">
                    <Glyph name="folder" />
                  </span>
                  <span className="source-name rx-list-name">{entry.group.title}</span>
                  <span className="rx-chevron">
                    <Glyph name="chevron" />
                  </span>
                </button>
                {!folded.has(entry.group.guid) &&
                  inGroup(entry.group.guid).map((list) => listRow(list, true))}
              </div>
            ),
          )}
          {/* Smart lists made by hand, after the lists, as the Mac shows them: a filter in its
              colour, with what it holds now. */}
          {customLists.map((smart) => {
            const title = smart.title ?? "Smart List";
            return (
              <div className="rx-list-row" key={smart.guid}>
                <button
                  className="source-row rx-list rx-smart-row"
                  aria-current={picked.type === "custom" && picked.guid === smart.guid}
                  onClick={() => {
                    setQuery("");
                    setSelection({ type: "custom", guid: smart.guid });
                    setDetail(true);
                  }}
                >
                  <span className="rx-badge" style={{ background: smartColour(smart) }}>
                    {smart.emblem ? <Glyph name={emblemGlyph(smart.emblem)} /> : <ListFilter aria-hidden="true" strokeWidth={2.25} />}
                  </span>
                  <span className="source-name rx-list-name">{title}</span>
                  <span className="count">{customCounts.get(smart.guid) ?? 0}</span>
                </button>
              </div>
            );
          })}
        </div>
        {loaded && lists.length === 0 && !arriving && <p className="side-heading">No lists yet</p>}

        {tags.length > 0 && (
          <>
            <p className="side-heading rx-side-heading">Tags</p>
            <div className="rx-tags" role="group" aria-label="Tags">
              {tags.map((t) => {
                const current = !searching && picked.type === "tag" && picked.name.toLowerCase() === t.name.toLowerCase();
                return (
                  <button
                    key={t.name}
                    className="rx-tag-chip"
                    data-tag={t.name}
                    aria-current={current}
                    aria-label={`#${t.name}, ${t.open} ${t.open === 1 ? "reminder" : "reminders"}`}
                    onClick={() => {
                      setQuery("");
                      setSelection({ type: "tag", name: t.name });
                      setDetail(true);
                    }}
                  >
                    #{t.name}
                  </button>
                );
              })}
            </div>
          </>
        )}

        {addingList ? (
          <form className="rx-new-list" onSubmit={createList}>
            <input
              className="field rx-rename"
              value={newList}
              onChange={(e) => setNewList(e.target.value)}
              placeholder="List name"
              aria-label="New list name"
              autoFocus
              onBlur={() => !newList.trim() && setAddingList(false)}
            />
          </form>
        ) : (
          <button className="side-add" onClick={() => setAddingList(true)}>
            <Glyph name="plus" />
            Add List
          </button>
        )}
        <AppFoot />
      </aside>

      <section className="rx-main" ref={mainRef}>
        <button className="compact-back" onClick={() => setDetail(false)}>
          <ChevronLeft size={18} strokeWidth={2.25} aria-hidden="true" />
          <span>Lists</span>
        </button>
        <header className="pane-head rx-head">
          <PaneTitle
            count={
              smartKind === "completed" && !searching
                ? `${items.length - outstanding} completed`
                : `${outstanding} ${outstanding === 1 ? "reminder" : "reminders"}`
            }
          >
            {searching ? `Results for “${query.trim()}”` : heading}
          </PaneTitle>
          {inList && (
            <button className="link" onClick={() => setShowDone((v) => !v)}>
              {showDone ? "Hide Completed" : "Show Completed"}
            </button>
          )}
        </header>
        <Unsent refused={outbox.refused} what="a change to a reminder" onResend={(r) => void outbox.resend(r)} onDiscard={(r) => void outbox.discard(r)} />

        {/* An account with no lists: a reminder cannot exist outside one, so the one thing
            that helps is offered instead of an add field with nowhere to go. */}
        <Arriving service="reminders" what="reminders" />
        {lists.length === 0 && loaded && !arriving && (
          <EmptyState
            className="rem-nolists"
            action={{ label: "Create a List", run: () => void makeFirstList(), disabled: syncing }}
          >
            Reminders live in lists. Make your first one to begin.
          </EmptyState>
        )}

        {unknownRules.length > 0 && selection.type === "custom" && !searching && (
          <p className="rx-capped" role="status">
            Shows what matches the rules read here. Your iPhone has the full list.
          </p>
        )}

        {heldBack > 0 && (
          <p className="rx-capped" role="status">
            Showing the first {MOST_AT_ONCE.toLocaleString()} — {heldBack.toLocaleString()} more
            are in this list.
          </p>
        )}

        {sections.map((section) => (
          <Fragment key={section.key}>
            {section.heading &&
              (inList && REMINDER_DETAIL_WRITES && namingSection === section.key ? (
                <input
                  className="field rx-section-name"
                  defaultValue={section.heading}
                  autoFocus
                  aria-label={`Rename section ${section.heading}`}
                  onBlur={(e) => {
                    setNamingSection(null);
                    const found = listSections.find((x) => x.guid === section.key);
                    if (found) void renameSection(found, e.target.value);
                  }}
                  onKeyDown={(e) => {
                    if (composing(e)) return;
                    if (e.key === "Enter") e.currentTarget.blur();
                    if (e.key === "Escape") {
                      e.currentTarget.value = section.heading ?? "";
                      e.currentTarget.blur();
                    }
                  }}
                />
              ) : (
                <h2
                  className={`rx-section${section.dated ? " is-quiet" : ""}${inList ? " is-list-section" : ""}`}
                  style={section.colour ? { color: section.colour } : undefined}
                  data-section={inList ? section.key : undefined}
                  onDoubleClick={
                    inList && REMINDER_DETAIL_WRITES && !section.key.startsWith(STAND_IN)
                      ? () => setNamingSection(section.key)
                      : undefined
                  }
                >
                  {section.heading}
                </h2>
              ))}
            <ul className="rx-rows">
              {section.rows.flatMap(({ reminder, depth }, i) => {
                const isOpen = openGuid === reminder.guid;
                // The next reminder's field goes below the one Return was pressed on, after its
                // subtasks — the same field throughout, keyed apart from the rows, so a row
                // becoming real under it (its stand-in replaced) never takes what is being typed.
                const anchored = (g: string) => insertAfter !== null && (g === insertAfter.guid || g === insertAfter.was);
                const below =
                  insertAfter !== null &&
                  (anchored(reminder.guid) || (depth > 0 && reminder.parentGuid !== null && anchored(reminder.parentGuid))) &&
                  !(section.rows[i + 1]?.reminder.parentGuid && anchored(section.rows[i + 1].reminder.parentGuid!));
                return [
                  <ReminderRow
                    key={reminder.guid}
                    reminder={reminder}
                    depth={depth}
                    dated={section.dated}
                    clock={reminder.dueAt === null || reminder.completedAt ? "" : `${today}:${reminder.dueAt * 1000 <= clock ? 1 : 0}`}
                    isOpen={isOpen}
                    marks={marks.get(reminder.guid)}
                    tags={isOpen && detailOf?.guid === reminder.guid ? detailOf.tags : (tagsOf.get(reminder.guid) ?? NO_TAGS)}
                    home={listById.get(reminder.listGuid)}
                    showListName={showListName}
                    justMoved={moved?.guid === reminder.guid ? moved.text : null}
                    waiting={outbox.waiting.has(reminder.guid)}
                    leaving={leaving.has(reminder.guid)}
                    settling={settling.has(reminder.guid)}
                    isDragging={dragging === reminder.guid}
                    canReorder={canReorder}
                    inList={inList}
                    act={rowActions}
                    focused={focusedBeforePress}
                    // What only the open row's details read: left out of every other row, so a
                    // change to any of it draws one row again, not a thousand.
                    detailOf={isOpen ? detailOf : null}
                    moreOf={isOpen ? moreOf : null}
                    lists={isOpen ? lists : NO_LISTS}
                    sections={isOpen ? listSections : NO_SECTIONS}
                    section={isOpen ? sectionOf(reminder) : null}
                    subtasks={isOpen ? items.filter((x) => x.parentGuid === reminder.guid).length : 0}
                    kids={depth === 0 ? (kidsOf.get(reminder.guid) ?? 0) : 0}
                    kidsHidden={hiddenSubtasks.has(reminder.guid)}
                  />,
                  ...(below && insertAfter
                    ? [
                        <NextReminder
                          key="rx-next"
                          subtask={Boolean(insertAfter.parentGuid)}
                          onAdd={(text, goOn) => {
                            const anchor = insertAfter;
                            if (text.trim()) void addBelow(text, anchor, goOn);
                            else setInsertAfter(null);
                          }}
                          onCancel={() => setInsertAfter(null)}
                        />,
                      ]
                    : []),
                ];
              })}
            </ul>
          </Fragment>
        ))}

        {addRow}

        {inList && REMINDER_DETAIL_WRITES && !searching && (
          namingSection === "new" ? (
            <input
              className="field rx-section-name"
              autoFocus
              placeholder="Section name"
              aria-label="New section name"
              onBlur={(e) => {
                setNamingSection(null);
                void createSection(e.target.value);
              }}
              onKeyDown={(e) => {
                if (composing(e)) return;
                if (e.key === "Enter") e.currentTarget.blur();
                if (e.key === "Escape") {
                  e.currentTarget.value = "";
                  e.currentTarget.blur();
                }
              }}
            />
          ) : (
            <button className="link rx-add-section" onClick={() => setNamingSection("new")}>
              Add Section
            </button>
          )
        )}

        {loaded && items.length === 0 && !error && lists.length > 0 && (
          <EmptyState className="rx-empty">
            {searching
              ? "No reminders match."
              : inList
                ? showDone
                  ? "Nothing in this list yet."
                  : "All done here."
                : smartKind === "completed"
                  ? "No Completed Reminders"
                  : "No Reminders"}
          </EmptyState>
        )}
      </section>
      {rowMenu && (
        <RowMenu
          at={rowMenu.at}
          onClose={() => setRowMenu(null)}
          entries={(() => {
            const r = rowMenu.reminder;
            return [
              { label: "Show Info", run: () => setOpenGuid(r.guid) },
              { label: r.flagged ? "Unflag" : "Flag", run: () => void flag(r) },
              { label: r.completedAt ? "Mark as Not Completed" : "Mark as Completed", run: () => void toggle(r) },
              null,
              ...(r.parentGuid
                ? [{ label: "Outdent", run: () => void outdent(r) }]
                : items.some((x) => x.parentGuid === r.guid)
                  ? []
                  : [{ label: "Indent", run: () => void indent(r) }]),
              // Only where there is somewhere else to go: an empty submenu is a dead end.
              ...(lists.some((l) => l.guid !== r.listGuid)
                ? [{
                    label: "Move to List",
                    submenu: lists
                      .filter((l) => l.guid !== r.listGuid)
                      .map((l) => ({ label: l.title, run: () => void moveToList(r, l.guid) })),
                  }]
                : []),
              ...(REMINDER_DETAIL_WRITES && inList && listSections.length > 0
                ? [{
                    label: "Move to Section",
                    submenu: [
                      ...(sectionOf(r) ? [{ label: "None", run: () => void moveToSection(r, null) }] : []),
                      ...listSections
                        .filter((x) => x.guid !== sectionOf(r) && !x.guid.startsWith(STAND_IN))
                        .map((x) => ({ label: x.title, run: () => void moveToSection(r, x.guid) })),
                    ],
                  }]
                : []),
              null,
              { label: "Delete", danger: true, run: () => void askRemove(r) },
            ];
          })()}
        />
      )}
      {listMenu && (
        <RowMenu
          at={listMenu.at}
          onClose={() => setListMenu(null)}
          entries={[
            { label: "List Details", run: () => setInfoFor(listMenu.list) },
            { label: "Rename List", run: () => setRenaming(listMenu.list.guid) },
            null,
            {
              label: "Delete List",
              danger: true,
              run: () => {
                const list = listMenu.list;
                // Deleting a list takes its reminders, so ask before doing it.
                void confirm({
                  title: `Delete \u201c${list.title}\u201d?`,
                  body: "Every reminder in it is deleted too, on all your devices.",
                  action: "Delete",
                  danger: true,
                  symbol: "trash",
                }).then((sure) => {
                  if (sure) void listWrite(() => api.reminderListDelete(list.guid));
                });
              },
            },
          ]}
        />
      )}
      {confirmSheet}
      {infoFor && (
        <ListInfo
          list={infoFor}
          onClose={() => setInfoFor(null)}
          onDone={(title, colour) => {
            const list = infoFor;
            setInfoFor(null);
            // Shown at once; Apple's answer confirms or puts it back.
            setLists((ls) => ls.map((l) => (l.guid === list.guid
              ? { ...l, title: title || l.title, colour: colour ? LIST_COLOURS.find((c) => c.key === colour)!.hex : l.colour }
              : l)));
            void listWrite(async () => {
              if (title && title !== list.title) await api.reminderListRename(list.guid, title);
              if (colour) await api.reminderListColour(list.guid, colour);
            });
          }}
        />
      )}
    </div>
  );
}

/** The actions a row takes, kept in one ref the list fills on every draw: the row reads them
 *  when it acts, so handing them down never changes a row's props. */
type RowActions = {
  toggle: (r: Reminder) => void;
  askRemove: (r: Reminder) => Promise<void>;
  rename: (r: Reminder, title: string) => Promise<unknown>;
  indent: (r: Reminder) => Promise<unknown>;
  outdent: (r: Reminder) => Promise<unknown>;
  flag: (r: Reminder) => Promise<unknown>;
  setNotes: (r: Reminder, notes: string) => Promise<unknown> | undefined;
  setUrl: (r: Reminder, url: string | null) => Promise<unknown>;
  setDue: (r: Reminder, day: string, time: string | null) => Promise<unknown>;
  setEarly: (r: Reminder, alert: [number, number] | null) => Promise<unknown>;
  setRepeat: (r: Reminder, repeat: { frequency: number; interval: number } | null, endAt: number | null, days?: number[]) => Promise<unknown>;
  tagAdd: (r: Reminder, name: string) => Promise<unknown>;
  tagRemove: (r: Reminder, name: string) => Promise<unknown>;
  setPriority: (r: Reminder, priority: number) => Promise<unknown>;
  moveToList: (r: Reminder, listGuid: string) => Promise<unknown>;
  moveToSection: (r: Reminder, section: string | null) => Promise<unknown>;
  openTag: (name: string) => void;
  setOpenGuid: (guid: string | null) => void;
  openMenu: (r: Reminder, at: { x: number; y: number; alignRight: boolean }) => void;
  reorder: (guid: string, before: string) => Promise<unknown>;
  /** Fold a reminder's subtasks away under it, or open them again. */
  foldSubtasks: (guid: string) => void;
  /** Return on a reminder's title: begin the next one, below it. */
  next: (r: Reminder) => void;
  /** The reminder being dragged now, read at the moment it is asked. */
  dragging: () => string | null;
  startDrag: (guid: string) => void;
  endDrag: () => void;
  /** Mark the row a drop would land before, and only that one. */
  markDrop: (row: HTMLElement) => void;
};

/** Marks with Urgent read from Apple laid in: a row that had none gains them; the map is
 *  replaced only when a row's Urgent changes, so nothing else is drawn again. */
function withUrgent(was: Map<string, ReminderMarks>, read: UrgentRead[]): Map<string, ReminderMarks> {
  let next: Map<string, ReminderMarks> | null = null;
  for (const { guid, urgent } of read) {
    const at = was.get(guid);
    if ((at?.urgent ?? false) === urgent) continue;
    next ??= new Map(was);
    next.set(guid, { ...(at ?? { guid, daysOfWeek: [], urgent: false, location: null, messaging: null, url: null }), urgent });
  }
  return next ?? was;
}

const NO_TAGS: string[] = [];
const NO_LISTS: ReminderList[] = [];
const NO_SECTIONS: ReminderSection[] = [];

type ReminderRowProps = {
  reminder: Reminder;
  depth: number;
  /** Under a heading that already says the day, so the row says only the time. */
  dated: boolean;
  /** Moves when this row's due label or its red can change: at midnight, and as it falls due. */
  clock: string;
  isOpen: boolean;
  marks: ReminderMarks | undefined;
  tags: string[];
  home: ReminderList | undefined;
  showListName: boolean;
  justMoved: string | null;
  waiting: boolean;
  leaving: boolean;
  settling: boolean;
  isDragging: boolean;
  canReorder: boolean;
  inList: boolean;
  act: React.MutableRefObject<RowActions>;
  focused: React.MutableRefObject<Element | null>;
  detailOf: ReminderDetail | null;
  moreOf: ReminderMore | null;
  lists: ReminderList[];
  sections: ReminderSection[];
  section: string | null;
  subtasks: number;
  /** How many subtasks it has in view, and whether they are folded away under it. */
  kids: number;
  kidsHidden: boolean;
};

/**
 * New Reminder at the foot of the view, keeping what is being typed to itself. Kept in the
 * screen, each key drew the whole screen again: every section worked out anew and every row's
 * props compared, ten thousand of them in a big list (desktop audit H7). Here a key draws this
 * field and its preview, and the list is drawn when a reminder is added.
 */
const AddReminder = memo(function AddReminder({
  onAdd, label, lists, target, onChooseList,
}: {
  onAdd: (typed: string, said: QuickReminder | null) => Promise<boolean>;
  label: string;
  /** The lists to choose from, when where it lands is a choice; otherwise null. */
  lists: ReminderList[] | null;
  target: string;
  onChooseList: (guid: string) => void;
}) {
  const [draft, setDraft] = useState("");
  /** The line is added exactly as typed, its dates and marks left in the title (the preview's
   *  Add as Typed). Cleared once the line changes or is added. */
  const [literal, setLiteral] = useState(false);
  const quick = useMemo<QuickReminder | null>(() => {
    if (literal || !draft.trim()) return null;
    const q = parseReminder(draft);
    return understood(q) ? q : null;
  }, [draft, literal]);
  const options = useMemo(() => lists?.map((l) => ({ value: l.guid, label: l.title })) ?? [], [lists]);
  return (
    <form
      className="rx-add"
      onSubmit={(e) => {
        e.preventDefault();
        const typed = draft.trim();
        if (!typed) return;
        // What the line said beyond its title, as the preview showed it.
        const said = quick;
        setLiteral(false);
        // The field is ready for the next the moment Return is pressed; a line that could not
        // be added comes back, unless something new was typed meanwhile.
        setDraft("");
        void onAdd(typed, said).then((added) => {
          if (!added) setDraft((d) => d || typed);
        });
      }}
    >
      <span className="rx-tick ghost" aria-hidden="true">
        <Glyph name="plus" />
      </span>
      <input
        value={draft}
        onChange={(e) => {
          setDraft(e.target.value);
          setLiteral(false);
        }}
        placeholder="New Reminder"
        aria-label={label}
      />
      {/* From a built-in view, where it lands is a real choice, so it is shown — but only
          once something is being typed, not as a permanent control on the screen. */}
      {lists && draft.trim() && (
        <Picker
          className="rx-add-target"
          label="List to add to"
          value={target}
          onChange={onChooseList}
          options={options}
        />
      )}
      {quick && (
        <div className="rx-add-understood" role="status">
          <span className="rx-add-reading">
            {[
              quick.dueAt !== null && dueLabel(quick.dueAt, false, quick.allDay).text,
              quick.repeat && repeatWords(quick.repeat),
              quick.priority === 1 ? "High priority" : quick.priority === 5 ? "Medium priority" : quick.priority === 9 ? "Low priority" : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </span>
          <button type="button" className="rx-add-literal" onClick={() => setLiteral(true)}>
            Add as Typed
          </button>
        </div>
      )}
    </form>
  );
});

/**
 * One reminder's row, drawn again only when something it shows changes. Inline, every row of
 * up to a thousand was rebuilt on each key typed into New Reminder or the search, each tick and
 * each row the pointer crossed while dragging.
 */
const ReminderRow = memo(function ReminderRow({
  reminder, depth, dated, isOpen, marks: rowMarks, tags: rowTags, home, showListName, justMoved, waiting,
  leaving, settling, isDragging, canReorder, inList, act, focused, detailOf, moreOf, lists, sections, section, subtasks,
  kids, kidsHidden,
}: ReminderRowProps) {
  const due = reminder.dueAt === null ? null : dueLabel(reminder.dueAt, dated, reminder.allDay);
  const repeat = repeatLabel(reminder);
  const mark = priorityMark(reminder.priority);
  // Kept in another zone, the phone adds that zone's clock: "8:30 PM (11:00 AM GMT-4)".
  const elsewhere =
    due && reminder.dueAt !== null && !reminder.allDay && reminder.timezone && reminder.timezone !== LOCAL_ZONE
      ? zonedClock(reminder.timezone, reminder.dueAt)
      : null;
  const chips = rowMarks && (rowMarks.location || rowMarks.messaging || rowMarks.url) ? rowMarks : null;
  // The title field is the reminder's for its life, and takes a new title from a sync only
  // while nobody is in it. Keyed on the title, a phone's edit landing while this one was being
  // typed made a new field under the cursor and the words typed were gone (desktop audit M3).
  const titleRef = useRef<HTMLInputElement | null>(null);
  /** The title the field was last given, to tell typing from a field only visited. */
  const titleShown = useRef(reminder.title);
  useEffect(() => {
    const field = titleRef.current;
    if (!field || document.activeElement === field) return;
    field.value = reminder.title;
    titleShown.current = reminder.title;
  }, [reminder.title]);
  return (
    <li
      className={`rx-row${reminder.completedAt ? " is-done" : ""}${leaving ? " is-leaving" : ""}${
        depth > 0 ? " is-subtask" : ""
      }${isOpen ? " is-open" : ""}${isDragging ? " is-dragging" : ""}`}
      // Drag to reorder, in a list, as Reminders does — open ones among themselves.
      draggable={canReorder && depth === 0 && !reminder.completedAt}
      onDragStart={(e) => {
        if (!canReorder) return;
        e.dataTransfer.effectAllowed = "move";
        e.dataTransfer.setData("text/plain", reminder.guid);
        // Its name under the pointer, not the whole row (lib/dragPicture).
        carryLabel(e, reminder.title);
        act.current.startDrag(reminder.guid);
      }}
      onDragEnd={() => act.current.endDrag()}
      // The row itself takes the keyboard, as on icloud.com and the Mac: arrows go
      // from row to row, Space ticks it, Return edits its title, Delete deletes it
      // once confirmed — a reminder deleted here cannot be brought back.
      tabIndex={-1}
      data-guid={reminder.guid}
      // The list's colour for everything on the row drawn in it: the circle, the
      // priority marks.
      style={{ "--tick": colourOf(home) } as React.CSSProperties}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        const rows = [...document.querySelectorAll<HTMLElement>(".rx-rows .rx-row")];
        const at = rows.indexOf(e.currentTarget);
        const go = (i: number) => rows[Math.max(0, Math.min(rows.length - 1, i))]?.focus();
        if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          go(at + (e.key === "ArrowDown" ? 1 : -1));
        } else if (e.key === "Home" || e.key === "End") {
          e.preventDefault();
          go(e.key === "Home" ? 0 : rows.length - 1);
        } else if (e.key === " ") {
          e.preventDefault();
          act.current.toggle(reminder);
        } else if (e.key === "Enter") {
          e.preventDefault();
          const title = e.currentTarget.querySelector<HTMLInputElement>(".rx-title");
          title?.focus();
          title?.select();
        } else if (e.key === "Delete" || e.key === "Backspace") {
          e.preventDefault();
          void act.current.askRemove(reminder);
        }
      }}
      // Right-click, as on the Mac: what can be done to this reminder, in one menu.
      onMouseDownCapture={() => {
        // What had focus before this press: the press itself focuses a title it
        // lands on, which is not the same as someone already typing in it.
        focused.current = document.activeElement;
      }}
      onContextMenu={(e) => {
        // Text being edited keeps its own menu; anywhere else on the row, the
        // reminder's — its title included, until someone is typing in it.
        const field = (e.target as HTMLElement).closest("input, textarea, [contenteditable=true]");
        if (field && field === focused.current) return;
        (document.activeElement as HTMLElement | null)?.blur?.();
        e.preventDefault();
        act.current.openMenu(reminder, { x: e.clientX, y: e.clientY, alignRight: false });
      }}
      onDragOver={(e) => {
        const dragging = act.current.dragging();
        if (!dragging || dragging === reminder.guid || depth > 0 || reminder.completedAt) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        // Marked on the row itself, not in state: a state change here drew the whole list
        // again each time the pointer crossed into another row.
        act.current.markDrop(e.currentTarget);
      }}
      onDrop={(e) => {
        e.preventDefault();
        const moving = act.current.dragging();
        act.current.endDrag();
        if (moving && moving !== reminder.guid) void act.current.reorder(moving, reminder.guid);
      }}
    >
      <button
        className="rx-tick"
        role="checkbox"
        aria-checked={Boolean(reminder.completedAt) || settling}
        aria-label={reminder.title}
        style={{ "--tick": colourOf(home) } as React.CSSProperties}
        // The fill fades in on the press itself, and only then (kept as state, it came
        // in again every time Reminders came back into view): opacity alone, on a
        // twenty-pixel circle, drawn by the page (lib/fade) — never the compositor.
        onClick={(e) => {
          const fill = e.currentTarget.querySelector<HTMLElement>(".rx-tick-fill");
          const filling = !reminder.completedAt;
          act.current.toggle(reminder);
          if (filling) requestAnimationFrame(() => fadeIn(fill, { from: 0.25, duration: 140 }));
        }}
      >
        <span className="rx-tick-fill" aria-hidden="true" />
      </button>

      <div className="rx-text">
        <div className="rx-title-row">
          {mark && (
            <span
              className="rx-priority"
              role="img"
              aria-label={`${
                PRIORITIES.find((p) => p.value === reminder.priority)?.label
              } priority`}
            >
              {mark}
            </span>
          )}
          <input
            className="rx-title"
            ref={titleRef}
            defaultValue={reminder.title}
            aria-label={`Rename ${reminder.title}`}
            onBlur={(e) => {
              const typed = e.target.value;
              // Visited, nothing typed: it shows the title as it is now, which a sync may have
              // changed meanwhile, rather than send the old one back over it.
              if (typed === titleShown.current) {
                e.target.value = reminder.title;
                titleShown.current = reminder.title;
                return;
              }
              titleShown.current = typed;
              void act.current.rename(reminder, typed);
            }}
            onKeyDown={(e) => {
              if (composing(e)) return;
              // As in Reminders on a Mac: Return finishes it and starts the next,
              // Tab tucks it under the one above, Shift-Tab brings it back out.
              if (e.key === "Enter") {
                e.preventDefault();
                e.currentTarget.blur();
                act.current.next(reminder);
              } else if (e.key === "Escape") {
                // Back to the row, which then takes the arrows and Space.
                e.currentTarget.value = reminder.title;
                e.currentTarget.closest<HTMLElement>(".rx-row")?.focus();
              } else if (inList && (e.key === "Tab" || ((e.ctrlKey || e.metaKey) && (e.key === "]" || e.key === "[")))) {
                // Tab and Shift-Tab, or Command-] and Command-[ as the Mac's menu has them.
                e.preventDefault();
                const typed = e.currentTarget.value;
                const out = e.key === "[" || (e.key === "Tab" && e.shiftKey);
                const caret = e.currentTarget.selectionStart ?? typed.length;
                const renamed = act.current.rename(reminder, typed);
                const moved = out ? (reminder.parentGuid ? act.current.outdent(reminder) : null) : (reminder.parentGuid ? null : act.current.indent(reminder));
                // The cursor stays in the title, as on the Mac: the rename redraws the field (and
                // the move the row), which had dropped it.
                const refocus = () => {
                  const active = document.activeElement;
                  if (active && active !== document.body && !active.closest(".rx-row")) return;
                  const field = document.querySelector<HTMLInputElement>(`.rx-row[data-guid="${CSS.escape(reminder.guid)}"] .rx-title`);
                  if (!field || field === active) return;
                  field.focus();
                  field.setSelectionRange(caret, caret);
                };
                window.setTimeout(refocus, 0);
                void Promise.all([renamed, moved]).then(refocus, refocus);
              } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !e.shiftKey && !e.altKey) {
                // Up and Down walk the list, keeping where the caret was.
                const all = [...document.querySelectorAll<HTMLInputElement>(".rx-row .rx-title")];
                const next = all[all.indexOf(e.currentTarget) + (e.key === "ArrowDown" ? 1 : -1)];
                if (next) {
                  e.preventDefault();
                  const caret = e.currentTarget.selectionStart ?? 0;
                  next.focus();
                  next.setSelectionRange(Math.min(caret, next.value.length), Math.min(caret, next.value.length));
                }
              }
            }}
          />
        </div>
        {reminder.notes && !isOpen && (
          <span className="rx-notes">{reminder.notes}</span>
        )}
        <Waiting shown={waiting} />
        {(due || repeat || justMoved || (showListName && home) || rowTags.length > 0 || rowMarks?.urgent) && (
          <span className="rx-meta">
            {justMoved ? (
              <span className="rx-moved" role="status">
                Moved to {justMoved}
              </span>
            ) : (
              <>
                {showListName && (
                  <span className="rx-from">{home?.title ?? "Reminders"}</span>
                )}
                {due && (
                  <span className={`rx-due${due.overdue ? " overdue" : ""}`}>
                    {elsewhere ? `${due.text} (${elsewhere})` : due.text}
                  </span>
                )}
                {repeat && (
                  <span className="rx-repeat" title={repeat}>
                    <span aria-hidden="true">↻</span> {repeatWord(reminder, rowMarks?.daysOfWeek)}
                    <span className="visually-hidden">{repeat}</span>
                  </span>
                )}
                {rowMarks?.urgent && (
                  <span className="rx-urgent" role="img" aria-label="Urgent" title="Urgent">
                    <AlarmClock aria-hidden="true" size={13} strokeWidth={2} />
                  </span>
                )}
                {rowTags.map((name) => (
                  <button
                    key={name}
                    type="button"
                    className="rx-tag"
                    tabIndex={-1}
                    aria-label={`Show reminders tagged ${name}`}
                    onClick={() => act.current.openTag(name)}
                  >
                    #{name}
                  </button>
                ))}
              </>
            )}
          </span>
        )}
        {chips && !justMoved && (
          <span className="rx-chips">
            {chips.location && (
              <span className="rx-chip">
                <MapPin aria-hidden="true" size={12} strokeWidth={2.2} className="rx-chip-place" />
                {chips.location.proximity && <b>{chips.location.proximity === "arriving" ? "Arriving:" : "Leaving:"}</b>}
                {chips.location.title ?? "A place"}
              </span>
            )}
            {chips.messaging && (
              <span className="rx-chip">
                <MessageCircle aria-hidden="true" size={12} strokeWidth={2.2} />
                <b>Messaging:</b>
                {chips.messaging}
              </span>
            )}
            {chips.url && outsideHref(chips.url) && (
              <a className="rx-chip rx-chip-link" href={outsideHref(chips.url)!} target="_blank" rel="noreferrer" tabIndex={-1} title={chips.url}>
                <Link2 aria-hidden="true" size={12} strokeWidth={2.2} />
                {hostOf(chips.url)}
              </a>
            )}
          </span>
        )}

        {isOpen && (
          <div className="rx-detail" role="group" aria-label={`Info for ${reminder.title}`}>
            <label className="rx-field rx-field-notes">
              <span>Notes</span>
              <textarea
                className="field"
                defaultValue={reminder.notes ?? ""}
                key={`n${reminder.guid}`}
                rows={2}
                placeholder="Add Note"
                onBlur={(e) => void act.current.setNotes(reminder, e.target.value)}
              />
            </label>
            <UrlField
              reminder={reminder}
              url={detailOf?.guid === reminder.guid ? detailOf.url : null}
              onSave={(url) => void act.current.setUrl(reminder, url)}
            />
            <div className="rx-detail-rule" aria-hidden="true" />
            <DueField reminder={reminder} onChange={(day, time) => void act.current.setDue(reminder, day, time)} />
            {moreOf?.guid === reminder.guid && <ZoneAndUrgent reminder={reminder} more={moreOf} />}
            <EarlyField
              reminder={reminder}
              alerts={detailOf?.guid === reminder.guid ? detailOf.earlyAlerts : []}
              onChange={(alert) => void act.current.setEarly(reminder, alert)}
            />
            <RepeatField
              reminder={reminder}
              rule={moreOf?.guid === reminder.guid ? moreOf.repeat : null}
              onChange={(repeat, endAt, days) => void act.current.setRepeat(reminder, repeat, endAt, days)}
            />
            <div className="rx-detail-rule" aria-hidden="true" />
            <TagsField
              reminder={reminder}
              tags={rowTags}
              onOpen={(name) => act.current.openTag(name)}
              onAdd={(name) => void act.current.tagAdd(reminder, name)}
              onRemove={(name) => void act.current.tagRemove(reminder, name)}
            />
            <div className="rx-field">
              <span>Subtasks</span>
              <span className="rx-value">{subtasks}</span>
            </div>
            <div className="rx-field">
              <span>Flag</span>
              <label className="rx-switch">
                <input
                  type="checkbox"
                  role="switch"
                  aria-label={`Flag for ${reminder.title}`}
                  checked={reminder.flagged}
                  onChange={() => void act.current.flag(reminder)}
                />
                <span className="switch" aria-hidden="true" />
              </label>
            </div>
            <div className="rx-field">
              <span>Priority</span>
              <Picker
                label={`Priority for ${reminder.title}`}
                value={String(reminder.priority)}
                onChange={(v) => void act.current.setPriority(reminder, Number(v))}
                options={PRIORITIES.map((p) => ({
                  value: String(p.value),
                  label: p.label,
                }))}
              />
            </div>
            {moreOf?.guid === reminder.guid && <PlacesAndPeople more={moreOf} />}
            {detailOf?.guid === reminder.guid && detailOf.images.length > 0 && (
              <div className="rx-images" role="list" aria-label="Pictures">
                {detailOf.images.map((id) => (
                  <img
                    key={id}
                    role="listitem"
                    className="rx-image"
                    src={reminderImageSrc(id)}
                    alt="Attached picture"
                    loading="lazy"
                    decoding="async"
                    draggable={false}
                  />
                ))}
              </div>
            )}
            {(detailOf?.guid === reminder.guid ? detailOf.otherAttachments - detailOf.images.length : 0) > 0 && (
              <div className="rx-field">
                <span>Attachments</span>
                <span className="rx-value">
                  {detailOf!.otherAttachments - detailOf!.images.length === 1
                    ? "1 file"
                    : `${detailOf!.otherAttachments - detailOf!.images.length} files`}
                </span>
              </div>
            )}
            <div className="rx-detail-rule" aria-hidden="true" />
            <div className="rx-field">
              <span>List</span>
              <Picker
                label={`List for ${reminder.title}`}
                value={reminder.listGuid}
                onChange={(guid) => void act.current.moveToList(reminder, guid)}
                options={lists.map((l) => ({ value: l.guid, label: l.title }))}
              />
            </div>
            {inList && sections.length > 0 && (
              <div className="rx-field">
                <span>Section</span>
                {REMINDER_DETAIL_WRITES ? (
                  <Picker
                    label={`Section for ${reminder.title}`}
                    value={section ?? ""}
                    onChange={(guid) => void act.current.moveToSection(reminder, guid || null)}
                    options={[
                      { value: "", label: "None" },
                      ...sections.map((x) => ({ value: x.guid, label: x.title })),
                    ]}
                  />
                ) : (
                  <span className="rx-value">
                    {sections.find((x) => x.guid === section)?.title ?? "None"}
                  </span>
                )}
              </div>
            )}
            <div className="rx-detail-actions">
              {depth > 0 ? (
                <button
                  className="link"
                  onClick={() => void act.current.outdent(reminder)}
                >
                  Move Out of Subtasks
                </button>
              ) : subtasks > 0 ? null : (
                <button
                  className="link"
                  onClick={() => void act.current.indent(reminder)}
                >
                  Make Subtask
                </button>
              )}
              <button
                className="link danger"
                aria-label={`Delete ${reminder.title}`}
                
                onClick={() => void act.current.askRemove(reminder)}
              >
                Delete
              </button>
            </div>
          </div>
        )}
      </div>

      <span className="rx-trailing">
        {/* A reminder with subtasks shows they are there, and folds them away under it. */}
        {kids > 0 && (
          <button
            className={`icon-button small rx-kids${kidsHidden ? "" : " is-open"}`}
            aria-expanded={!kidsHidden}
            aria-label={`${kidsHidden ? "Show" : "Hide"} ${kids === 1 ? "subtask" : `${kids} subtasks`} of ${reminder.title}`}
            title={kidsHidden ? `${kids === 1 ? "1 subtask" : `${kids} subtasks`}` : undefined}
            onClick={() => act.current.foldSubtasks(reminder.guid)}
          >
            {kidsHidden && <span className="rx-kids-count">{kids}</span>}
            <ChevronRight size={14} strokeWidth={2.25} aria-hidden="true" />
          </button>
        )}
        <button
          className="icon-button small rx-icon-button"
          aria-label={
            isOpen ? `Close details for ${reminder.title}` : `Details for ${reminder.title}`
          }
          aria-expanded={isOpen}
          onClick={() => act.current.setOpenGuid(isOpen ? null : reminder.guid)}
        >
          <Glyph name="info" />
        </button>
        <button
          className={`icon-button small is-glyph-state rx-icon-button rx-flag${reminder.flagged ? " on" : ""}`}
          aria-label={
            reminder.flagged ? `Unflag ${reminder.title}` : `Flag ${reminder.title}`
          }
          aria-pressed={reminder.flagged}
          
          onClick={() => void act.current.flag(reminder)}
        >
          <Glyph name="flagged" />
        </button>
      </span>
    </li>
  );
});

/**
 * The next reminder, being typed right below the one Return was pressed on. Return adds it and
 * goes on to another; Return or Escape on an empty one ends there, and leaving it with words
 * in it adds them.
 */
function NextReminder({ subtask, onAdd, onCancel }: { subtask: boolean; onAdd: (text: string, goOn: boolean) => void; onCancel: () => void }) {
  const done = useRef(false);
  return (
    <li className={`rx-row rx-next${subtask ? " is-subtask" : ""}`}>
      <span className="rx-tick" aria-hidden="true" />
      <div className="rx-text">
        <div className="rx-title-row">
          <input
            className="rx-title"
            autoFocus
            aria-label="New reminder"
            onKeyDown={(e) => {
              if (composing(e)) return;
              if (e.key === "Enter") {
                e.preventDefault();
                const typed = e.currentTarget.value;
                // Added, and the same field goes on below it, empty, for the next.
                if (typed.trim()) e.currentTarget.value = "";
                else done.current = true;
                onAdd(typed, true);
              } else if (e.key === "Escape") {
                e.preventDefault();
                done.current = true;
                onCancel();
              }
            }}
            onBlur={(e) => {
              const field = e.currentTarget;
              // Moved down past the row just added, the field can lose the focus for a moment and
              // have it put back: only a focus that stays elsewhere is leaving it.
              window.setTimeout(() => {
                if (done.current || document.activeElement === field || !field.isConnected) return;
                done.current = true;
                onAdd(field.value, false);
              }, 0);
            }}
          />
        </div>
      </div>
    </li>
  );
}

/** Apple's twelve list colours, in its picker's order, keyed as CloudKit names them. */
const LIST_COLOURS = [
  { key: "red", hex: "#FF3B30" }, { key: "orange", hex: "#FF9500" }, { key: "yellow", hex: "#FFCC00" },
  { key: "green", hex: "#34C759" }, { key: "lightBlue", hex: "#51AAF2" }, { key: "blue", hex: "#007AFF" },
  { key: "indigo", hex: "#5856D6" }, { key: "pink", hex: "#EA426A" }, { key: "purple", hex: "#C077DB" },
  { key: "brown", hex: "#A2845E" }, { key: "gray", hex: "#5B6770" }, { key: "rose", hex: "#D9A69F" },
] as const;

/** List Details: the list's mark, its name, and the twelve colours iCloud keeps for lists. */
function ListInfo({ list, onClose, onDone }: {
  list: ReminderList;
  onClose: () => void;
  onDone: (title: string, colour: string | null) => void;
}) {
  const [title, setTitle] = useState(list.title);
  const current = LIST_COLOURS.find((c) => c.hex.toLowerCase() === (list.colour ?? "").toLowerCase())?.key ?? null;
  const [colour, setColour] = useState<string | null>(current);
  const shown = LIST_COLOURS.find((c) => c.key === colour)?.hex ?? list.colour ?? "#0068D6";
  const sheet = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheet);
  const live = useActiveRef();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => live.current && e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  const done = () => onDone(title.trim(), colour !== current ? colour : null);
  return (
    <Portal>
    <div className="modal-veil" onClick={onClose}>
      <div ref={sheet} className="modal-sheet rx-info" role="dialog" aria-modal="true" aria-label={`\u201c${list.title}\u201d Info`} onClick={(e) => e.stopPropagation()}>
        <h2>“{list.title}” Info</h2>
        <span className="rx-info-mark" style={{ background: shown }} aria-hidden="true">
          <Glyph name="list" />
        </span>
        <input
          className="field rx-info-name"
          value={title}
          aria-label="List name"
          // The list's colour, deepened toward the text colour so the name reads at 4.5:1.
          style={{ color: `color-mix(in srgb, ${shown} 55%, var(--text))` }}
          autoFocus
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => !composing(e) && e.key === "Enter" && done()}
        />
        <div className="rx-info-colours" role="radiogroup" aria-label="Colour">
          {LIST_COLOURS.map((c) => (
            <button
              key={c.key}
              role="radio"
              aria-checked={colour === c.key}
              aria-label={c.key}
              className={`rx-info-swatch${colour === c.key ? " is-on" : ""}`}
              style={{ background: c.hex }}
              onClick={() => setColour(c.key)}
            />
          ))}
        </div>
        <div className="sheet-actions rx-info-actions">
          <button className="quiet" onClick={onClose}>Cancel</button>
          <button className="primary" onClick={done} disabled={!title.trim()}>Done</button>
        </div>
      </div>
    </div>
    </Portal>
  );
}

/**
 * The phone's date controls: a date, and a Time switch. Off, the reminder is due on the
 * day — all day, no alert; on, at the time chosen. Each change is saved when it is made.
 */
function DueField({ reminder, onChange }: { reminder: Reminder; onChange: (day: string, time: string | null) => void }) {
  // Read on the clock of the reminder's own zone, where it has one.
  const w = reminder.dueAt === null ? null : reminder.allDay ? localWall(reminder.dueAt) : wallIn(zoneOf(reminder), reminder.dueAt);
  const pad = (n: number) => String(n).padStart(2, "0");
  const day = w ? `${w.y}-${pad(w.m)}-${pad(w.d)}` : "";
  const time = w && !reminder.allDay ? `${pad(w.h)}:${pad(w.min)}` : null;
  return (
    <div className="rx-field">
      <span>Date</span>
      <div className="rx-due">
        <input
          type="date"
          className="field rx-due-day"
          aria-label={`Date for ${reminder.title}`}
          defaultValue={day}
          key={`d${reminder.dueAt}`}
          // A new date keeps the time already set; a first date is a day, as on the phone.
          onBlur={(e) => e.target.value !== day && onChange(e.target.value, e.target.value ? time : null)}
        />
        <label className="rx-switch">
          <input
            type="checkbox"
            role="switch"
            aria-label={`Time for ${reminder.title}`}
            checked={time !== null}
            disabled={!day}
            onChange={(e) => onChange(day, e.target.checked ? "09:00" : null)}
          />
          <span className="switch" aria-hidden="true" />
          <span>Time</span>
        </label>
        {time !== null && w && (
          <TimeField
            className="field"
            label={`Time of day for ${reminder.title}`}
            hours={w.h}
            minutes={w.min}
            onChange={(h, m) => onChange(day, `${pad(h)}:${pad(m)}`)}
          />
        )}
      </div>
    </div>
  );
}

/**
 * The reminder's link: opened in the browser with a click. Typed in or cleared once Apple takes
 * the change (REMINDER_DETAIL_WRITES); until then shown only where there is one.
 */
function UrlField({ reminder, url, onSave }: { reminder: Reminder; url: string | null; onSave: (url: string | null) => void }) {
  if (!REMINDER_DETAIL_WRITES) {
    if (!url) return null;
    // A link of a kind Skykeep does not open (lib/links.ts) is shown, never followed.
    const out = outsideHref(url);
    return (
      <div className="rx-field">
        <span>URL</span>
        {out ? (
          <a className="rx-url" href={out} target="_blank" rel="noreferrer" title={url}>
            {url.replace(/^https?:\/\//, "").replace(/\/$/, "")}
          </a>
        ) : (
          <span className="rx-url" title={refusedLink()}>{url}</span>
        )}
      </div>
    );
  }
  return (
    <label className="rx-field rx-field-url">
      <span>URL</span>
      <input
        className="field"
        type="url"
        key={`${reminder.guid}-${url ?? ""}`}
        defaultValue={url ?? ""}
        placeholder="Add URL"
        aria-label={`URL for ${reminder.title}`}
        onBlur={(e) => {
          const typed = e.target.value.trim();
          if (typed !== (url ?? "")) onSave(typed || null);
        }}
        onKeyDown={(e) => {
          if (composing(e)) return;
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") {
            e.currentTarget.value = url ?? "";
            e.currentTarget.blur();
          }
        }}
      />
      {url && /^(https?:|mailto:)/i.test(url) && outsideHref(url) && (
        <a className="rx-url rx-url-open" href={outsideHref(url)!} target="_blank" rel="noreferrer" title={url}>
          Open
        </a>
      )}
    </label>
  );
}

/** "30 minutes early": a second alert before the due one. Shown once there is a date. */
function EarlyField({ reminder, alerts, onChange }: {
  reminder: Reminder;
  alerts: { count: number; unit: number }[];
  onChange: (alert: [number, number] | null) => void;
}) {
  const said = alerts.map((a) => earlyText(a.count, a.unit)).join(", ");
  if (!REMINDER_DETAIL_WRITES) {
    if (!alerts.length) return null;
    return (
      <div className="rx-field">
        <span>Early Reminder</span>
        <span className="rx-value">{said}</span>
      </div>
    );
  }
  if (reminder.dueAt === null && !alerts.length) return null;
  const now = alerts[0] ? `${alerts[0].count}:${alerts[0].unit}` : "none";
  const known = EARLY_CHOICES.some(([c, u]) => `${c}:${u}` === now);
  return (
    <div className="rx-field">
      <span>Early Reminder</span>
      <Picker
        label={`Early reminder for ${reminder.title}`}
        value={now}
        onChange={(v) => {
          if (v === now) return;
          const [c, u] = v.split(":").map(Number);
          onChange(v === "none" ? null : [c, u]);
        }}
        options={[
          { value: "none", label: "None" },
          ...(!known && alerts[0] ? [{ value: now, label: said }] : []),
          ...EARLY_CHOICES.map(([c, u]) => ({ value: `${c}:${u}`, label: earlyText(c, u) })),
        ]}
      />
    </div>
  );
}

/** Tags as Apple writes them, #name. Tap one to see everything carrying it. */
function TagsField({ reminder, tags, onOpen, onAdd, onRemove }: {
  reminder: Reminder;
  tags: string[];
  onOpen: (name: string) => void;
  onAdd: (name: string) => void;
  onRemove: (name: string) => void;
}) {
  const [typed, setTyped] = useState("");
  if (!REMINDER_DETAIL_WRITES && tags.length === 0) return null;
  return (
    <div className="rx-field">
      <span>Tags</span>
      <div className="rx-tag-field">
        {tags.map((name) => (
          <span className="rx-tag is-chip" key={name}>
            <button type="button" className="rx-tag-name" onClick={() => onOpen(name)} aria-label={`Show reminders tagged ${name}`}>
              #{name}
            </button>
            {REMINDER_DETAIL_WRITES && (
              <button type="button" className="rx-tag-remove" aria-label={`Remove tag ${name}`} onClick={() => onRemove(name)}>
                <X aria-hidden="true" size={11} strokeWidth={2.5} />
              </button>
            )}
          </span>
        ))}
        {REMINDER_DETAIL_WRITES && (
          <input
            className="rx-tag-input"
            value={typed}
            placeholder={tags.length ? "Add" : "Add Tag"}
            aria-label={`Add a tag to ${reminder.title}`}
            onChange={(e) => setTyped(e.target.value.replace(/\s/g, ""))}
            onKeyDown={(e) => {
              if (composing(e)) return;
              if ((e.key === "Enter" || e.key === "," || e.key === " ") && typed.replace(/^#+/, "")) {
                e.preventDefault();
                onAdd(typed);
                setTyped("");
              } else if (e.key === "Backspace" && !typed && tags.length) {
                onRemove(tags[tags.length - 1]);
              }
            }}
            onBlur={() => {
              if (typed.replace(/^#+/, "")) onAdd(typed);
              setTyped("");
            }}
          />
        )}
      </div>
    </div>
  );
}

/** The weekday, month-day or month a rule falls on by its due date, as the phone preselects. */
const WEEKDAY_LETTERS = ["S", "M", "T", "W", "T", "F", "S"];
const WEEKDAY_SHORT = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAY_NAMES = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTH_SHORT = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "every day", "every 2 weeks", "every hour". */
function everyText(frequency: number, interval: number): string {
  const unit = REPEAT_UNITS[frequency] ?? "day";
  return interval > 1 ? `every ${interval} ${unit}s` : `every ${unit}`;
}

/** The end of a day, on the clock of `zone`: an end date takes in every time on that day. */
const endOfDayIn = (day: string, zone: string) => timedInstant(day, "23:59", zone);

/**
 * Repeat, as the iPhone lays it out: the preset menu; Custom's Frequency and Every, with the
 * days the rule falls on; then End Repeat, Never or On Date. Each change is shown at once
 * and saved as it is made.
 */
function RepeatField({ reminder, rule, onChange }: {
  reminder: Reminder;
  rule: RepeatRule | null;
  onChange: (repeat: { frequency: number; interval: number } | null, endAt: number | null, days?: number[]) => void;
}) {
  const [customOpen, setCustomOpen] = useState(false);
  const days = rule?.daysOfWeek ?? [];
  useEffect(() => setCustomOpen(false), [reminder.guid]);
  const repeating = reminder.recurFrequency !== null && reminder.recurFrequency !== undefined;
  const frequency = reminder.recurFrequency ?? 0;
  const interval = reminder.recurInterval ?? 1;
  const endAt = rule?.endAt ?? null;
  const key = repeatKey(reminder, rule, days);
  const custom = repeating && (customOpen || key === "custom");
  const dated = reminder.dueAt !== null;
  const zone = zoneOf(reminder);
  const pad = (n: number) => String(n).padStart(2, "0");
  const endWall = endAt === null ? null : wallIn(zone, endAt);
  const endDay = endWall ? `${endWall.y}-${pad(endWall.m)}-${pad(endWall.d)}` : "";
  const [typed, setTyped] = useState(String(interval));
  useEffect(() => setTyped(String(interval)), [interval, reminder.guid]);
  const commitEvery = (raw: string) => {
    const n = Math.max(1, Math.min(999, Math.round(Number(raw)) || 1));
    setTyped(String(n));
    if (n !== interval) onChange({ frequency, interval: n }, endAt, days);
  };
  // The day the rule falls on, read from the due date on its own zone's clock.
  const due = reminder.dueAt === null ? null : reminder.allDay ? localWall(reminder.dueAt) : wallIn(zone, reminder.dueAt);
  const dueWeekday = due ? new Date(Date.UTC(due.y, due.m - 1, due.d)).getUTCDay() : -1;
  const fromPhone = (rule?.other.length ?? 0) > 0;
  const on = (() => {
    if (fromPhone) return "on the days chosen on your iPhone";
    if (!due) return "";
    if (frequency === 1) {
      const on = days.length ? days : [dueWeekday + 1];
      return `on ${on.map((d) => WEEKDAY_NAMES[d - 1]).join(", ").replace(/, ([^,]*)$/, " and $1")}`;
    }
    if (frequency === 2) return `on day ${due.d}`;
    if (frequency === 3) return `in ${MONTH_SHORT[due.m - 1]}`;
    return "";
  })();
  return (
    <>
      <div className="rx-field">
        <span>Repeat</span>
        <Picker
          label={`Repeat for ${reminder.title}`}
          value={custom ? "custom" : key}
          disabled={!dated && !repeating}
          onChange={(k) => {
            if (k === "custom") {
              setCustomOpen(true);
              // The phone's Custom starts at Daily, every 1.
              if (!repeating) onChange({ frequency: 0, interval: 1 }, null);
              return;
            }
            const chosen = REPEATS.find((o) => o.key === k);
            if (!chosen || chosen.disabled) return;
            setCustomOpen(false);
            onChange(chosen.repeat, chosen.repeat ? endAt : null, chosen.days ?? []);
          }}
          options={REPEATS.map((o) => ({ value: o.key, label: o.label, divider: o.divider, disabled: o.disabled }))}
        />
      </div>
      {!dated && !repeating && <p className="rx-hint">Add a date to repeat it.</p>}
      {custom && (
        <div className="rx-custom" role="group" aria-label={`Custom repeat for ${reminder.title}`}>
          <div className="rx-field">
            <span>Frequency</span>
            <Picker
              label={`Frequency for ${reminder.title}`}
              value={String(frequency)}
              options={FREQUENCIES}
              onChange={(f) => Number(f) !== frequency && onChange({ frequency: Number(f), interval }, endAt, Number(f) === 1 ? days : [])}
            />
          </div>
          <div className="rx-field">
            <span>Every</span>
            <span className="rx-every">
              <button
                type="button"
                className="rx-step"
                aria-label="Fewer"
                disabled={interval <= 1}
                onClick={() => commitEvery(String(interval - 1))}
              >
                −
              </button>
              <input
                className="field rx-every-count"
                inputMode="numeric"
                aria-label={`Every how many ${REPEAT_UNITS[frequency] ?? "day"}s`}
                value={typed}
                onChange={(e) => setTyped(e.target.value.replace(/\D/g, "").slice(0, 3))}
                onBlur={(e) => commitEvery(e.target.value)}
                onKeyDown={(e) => !composing(e) && e.key === "Enter" && e.currentTarget.blur()}
              />
              <button type="button" className="rx-step" aria-label="More" disabled={interval >= 999} onClick={() => commitEvery(String(interval + 1))}>
                +
              </button>
              <span className="rx-every-unit">{REPEAT_UNITS[frequency] ?? "day"}{interval === 1 ? "" : "s"}</span>
            </span>
          </div>
          {frequency === 1 && (
            <div className="rx-days" role="group" aria-label="Days of the week">
              {WEEKDAY_LETTERS.map((letter, i) => {
                // The days it falls on; none chosen is its due date's day, as the phone shows.
                const on = days.length ? days.includes(i + 1) : i === dueWeekday;
                return (
                  <button
                    key={i}
                    type="button"
                    className={`rx-day is-pick${on ? " is-on" : ""}`}
                    aria-label={WEEKDAY_NAMES[i]}
                    aria-pressed={on}
                    disabled={fromPhone}
                    onClick={() => {
                      const now = days.length ? days : dueWeekday >= 0 ? [dueWeekday + 1] : [];
                      const next = now.includes(i + 1) ? now.filter((d) => d !== i + 1) : [...now, i + 1].sort((a, b) => a - b);
                      // The last day off is its due date's day again, as the phone keeps one.
                      onChange({ frequency, interval }, endAt, next.length === 1 && next[0] === dueWeekday + 1 ? [] : next);
                    }}
                  >
                    {letter}
                  </button>
                );
              })}
            </div>
          )}
          {frequency === 2 && (
            <div className="rx-days is-month" role="group" aria-label="Days of the month">
              {Array.from({ length: 31 }, (_, i) => i + 1).map((d) => (
                <span key={d} className={`rx-day${!fromPhone && due?.d === d ? " is-on" : ""}`}
                  aria-current={!fromPhone && due?.d === d ? "true" : undefined}>
                  {d}
                </span>
              ))}
            </div>
          )}
          {frequency === 3 && (
            <div className="rx-days is-year" role="group" aria-label="Months of the year">
              {MONTH_SHORT.map((m, i) => (
                <span key={m} className={`rx-day${!fromPhone && due?.m === i + 1 ? " is-on" : ""}`}
                  aria-current={!fromPhone && due?.m === i + 1 ? "true" : undefined}>
                  {m}
                </span>
              ))}
            </div>
          )}
          <p className="rx-hint rx-custom-hint">
            {`Repeats ${everyText(frequency, interval)}${on ? ` ${on}` : ""}.`}
            {frequency >= 2 && frequency <= 3 && !fromPhone && " Pick other days on your iPhone or iPad."}
          </p>
        </div>
      )}
      {repeating && (
        <div className="rx-field">
          <span>End Repeat</span>
          <div className="rx-due">
            <Picker
              label={`End repeat for ${reminder.title}`}
              value={endAt === null ? "never" : "date"}
              options={[
                { value: "never", label: "Never" },
                { value: "date", label: "On Date" },
              ]}
              onChange={(v) => {
                if (v === "never" && endAt !== null) onChange({ frequency, interval }, null, days);
                if (v === "date" && endAt === null && due) {
                  // A month on from the day it is due, to start from.
                  const at = new Date(Date.UTC(due.y, due.m, due.d));
                  onChange({ frequency, interval }, endOfDayIn(`${at.getUTCFullYear()}-${pad(at.getUTCMonth() + 1)}-${pad(at.getUTCDate())}`, zone), days);
                }
              }}
            />
            {endAt !== null && (
              <input
                type="date"
                className="field rx-due-day"
                aria-label={`End repeat date for ${reminder.title}`}
                defaultValue={endDay}
                key={`e${endAt}`}
                min={due ? `${due.y}-${pad(due.m)}-${pad(due.d)}` : undefined}
                onBlur={(e) => e.target.value && e.target.value !== endDay && onChange({ frequency, interval }, endOfDayIn(e.target.value, zone), days)}
              />
            )}
          </div>
        </div>
      )}
    </>
  );
}

/** The phone's Time Zone row, and Urgent where it is on. */
function ZoneAndUrgent({ reminder, more }: { reminder: Reminder; more: ReminderMore }) {
  const zone = more.timeZone;
  const timed = reminder.dueAt !== null && !reminder.allDay;
  const named = zone && timed ? zoneName(zone, reminder.dueAt!) : null;
  if (!named && !more.urgent) return null;
  return (
    <>
      {named && (
        <div className="rx-field">
          <span>Time Zone</span>
          <span className="rx-value">
            {named.city}
            <span className="rx-quiet">{named.offset}</span>
          </span>
        </div>
      )}
      {more.urgent && (
        <>
          <div className="rx-field">
            <span>Urgent</span>
            <span className="rx-value">On</span>
          </div>
          <p className="rx-hint">Set on your iPhone. It rings as an alarm, even in a Focus or on silent.</p>
        </>
      )}
    </>
  );
}

/** Places & People: where it rings, and who it comes up for when messaging. */
function PlacesAndPeople({ more }: { more: ReminderMore }) {
  const { location, messaging } = more;
  if (!location && !messaging) return null;
  const side = location?.proximity === "arriving" ? "Arriving" : location?.proximity === "leaving" ? "Leaving" : null;
  return (
    <>
      <div className="rx-detail-rule" aria-hidden="true" />
      {location && (
        <div className="rx-field">
          <span>Location</span>
          <span className="rx-value">
            {location.title ?? "A place"}
            {side && <span className="rx-quiet">{side}</span>}
          </span>
        </div>
      )}
      {messaging && (
        <div className="rx-field">
          <span>When Messaging</span>
          <span className="rx-value">{messaging.name ?? messaging.handles[0]}</span>
        </div>
      )}
      <p className="rx-hint">Set on your iPhone, and kept just as it is when you edit here.</p>
    </>
  );
}
