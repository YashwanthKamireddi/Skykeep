import { ToastHost, toast } from "../components/Toast";
import { UpdateNotice } from "../components/UpdateNotice";
import { EmptyState } from "../components/EmptyState";
import { StatusLine } from "../components/StatusLine";
import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { api, DATA_ACCESS, messageOf, NEW_EVENT, SYNCED, type AccountInfo, type DataAccess } from "../lib/ipc";
import { ApprovalNotice, ApprovalScreen, awaitsApproval } from "../components/DeviceApproval";
import { SyncProvider, useSync } from "../lib/sync";
import { AccountCard } from "../components/Account";
import { AppFootContext } from "../components/AppFoot";
import { useSidePane } from "../components/SidePane";
import { reveal, takeWaiting, type Reveal } from "../lib/reveal";

/*
 * Each app is its own chunk, so the window opens on the code for the app it shows, not all
 * seven — Notes' editor alone is a few hundred kilobytes. Each app is its own window now and a
 * window only ever shows its own (`choose` opens any other in its window), so the others are
 * never fetched here: fetched 300 ms after opening, as they were when one window switched
 * between apps, each window parsed and held about 700 KB it could never show (audit H9).
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
  const sheetOpen = useRef(false);
  sheetOpen.current = sheet;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.defaultPrevented) return;
      // The key that opened the shortcuts closes them, as a toggle does.
      if ((e.key === "/" || e.key === "?") && sheetOpen.current) {
        e.preventDefault();
        setSheet(false);
        return;
      }
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


/** The app this window opens on, when it was opened as one (`?app=notes`, from its launcher
 * entry or the tray). */
const ONLY = new URLSearchParams(window.location.search).get("app");
/**
 * Every window is one thing, as on a Mac (docs/apps-as-windows.md). An app's own window is that
 * app alone: its own folders are the window's sidebar, with the account and sync status at
 * their foot, and no switcher of the six. Skykeep's own window, signed in, is Settings alone.
 * Another app asked for from either (a link, Calendar's Show in Reminders) opens in its own
 * window, and Settings in Skykeep's.
 */
const SOLO: string = ONLY && ONLY in apps ? ONLY : "settings";

/**
 * The code for the screen this window will most likely open on, asked for as this file loads —
 * beside the session check, rather than after the sidebar's list has come back.
 */
const LIKELY = ONLY && ONLY in apps ? (ONLY as keyof typeof apps) : "settings" in apps ? ("settings" as keyof typeof apps) : null;
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
  const content = useRef<HTMLElement | null>(null);

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
        setSelected(SOLO);
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
    // Skykeep's own window is Skykeep (the account, sync, Backup and settings), as its icon
    // and the tray name it; an app's window is the app.
    const title = selected === "settings" ? "Skykeep" : SCREENS.find((s) => s.key === selected)?.name;
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

  /** This window's one thing, or another app's window, or Settings in Skykeep's window. */
  const choose = (key: string) => {
    if (key !== SOLO) {
      if (key === "settings") void invoke("settings_open").catch(() => {});
      else {
        const asked = takeWaiting(key);
        void (asked ? invoke("app_reveal", { key, reveal: asked }) : invoke("app_open", { key })).catch(() => {});
      }
      return;
    }
    setSelected(key);
  };

  // Every app's own sidebar moves with the arrows, as a Mac's does: Up and Down step through its
  // rows, Home and End go to the ends; Return or Space opens the row, as a click does. One rule
  // for all six sidebars, so none moves differently.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
      const at = (e.target as HTMLElement | null)?.closest?.<HTMLElement>(".source-row");
      const side = at?.closest<HTMLElement>("aside");
      if (!at || !side) return;
      const rows = [...side.querySelectorAll<HTMLElement>(".source-row")].filter((r) => r.offsetParent !== null);
      const i = rows.indexOf(at);
      const to = { ArrowDown: i + 1, ArrowUp: i - 1, Home: 0, End: rows.length - 1 }[e.key];
      if (to === undefined || i < 0) return;
      e.preventDefault();
      // A row that is a label (Calendar's calendars) is reached by the box it holds.
      const row = rows[Math.max(0, Math.min(rows.length - 1, to))];
      (row?.matches("label") ? row.querySelector<HTMLElement>("input, button") : row)?.focus();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // A request for an item (a link, the desktop's search, another app's Show in…): this
  // window's own is taken by its screen; another app's is handed to that app's window.
  useEffect(() => {
    const front = (e?: Event) => {
      const screen = revealedScreen ?? (e as CustomEvent<{ screen?: string }> | undefined)?.detail?.screen ?? null;
      revealedScreen = null;
      if (screen && services.includes(screen)) choose(screen);
    };
    front();
    window.addEventListener("quince:revealed", front);

    return () => {
      window.removeEventListener("quince:revealed", front);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [services]);

  // Apple waiting on something at icloud.com (new terms, an account to finish): every service
  // fails until then, so an app's window says so in its place, and Skykeep's window above its
  // page. One plain answer, with where to do it.
  const icloudNotice =
    atIcloud && webAccess !== false ? (
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
    ) : null;

  // The sidebar button: in an app's own window, over an app (not a notice in its place).
  const side = useSidePane(SOLO !== "settings" && !icloudNotice && webAccess !== false ? SOLO : null);

  // An app's own window: the account and sync status at the foot of the app's own sidebar,
  // opening Settings in the main window (see `choose`).
  const soloFoot = SOLO !== "settings" ? (
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
      <div className={`library enter-library is-solo${side.className}`} data-app={SOLO}>
        {side.button}

        {/* Which screen is in front, for its layout (each screen's CSS keys on it): asked with
            :has() across the whole content instead, WebKit checked it again on every change
            anywhere inside — every tile's selection in a 20,000-photo grid. */}
        <main className="content" ref={content} data-screen={selected}>
          {/* The apps Apple offers this account could not be read: said here, where the
              screen would be. */}
          {error && !ready && <StatusLine tone="error">{error}</StatusLine>}
          {/* In Skykeep's own window the same notice sits above the page, and Settings stay
              usable beneath it (Sign Out, Try Again). */}
          {icloudNotice && selected === "settings" && icloudNotice}
          {icloudNotice && selected !== "settings" ? (
            icloudNotice
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
