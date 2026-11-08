import { ToastHost, toast } from "../components/Toast";
import { UpdateNotice } from "../components/UpdateNotice";
import { EmptyState } from "../components/EmptyState";
import { StatusLine } from "../components/StatusLine";
import { lazy, Suspense, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { PanelLeft } from "lucide-react";
import { api, DATA_ACCESS, messageOf, NEW_EVENT, OPEN_REMINDER, OPEN_SETTINGS, SYNCED, type AccountInfo, type DataAccess } from "../lib/ipc";
import { ApprovalNotice, ApprovalScreen, awaitsApproval } from "../components/DeviceApproval";
import { SyncProvider, useSync } from "../lib/sync";
import { AccountCard } from "../components/Account";
import { AppFootContext } from "../components/AppFoot";
import { reveal, takeWaiting, type Reveal } from "../lib/reveal";

/*
 * Each app is its own chunk, so the window opens on the code for the app it shows, not all
 * five — Notes' editor alone is a few hundred kilobytes. The rest are fetched as soon as the
 * window is up, before anyone could reach them, so switching never waits on a load.
 */
/**
 * An app's code, fetched once. Once it is here, the screen is drawn in the same frame as the
 * click that opens it: `lazy` alone suspends on first use even when the code arrived long ago
 * (a promise answers a tick later), and that tick was a frame with nothing in the content —
 * the old screen gone, the new one not yet drawn. Handed back at once, it never suspends.
 */
function screen<P extends object>(load: () => Promise<React.ComponentType<P>>) {
  let mod: { default: React.ComponentType<P> } | null = null;
  let loading: Promise<{ default: React.ComponentType<P> }> | null = null;
  const fetch = () =>
    (loading ??= load().then(
      (c) => (mod = { default: c }),
      (e) => {
        // Asked again next time: a chunk that failed to arrive once is not gone for good.
        loading = null;
        throw e;
      },
    ));
  const Screen = lazy(() =>
    mod
      ? // A thenable that answers straight away: `lazy` takes it as already resolved.
        ({ then: (done: (m: { default: React.ComponentType<P> }) => void) => done(mod!) } as unknown as Promise<{ default: React.ComponentType<P> }>)
      : fetch(),
  );
  return { Screen, fetch };
}
const apps = {
  calendar: screen(() => import("./calendar/CalendarView").then((m) => m.CalendarView)),
  reminders: screen(() => import("./RemindersView").then((m) => m.RemindersView)),
  notes: screen(() => import("./NotesView").then((m) => m.NotesView)),
  contacts: screen(() => import("./ContactsView").then((m) => m.ContactsView)),
  drivews: screen(() => import("./DriveView").then((m) => m.DriveView)),
  photos: screen(() => import("./PhotosView").then((m) => m.PhotosView)),
  settings: screen(() => import("./SettingsView").then((m) => m.SettingsView)),
};
const CalendarView = apps.calendar.Screen;
const RemindersView = apps.reminders.Screen;
const NotesView = apps.notes.Screen;
const ContactsView = apps.contacts.Screen;
const DriveView = apps.drivews.Screen;
const PhotosView = apps.photos.Screen;
const SettingsView = apps.settings.Screen;
import { ScreenBoundary } from "../components/ScreenBoundary";
import { ScreenActive, modalOpen } from "../lib/screen";
import { AppIcon } from "../components/AppIcon";
import { GlobalSearch } from "../components/GlobalSearch";
// Only when asked for (Ctrl-/): the window opens without its code.
const ShortcutsSheet = lazy(() => import("../components/ShortcutsSheet").then((m) => ({ default: m.ShortcutsSheet })));

/**
 * What the sidebar holds: the services with a real screen behind them, most used first.
 *
 * Apple advertises thirty-odd services for an account — "CloudKit data", "Push", "Account
 * gateway" — and they were all listed, behind "Show 23 more", along with Photos, Mail and Find
 * My, which opened onto a page saying they were not built. None of that is something a person
 * can use here, so none of it is shown. A service joins this list when it works.
 */
const SCREENS: { key: string; name: string }[] = [
  { key: "reminders", name: "Reminders" },
  { key: "notes", name: "Notes" },
  { key: "drivews", name: "Drive" },
  { key: "photos", name: "Photos" },
  { key: "calendar", name: "Calendar" },
  { key: "contacts", name: "Contacts" },
];

/** The sync service behind a screen, where its key differs: Drive's screen is Apple's `drivews`. */
const SERVICE_OF: Record<string, string> = { drivews: "drive" };
const serviceOf = (screen: string) => SERVICE_OF[screen] ?? screen;

/**
 * Ctrl+R and F5 update the app in front, as they reload a page in a browser — and never
 * reload the window itself, which would throw away every open screen. In Settings they
 * update the whole account.
 */
function UpdateKeys({ screen }: { screen: string | null }) {
  const { refresh, refreshService } = useSync();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const reload = e.key === "F5" || ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "r");
      if (!reload) return;
      e.preventDefault();
      if (e.repeat) return;
      if (screen && SCREENS.some((s) => s.key === screen)) void refreshService(serviceOf(screen), true);
      else void refresh(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [screen, refresh, refreshService]);
  return null;
}

/**
 * Ctrl+, opens Settings and Ctrl+/ (or Ctrl+?) the Keyboard Shortcuts sheet, from any app —
 * as ⌘, and the Help menu's shortcuts do on a Mac. Typing in a field keeps its own Ctrl+/.
 */
function AppKeys({ screen, onSettings }: { screen: string | null; onSettings: () => void }) {
  const [sheet, setSheet] = useState(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.defaultPrevented) return;
      if ((e.key === "," || e.key === "/" || e.key === "?") && modalOpen()) {
        e.preventDefault();
        return;
      }
      if (e.key === ",") {
        e.preventDefault();
        onSettings();
      } else if (e.key === "/" || e.key === "?") {
        e.preventDefault();
        setSheet(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onSettings]);
  return sheet ? (
    <Suspense fallback={null}>
      <ShortcutsSheet front={screen} onClose={() => setSheet(false)} />
    </Suspense>
  ) : null;
}

/**
 * The first time a library arrives on this computer, one line on what to try: change
 * something on the phone and watch it land here — the moment that shows what Skykeep is.
 * Once per computer, only for a library that had nothing synced when the window opened.
 */
const FIRST_HINT_KEY = "quince.firstSyncHint";
function FirstSyncHint() {
  const { report } = useSync();
  const startedEmpty = useRef<boolean | null>(null);
  useEffect(() => {
    if (!report) return;
    const synced = report.services.some((s) => s.lastSyncedAt);
    if (startedEmpty.current === null) startedEmpty.current = !synced;
    if (!startedEmpty.current || !synced) return;
    startedEmpty.current = false;
    try {
      if (window.localStorage.getItem(FIRST_HINT_KEY)) return;
      window.localStorage.setItem(FIRST_HINT_KEY, "1");
    } catch {
      return;
    }
    toast({ message: "Your iCloud is here. Edit a reminder or note on your iPhone and watch it change here in seconds.", duration: 9000 });
  }, [report]);
  return null;
}

/*
 * A result picked in the desktop's own search (GNOME Shell, KRunner) arrives as an event for
 * this window: the item is handed to its screen (which takes it when it is up), and the
 * screen is brought to the front. Listened for from the moment the code loads, so a window
 * opened for the result does not miss it.
 */
const REVEAL = "quince://reveal";
let revealedScreen: string | null = null;
void listen<Reveal>(REVEAL, (e) => {
  revealedScreen = e.payload.screen;
  reveal(e.payload);
  window.dispatchEvent(new CustomEvent("quince:revealed"));
}).catch(() => {});

/*
 * "New Event" from the taskbar's jump list (`skykeep --new event`, src-tauri/src/apps.rs):
 * Calendar brought to the front with a new event started, as its + button does. A Calendar
 * window made for it is asked in its address; one already open is told.
 */
const startNewEvent = () => {
  revealedScreen = "calendar";
  reveal({ screen: "calendar", id: "", startNew: true });
  window.dispatchEvent(new CustomEvent("quince:revealed"));
};
if (new URLSearchParams(window.location.search).get("new") === "event") startNewEvent();
// An item asked for from another app's window, carried in this window's address as it was made
// (apps.rs `app_reveal`): taken as the desktop's search hands one over.
{
  const asked = new URLSearchParams(window.location.search).get("reveal");
  if (asked) {
    try {
      const r = JSON.parse(atob(asked.replace(/-/g, "+").replace(/_/g, "/"))) as Reveal;
      revealedScreen = r.screen;
      reveal(r);
    } catch {
      // A request that can't be read opens the app as asked, at its start.
    }
  }
}
void listen(NEW_EVENT, startNewEvent).catch(() => {});

/** The screen open last time, so the app reopens where it was left. */
const LAST_KEY = "quince.library.screen";
const RAIL_KEY = "quince.library.rail";

/** The app this window opens on, when it was opened as one (`?app=notes`, from its launcher
 * entry or the tray). */
const ONLY = new URLSearchParams(window.location.search).get("app");
/**
 * An app's own window is that app alone, as a Mac's Notes is: its own folders are the window's
 * sidebar, with the account and sync status at their foot, and no switcher of the six. Another
 * app asked for from here (a link, a notification, Calendar's Show in Reminders) opens in its
 * own window; Settings open in Skykeep's main window, the one place they live.
 */
const SOLO = ONLY && ONLY in apps ? ONLY : null;

/**
 * The code for the screen this window will most likely open on, asked for as this file loads —
 * beside the session check, rather than after the sidebar's list has come back.
 */
const LIKELY = (() => {
  if (ONLY && ONLY in apps) return ONLY as keyof typeof apps;
  try {
    const last = window.localStorage.getItem(LAST_KEY);
    return last && last in apps ? (last as keyof typeof apps) : null;
  } catch {
    return null;
  }
})();
if (LIKELY) void apps[LIKELY].fetch().catch(() => {});

/** Mounted beside a screen under its Suspense, so it mounts in the commit that draws the
 *  screen itself — once its code is in, never while the pane is still blank. */
function Drawn({ then }: { then: () => void }) {
  useEffect(() => then(), [then]);
  return null;
}

export function Library({ onSignOut, onShown }: { onSignOut: (keep: boolean) => Promise<void>; onShown?: () => void }) {
  const [services, setServices] = useState<string[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  /** False when the account keeps its data from the web: nothing here can be read. */
  const [webAccess, setWebAccess] = useState<boolean | null>(null);
  /** In a narrow window the sidebar slides over the content, and only when asked for. */
  const [sidebarOpen, setSidebarOpen] = useState(false);
  /** A wide window can fold the sidebar to its icons, as Finder and Mail can; remembered. */
  const [rail, setRail] = useState(() => {
    try {
      return window.localStorage.getItem(RAIL_KEY) === "1";
    } catch {
      return false;
    }
  });
  /** Folding applies to a wide window only; a narrow one has its sidebar whole, over the content. */
  const [wide, setWide] = useState(() => window.matchMedia("(min-width: 901px)").matches);
  useEffect(() => {
    const mq = window.matchMedia("(min-width: 901px)");
    const on = () => setWide(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  const content = useRef<HTMLElement | null>(null);
  /** Where the content began before a fold, for it to slide from. */
  const slideFrom = useRef<number | null>(null);
  const contentLeft = () => {
    const el = content.current;
    return el ? el.getBoundingClientRect().left + parseFloat(getComputedStyle(el).paddingLeft) : 0;
  };
  /**
   * Folding the sidebar moves the content without laying it out on every frame. Animating
   * the content's padding made the whole screen — a calendar grid, a long list — lay itself
   * out sixty times a second, and that was the stutter. Instead it takes its new width at
   * once and slides from where it was, which the compositor does alone. Started in a layout
   * effect — after the new layout, before it is painted — so no frame ever shows the content
   * in its new place and then back in the old: started a frame later, that jump was the
   * whole window glitching.
   */
  useLayoutEffect(() => {
    const from = slideFrom.current;
    slideFrom.current = null;
    const el = content.current;
    if (from === null || !el || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    // The content takes its new place at once; only the sidebar's own width moves. Sliding
    // the whole content — a screen of text — made WebKitGTK redraw its text each frame, and
    // it shimmered and flickered on his machine; moving its padding laid it out every frame.
    void from;
  }, [rail]);
  const toggleRail = () => {
    slideFrom.current = contentLeft();
    setRailState();
  };
  const setRailState = () =>
    setRail((was) => {
      try {
        window.localStorage.setItem(RAIL_KEY, was ? "0" : "1");
      } catch {
        // Folds for now; forgotten on restart.
      }
      return !was;
    });

  /**
   * Apple waiting on something at icloud.com — new iCloud terms to agree to, or the account to
   * finish — before it opens a whole web session: every service fails until then, Calendar and
   * Contacts too (issue #5). Read again after every pass, so it clears by itself once done.
   */
  const [atIcloud, setAtIcloud] = useState<"terms" | "repair" | null>(null);
  const takeAccount = useCallback((a: AccountInfo | null | undefined) => {
    setWebAccess(a?.webAccessAllowed ?? null);
    setAtIcloud(a?.termsUpdateNeeded ? "terms" : a?.repairNeeded ? "repair" : null);
  }, []);
  useEffect(() => {
    api.accountInfo().then(takeAccount, () => {});
    const off = listen(SYNCED, () => {
      api.accountInfo().then(takeAccount, () => {});
    });
    return () => {
      void off.then((f) => f());
    };
  }, [takeAccount]);
  const [checking, setChecking] = useState(false);
  const [checkError, setCheckError] = useState<string | null>(null);
  const checkAgain = () => {
    setChecking(true);
    setCheckError(null);
    api
      .accountRefresh()
      .then(takeAccount, (e) => setCheckError(messageOf(e)))
      .finally(() => setChecking(false));
  };

  /** Advanced Data Protection: which services wait on an approval on the person's devices. */
  const [access, setAccess] = useState<DataAccess | null>(null);
  useEffect(() => {
    let live = true;
    api.dataAccess().then((a) => live && setAccess(a), () => {});
    const off = listen<DataAccess>(DATA_ACCESS, (e) => setAccess(e.payload));
    return () => {
      live = false;
      void off.then((f) => f());
    };
  }, []);
  // Apple saying web access is off, as the session found it, is the same screen as at sign-in.
  useEffect(() => {
    if (access?.state === "webAccessOff") setWebAccess(false);
  }, [access]);
  /** When each service last synced, while an approval is awaited: one with a copy here keeps it. */
  const [lastSynced, setLastSynced] = useState<Record<string, number | null>>({});
  useEffect(() => {
    if (!access || access.state === "open") return undefined;
    const read = () =>
      api
        .syncStatus()
        .then((r) => setLastSynced(Object.fromEntries(r.services.map((s) => [s.service, s.lastSyncedAt ?? null]))), () => {});
    void read();
    const off = listen(SYNCED, () => void read());
    return () => {
      void off.then((f) => f());
    };
  }, [access]);

  useEffect(() => {
    api
      .services()
      .then((list) => {
        const offered = SCREENS.map((s) => s.key).filter((k) => list.includes(k));
        setServices(offered);
        let last: string | null = null;
        try {
          last = window.localStorage.getItem(LAST_KEY);
        } catch {
          // Without storage it opens on the first screen, which is fine.
        }
        setSelected(
          ONLY && SCREENS.some((x) => x.key === ONLY)
            ? ONLY
            : last && (offered.includes(last) || last === "settings") ? last : (offered[0] ?? null),
        );
        setReady(true);
      })
      .catch((e) => setError(messageOf(e)));
  }, []);

  // The open app's accent belongs to the whole window — menus and popovers drawn over it
  // too — as each Mac app colours itself: Notes yellow, Calendar red, the rest blue.
  useEffect(() => {
    document.documentElement.dataset.app = selected ?? "";
    return () => {
      delete document.documentElement.dataset.app;
    };
  }, [selected]);

  // The window is named for what it shows: an app's own window that has moved on to Photos
  // is not "Notes" any more. Outside Tauri, or where not permitted, the title simply stays.
  useEffect(() => {
    if (!selected) return;
    const title = selected === "settings" ? "Settings" : SCREENS.find((s) => s.key === selected)?.name;
    if (!title) return;
    document.title = title;
    // Asked of the window plugin by itself: `@tauri-apps/api/window` for this one call was a
    // quarter of the code the window had to read before it could draw anything.
    try {
      const label = (window as { __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } } }).__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
      if (label) void invoke("plugin:window|set_title", { label, value: title }).catch(() => {});
    } catch {
      // Not in a Tauri window.
    }
  }, [selected]);

  // The window is shown once there is something in it: the sidebar's apps, and the screen in
  // front drawn with its code in (`Drawn`, beside it under its Suspense) — so it maps on the
  // app, not on a blank pane that fills a moment later. With no screen to draw it is said as
  // soon as that is known. (The backend shows the window anyway after three seconds.)
  const told = useRef(false);
  const onShownRef = useRef(onShown);
  onShownRef.current = onShown;
  const drawn = useCallback(() => {
    if (told.current) return;
    told.current = true;
    onShownRef.current?.();
  }, []);
  /**
   * The screen in front waits on an approval: drawn in its place when there is nothing of it on
   * this computer yet, or as a card over it when there is — its copy stays readable meanwhile.
   */
  const frontService = selected && selected !== "settings" ? serviceOf(selected) : null;
  const approval: "screen" | "notice" | null =
    frontService && access && awaitsApproval(access, frontService) ? (lastSynced[frontService] ? "notice" : "screen") : null;
  const noScreen =
    Boolean(error) || (ready && (!selected || webAccess === false || approval === "screen" || atIcloud !== null));
  useEffect(() => {
    if (noScreen) drawn();
  }, [noScreen, drawn]);

  // The other apps' code, fetched while nothing else is happening.
  useEffect(() => {
    if (!ready) return undefined;
    const t = window.setTimeout(() => {
      for (const app of Object.values(apps)) void app.fetch().catch(() => {});
    }, 300);
    return () => window.clearTimeout(t);
  }, [ready]);

  // The screens opened so far, in the order they were first opened.
  const keptRef = useRef<string[]>([]);
  if (webAccess !== false && selected && SCREENS.some((s) => s.key === selected) && !keptRef.current.includes(selected)) {
    keptRef.current = [...keptRef.current, selected];
  }
  const kept = keptRef.current;
  /**
   * Each screen's element, made once and handed back the same ever after: React passes over an
   * element it has already drawn, so the sidebar moving, the window's title changing or a sync
   * landing in this component no longer draws every screen kept here again — each switch of
   * app used to redraw all of them, out of sight or not. What they read from context (the sync,
   * whether they are in front) still reaches them.
   */
  const made = useRef(new Map<string, JSX.Element>());
  const screenFor = (key: string) => {
    let el = made.current.get(key);
    if (!el) made.current.set(key, (el = makeScreen(key)));
    return el;
  };
  const makeScreen = (key: string) =>
    key === "calendar" ? (
      <CalendarView />
    ) : key === "reminders" ? (
      <RemindersView />
    ) : key === "notes" ? (
      <NotesView />
    ) : key === "contacts" ? (
      <ContactsView />
    ) : key === "photos" ? (
      <PhotosView />
    ) : (
      <DriveView />
    );

  /**
   * The app the sidebar marks. It is the one in front, except while an arrow key is held: then
   * it walks down the apps at the key's pace and the content follows only once it settles.
   */
  const [marked, setMarked] = useState<string | null>(null);
  const shownMark = marked ?? selected;
  /** An app the arrows have reached and not yet opened (see `glide`). */
  const gliding = useRef<string | null>(null);
  const settleTimer = useRef<number | undefined>(undefined);

  const choose = (key: string) => {
    if (SOLO && key !== SOLO) {
      // Not this window's app: Settings in the main window, another app in its own.
      if (key === "settings") void invoke("settings_open").catch(() => {});
      else {
        const asked = takeWaiting(key);
        void (asked ? invoke("app_reveal", { key, reveal: asked }) : invoke("app_open", { key })).catch(() => {});
      }
      return;
    }
    window.clearTimeout(settleTimer.current);
    gliding.current = null;
    setMarked(null);
    setSelected(key);
    setSidebarOpen(false);
    try {
      window.localStorage.setItem(LAST_KEY, key);
    } catch {
      // Remembering is a convenience; choosing still works.
    }
  };
  const chooseRef = useRef(choose);
  chooseRef.current = choose;

  /**
   * The arrows in the sidebar, as in a Mac's: the selection moves at once, and the app it lands
   * on opens when the key is let go. Held, every app passed used to be opened on the way — each
   * one's whole screen built (Photos' grid, a long address book) at each repeat of the key —
   * and the walk crawled. A single press onto an app already open opens it straight away, as
   * that costs nothing; one never opened yet opens as the key comes up, a moment later.
   */
  const glide = (key: string, repeat: boolean) => {
    window.clearTimeout(settleTimer.current);
    if (!repeat && keptRef.current.includes(key)) {
      choose(key);
      return;
    }
    gliding.current = key;
    setMarked(key);
    // A key let go out of this window's sight (focus taken elsewhere) is never heard: then it
    // settles after a pause longer than a key's first repeat waits.
    settleTimer.current = window.setTimeout(() => settle(), 700);
  };
  const settle = () => {
    const key = gliding.current;
    if (key !== null) chooseRef.current(key);
  };
  useEffect(() => {
    const onUp = (e: KeyboardEvent) => {
      if (gliding.current !== null && ["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) settle();
    };
    const onBlur = () => settle();
    window.addEventListener("keyup", onUp);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keyup", onUp);
      window.removeEventListener("blur", onBlur);
      window.clearTimeout(settleTimer.current);
    };
  }, []);

  useEffect(() => {
    if (!sidebarOpen) return undefined;
    // Escape a field or menu inside it answered for itself (the search steps back first) is not
    // this one's.
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !e.defaultPrevented && setSidebarOpen(false);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      // Put away with the focus inside it: back to the button that brings it out, rather than
      // left in a sidebar no longer drawn.
      if (document.activeElement?.closest(".sidebar") && window.matchMedia("(max-width: 900px)").matches) {
        document.querySelector<HTMLElement>(".sidebar-toggle")?.focus({ preventScroll: true });
      }
    };
  }, [sidebarOpen]);

  // Clicking a reminder's alert brings the window back on Reminders.
  useEffect(() => {
    // A screen asking for another (Calendar's "Show in Reminders") names it on the event.
    const front = (e?: Event) => {
      const screen = revealedScreen ?? (e as CustomEvent<{ screen?: string }> | undefined)?.detail?.screen ?? null;
      revealedScreen = null;
      if (screen && services.includes(screen)) choose(screen);
    };
    front();
    window.addEventListener("quince:revealed", front);
    // Sent to every window: the main window (or Reminders' own) answers it; another app's
    // window leaves it be, rather than opening Reminders a second time.
    const stop = listen<string>(OPEN_REMINDER, () => {
      if (services.includes("reminders") && (!SOLO || SOLO === "reminders")) choose("reminders");
    });
    const settings = SOLO ? null : listen(OPEN_SETTINGS, () => choose("settings"));
    return () => {
      window.removeEventListener("quince:revealed", front);
      void stop.then((unlisten) => unlisten()).catch(() => {});
      void settings?.then((unlisten) => unlisten()).catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [services]);

  // Native sidebars move with the arrow keys and occupy a single tab stop. Without this a
  // keyboard user has to Tab through every row to reach the content.
  const rows = useRef<(HTMLButtonElement | null)[]>([]);
  const order = services;

  function onKeyDown(e: React.KeyboardEvent, index: number) {
    const move = (to: number) => {
      e.preventDefault();
      const clamped = Math.max(0, Math.min(to, order.length - 1));
      rows.current[clamped]?.focus();
      if (order[clamped] !== shownMark || gliding.current !== null) glide(order[clamped], e.repeat);
    };
    if (e.key === "ArrowDown") move(index + 1);
    else if (e.key === "ArrowUp") move(index - 1);
    else if (e.key === "Home") move(0);
    else if (e.key === "End") move(order.length - 1);
  }

  const item = (key: string) => {
    const index = order.indexOf(key);
    return (
      <button
        key={key}
        ref={(el) => {
          rows.current[index] = el;
        }}
        className="nav-item"
        aria-current={shownMark === key}
        tabIndex={shownMark === key ? 0 : -1}
        onKeyDown={(e) => onKeyDown(e, index)}
        onClick={() => choose(key)}
        title={rail ? (SCREENS.find((s) => s.key === key)?.name ?? key) : undefined}
      >
        <AppIcon app={key} />
        <span className="nav-name">{SCREENS.find((s) => s.key === key)?.name ?? key}</span>
      </button>
    );
  };

  // An app's own window: the account and sync status at the foot of the app's own sidebar,
  // opening Settings in the main window (see `choose`).
  const soloFoot = SOLO ? (
    <AccountCard
      current={false}
      onOpen={() => choose("settings")}
      app={(() => {
        const app = SCREENS.find((s) => s.key === SOLO);
        return app && webAccess !== false ? { service: serviceOf(app.key), name: app.name } : null;
      })()}
    />
  ) : null;

  return (
    <SyncProvider>
      <UpdateKeys screen={selected} />
      <AppKeys screen={selected} onSettings={() => choose("settings")} />
      <FirstSyncHint />
      <AppFootContext.Provider value={soloFoot}>
      <div className={`library enter-library${SOLO ? " is-solo" : `${sidebarOpen ? " is-sidebar-open" : ""}${rail ? " is-rail" : ""}`}`}>
        {!SOLO && (
        <>
        {/* Only in a narrow window, where the sidebar gives its room to the content. */}
        <button
          className="sidebar-toggle"
          aria-label={sidebarOpen ? "Hide Sidebar" : "Show Sidebar"}
          aria-expanded={sidebarOpen}
          title={sidebarOpen ? "Hide Sidebar" : "Show Sidebar"}
          onClick={() => setSidebarOpen((open) => !open)}
        >
          <PanelLeft size={17} strokeWidth={2} aria-hidden="true" />
        </button>
        <div className="sidebar-scrim" aria-hidden="true" onClick={() => setSidebarOpen(false)} />
        <nav className="sidebar" aria-label="Your iCloud">
          <div className="sidebar-head">
            <h2>iCloud</h2>
            <button
              className="icon-button sidebar-fold"
              aria-label={rail ? "Show Sidebar Names" : "Fold Sidebar to Icons"}
              aria-pressed={rail}
              title={rail ? "Show Sidebar Names" : "Fold Sidebar to Icons"}
              onClick={toggleRail}
            >
              <PanelLeft size={16} strokeWidth={2} aria-hidden="true" />
            </button>
          </div>
          {/* Folded, a magnifier stands in for the field; Ctrl+K reaches it either way, and in a
              narrow window brings the sidebar out to it. */}
          <GlobalSearch
            available={services}
            folded={rail && wide}
            onSummon={() => {
              if (window.matchMedia("(max-width: 900px)").matches) setSidebarOpen(true);
            }}
            onOpen={(screen) => choose(screen)}
          />
          {services.map(item)}
          {!ready && !error && <StatusLine tone="pending">Loading…</StatusLine>}
          {error && <StatusLine tone="error">{error}</StatusLine>}

          <div className="sidebar-foot">
            <AccountCard
              current={selected === "settings"}
              onOpen={() => choose("settings")}
              app={(() => {
                const app = SCREENS.find((s) => s.key === selected);
                return app && webAccess !== false ? { service: serviceOf(app.key), name: app.name } : null;
              })()}
            />
          </div>
        </nav>
        </>
        )}

        {/* Which screen is in front, for its layout (each screen's CSS keys on it): asked with
            :has() across the whole content instead, WebKit checked it again on every change
            anywhere inside — every tile's selection in a 20,000-photo grid. */}
        <main className="content" ref={content} data-screen={selected}>
          {atIcloud && webAccess !== false && selected !== "settings" ? (
            // Apple opens only part of a web session until this is done at icloud.com, and
            // every screen would fail its own way. One plain answer, with where to do it.
            <div className="web-access-off icloud-com-waiting" role="alert">
              <h1>{atIcloud === "terms" ? "Agree to the new iCloud terms" : "Finish setting up iCloud"}</h1>
              <p>
                {atIcloud === "terms"
                  ? "Apple has updated the iCloud Terms and Conditions. Once you agree to them, your iCloud opens here."
                  : "Apple needs one more step on your Apple Account before your iCloud opens here."}
              </p>
              <p>
                Sign in at <b>icloud.com</b> in your browser and follow what it asks
                {atIcloud === "terms" ? ", then agree to the terms" : ""}. Then click Try Again.
              </p>
              <p className="device-approval-status" role="status" aria-live="polite">
                <span>{checkError ?? (checking ? "Checking with Apple…" : "")}</span>
              </p>
              <button className="primary" onClick={checkAgain} disabled={checking}>
                Try Again
              </button>
            </div>
          ) : webAccess === false && selected !== "settings" ? (
            // Advanced Data Protection with web access off: every read would fail, and each
            // screen would say so its own way. One plain answer, with Apple's own switch.
            <div className="web-access-off" role="alert">
              <h1>Your iCloud data is kept off the web</h1>
              <p>
                Your Apple Account uses Advanced Data Protection with “Access iCloud Data on the Web”
                turned off, so Apple doesn’t let any app outside your devices — Skykeep included —
                read your iCloud.
              </p>
              <p>
                To use Skykeep, turn it on: on your iPhone, open <b>Settings</b>, tap your name, then
                <b> iCloud</b>, and turn on <b>Access iCloud Data on the Web</b>. Then sign in to Skykeep
                again.
              </p>
            </div>
          ) : (
            <>
              {selected === "settings" && (
                <ScreenBoundary key="settings" name="Settings">
                  <Suspense fallback={null}>
                    <SettingsView onSignOut={onSignOut} />
                    <Drawn then={drawn} />
                  </Suspense>
                </ScreenBoundary>
              )}
              {/* Each screen is kept once opened, out of sight while another is in front:
                  coming back is instant, and finds it as it was left — its place, its
                  selection, its scroll — as a Mac app's windows are. Out of sight it is inert,
                  draws no menus and answers no keys (lib/screen). */}
              {approval === "screen" && access && frontService && (
                <ApprovalScreen access={access} onAccess={setAccess} />
              )}
              {kept.map((key) => {
                const on = key === selected && approval !== "screen";
                return (
                  <div key={key} className="screen-slot" hidden={!on} {...(on ? {} : { inert: "" })}>
                    <ScreenActive.Provider value={on}>
                      <ScreenBoundary name={SCREENS.find((s) => s.key === key)?.name ?? "This screen"}>
                        <Suspense fallback={null}>
                          {screenFor(key)}
                          {on && <Drawn then={drawn} />}
                        </Suspense>
                      </ScreenBoundary>
                    </ScreenActive.Provider>
                  </div>
                );
              })}
            </>
          )}
          {ready && services.length === 0 && (
            <EmptyState>
              Apple hasn’t offered Reminders, Notes, iCloud Drive, Calendar or Contacts to
              this account yet.
            </EmptyState>
          )}
        </main>
        <ToastHost>
          {approval === "notice" && access && frontService && (
            <ApprovalNotice access={access} lastSyncedAt={lastSynced[frontService] ?? null} onAccess={setAccess} />
          )}
          <UpdateNotice />
        </ToastHost>
      </div>
      </AppFootContext.Provider>
    </SyncProvider>
  );
}
