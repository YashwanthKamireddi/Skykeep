import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { api, messageOf, SYNCED, type ItemChanges, type Refresh } from "./ipc";
import { useScreenActive } from "./screen";

/**
 * One refresh for the whole account, shared by every screen.
 *
 * Screens read the local library and never the network, so they do not need to know a
 * refresh is happening — only that one finished. `revision` is that signal: it ticks once
 * per completed pass, from the button or from the background timer alike, and a screen
 * re-reads by listing it as a dependency.
 */
type Sync = {
  /** What the last pass did, per service. */
  report: Refresh | null;
  /** A pass is in flight. */
  syncing: boolean;
  /** Ticks once per completed pass. Screens re-read when it changes. */
  revision: number;
  /** Ticks per service whose rows a pass changed; `all` for a change not placed. */
  revisions: Readonly<Record<string, number>>;
  /** Ask for a pass now. Safe to call while one is running; it will not start a second. */
  /** `asked`: the person pressed Sync Now (see `api.syncAll`). */
  refresh: (asked?: boolean) => Promise<void>;
  /** The last refresh that failed outright, rather than a single service reporting trouble. */
  error: string | null;
  /** Refresh one service alone — one app's "update now". Never starts a second pass of it. */
  refreshService: (service: string, asked?: boolean) => Promise<void>;
  /** The services being refreshed on their own right now. */
  updating: readonly string[];
  /** Why a service's own refresh failed outright, by service. */
  serviceErrors: Readonly<Record<string, string>>;
  /** When the whole-account pass in flight began (unix seconds), or null. */
  passStartedAt: number | null;
  /** Like `revisions`, but only for changes whose items could not be named (`useItemChanges`). */
  wholeRevisions: Readonly<Record<string, number>>;
  /** Be told the items each pass changes in a service, when they are known. */
  subscribe: (service: string, told: (items: ItemChanges) => void) => () => void;
};

const Context = createContext<Sync | null>(null);

/** The least time between refreshes brought on by coming back to the window. */
export const RETURN_THROTTLE_MS = 30_000;

/**
 * The service this window is the app for (`?app=notes`), or null in Skykeep's own window. An
 * app's window opening or coming to the front asks for its own service only: it asked for all
 * six, so opening Notes fetched Photos, Drive and Calendar's whole months too, once per window
 * and again on every return (desktop audit M1). Skykeep's window, which shows every service's
 * state, still asks for all of them.
 */
const OWN_SERVICE = (() => {
  if (typeof window === "undefined") return null;
  const app = new URLSearchParams(window.location.search).get("app");
  return app && ["notes", "reminders", "calendar", "contacts", "drivews", "photos"].includes(app) ? app : null;
})();

export function SyncProvider({ children }: { children: ReactNode }) {
  const [report, setReport] = useState<Refresh | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [revision, setRevision] = useState(0);
  const [revisions, setRevisions] = useState<Record<string, number>>({});
  const [wholeRevisions, setWholeRevisions] = useState<Record<string, number>>({});
  const [error, setError] = useState<string | null>(null);
  /** Who is told of each service's items, as `useItemChanges` asks. */
  const listeners = useRef(new Map<string, Set<(items: ItemChanges) => void>>());
  const subscribe = useCallback((service: string, told: (items: ItemChanges) => void) => {
    const mine = listeners.current.get(service) ?? new Set();
    listeners.current.set(service, mine);
    mine.add(told);
    return () => {
      mine.delete(told);
    };
  }, []);
  /**
   * A pass landed. With the services it changed named, only their screens read again — and
   * none, when it stored nothing new: a clean pass landing every service told every open
   * screen to read its whole list seven times. Unnamed, every screen reads again.
   */
  const bump = useCallback((changed?: readonly string[], items?: Readonly<Record<string, ItemChanges>>) => {
    if (Array.isArray(changed) && changed.length === 0) return;
    const services = Array.isArray(changed) ? changed : ["all"];
    setRevision((n) => n + 1);
    setRevisions((was) => {
      const next = { ...was };
      for (const s of services) next[s] = (next[s] ?? 0) + 1;
      return next;
    });
    // A service whose items are named: those who take them redraw just those rows. The rest —
    // and every screen that reads by `revisions` — read whole, as before.
    const whole: string[] = [];
    for (const s of services) {
      const named = items?.[s];
      const told = listeners.current.get(s);
      if (named && told?.size) for (const tell of told) tell(named);
      else if (!named) whole.push(s);
    }
    if (whole.length)
      setWholeRevisions((was) => {
        const next = { ...was };
        for (const s of whole) next[s] = (next[s] ?? 0) + 1;
        return next;
      });
  }, []);
  // A second click while a pass is running would mean two walks of the same zones.
  const inFlight = useRef(false);
  const [updating, setUpdating] = useState<string[]>([]);
  const [serviceErrors, setServiceErrors] = useState<Record<string, string>>({});
  const [passStartedAt, setPassStartedAt] = useState<number | null>(null);
  const serviceInFlight = useRef(new Set<string>());

  const refresh = useCallback(async (asked?: boolean) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setSyncing(true);
    setPassStartedAt(Math.floor(Date.now() / 1000));
    setError(null);
    // Each service that stored something says so as it lands (the backend's event); a pass
    // that cannot say so has every screen read again, as before.
    let told = false;
    try {
      const done = await api.syncAll(asked);
      told = Array.isArray(done?.changed);
      setReport(done);
    } catch (e) {
      setError(messageOf(e));
    } finally {
      inFlight.current = false;
      setSyncing(false);
      setPassStartedAt(null);
      if (!told) bump();
    }
  }, [bump]);

  const refreshService = useCallback(async (service: string, asked?: boolean) => {
    if (serviceInFlight.current.has(service)) return;
    serviceInFlight.current.add(service);
    setUpdating([...serviceInFlight.current]);
    setServiceErrors(({ [service]: _gone, ...rest }) => rest);
    let told = false;
    try {
      const part = await api.syncSome([service], asked);
      told = Array.isArray(part?.changed);
      // The backend's event brings the whole picture; this keeps the answer on screen even
      // where no event arrives, by folding this service's own report into the last one.
      // A reply with no report in it leaves the last one as it was.
      setReport((was) =>
        !Array.isArray(part?.services)
          ? was
          : was
          ? {
              ...was,
              services: [
                ...was.services.filter((s) => !part.services.some((p) => p.service === s.service)),
                ...part.services,
              ],
            }
          : part,
      );
    } catch (e) {
      const reason = messageOf(e);
      setServiceErrors((was) => ({ ...was, [service]: reason }));
    } finally {
      serviceInFlight.current.delete(service);
      setUpdating([...serviceInFlight.current]);
      if (!told) bump();
    }
  }, [bump]);

  useEffect(() => {
    let cancelled = false;
    const pass = () => (OWN_SERVICE ? refreshService(OWN_SERVICE) : refresh());

    // Show the freshness the library already knows before asking Apple anything, so the
    // window is never blank while the first pass runs.
    api
      .syncStatus()
      .then((known) => {
        if (!cancelled) setReport(known);
      })
      .catch(() => {
        /* the refresh below will say what is wrong */
      })
      // Opening the window is itself a request for current data, so a pass always runs.
      // Showing the stored freshness first means the wait happens behind real content.
      .finally(() => {
        if (!cancelled) void pass();
      });

    // Background passes finish without anyone asking, and every open screen has to notice.
    const stop = listen<Refresh>(SYNCED, (event) => {
      if (cancelled) return;
      setReport(event.payload);
      bump(event.payload?.changed, event.payload?.items);
    });

    /*
     * Coming back to the window is when somebody looks for a change they just made on their
     * phone. Between the five-minute background passes it could be up to five minutes stale
     * at exactly that moment, which reads as sync not working at all. So returning to the
     * window refreshes — but not more than once in half a minute, because switching back and
     * forth between two windows is not a reason to ask Apple the same question again.
     */
    let lastFocusPass = Date.now();
    const onReturn = () => {
      if (document.visibilityState === "hidden") return;
      if (Date.now() - lastFocusPass < RETURN_THROTTLE_MS) return;
      lastFocusPass = Date.now();
      void pass();
    };
    window.addEventListener("focus", onReturn);
    document.addEventListener("visibilitychange", onReturn);

    return () => {
      cancelled = true;
      void stop.then((off) => off());
      window.removeEventListener("focus", onReturn);
      document.removeEventListener("visibilitychange", onReturn);
    };
  }, [refresh, refreshService, bump]);

  const value = useMemo(
    () => ({ report, syncing, revision, revisions, refresh, error, refreshService, updating, serviceErrors, passStartedAt, wholeRevisions, subscribe }),
    [report, syncing, revision, revisions, refresh, error, refreshService, updating, serviceErrors, passStartedAt, wholeRevisions, subscribe]
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useSync(): Sync {
  const found = useContext(Context);
  if (!found) throw new Error("useSync must be used inside a SyncProvider");
  return found;
}

/**
 * A revision for the screen that shows these services: it moves when a pass changed one of
 * them (or changed something it could not name), and stays put for passes that changed only
 * what another screen shows.
 */
export function useRevision(...services: string[]): number {
  const { revisions } = useSync();
  return services.reduce((n, s) => n + (revisions[s] ?? 0), revisions.all ?? 0);
}

/**
 * `useRevision`, for a screen kept out of sight: it holds still while another screen is in
 * front, and catches up once when this one comes back. Each pass that landed used to have every
 * kept screen read its whole list again behind the one being looked at (lib/screen).
 */
export function useShownRevision(...services: string[]): number {
  const revision = useRevision(...services);
  const active = useScreenActive();
  const seen = useRef(revision);
  if (active) seen.current = revision;
  return seen.current;
}

/**
 * For a screen that redraws only what a pass changed. `apply` is handed the ids of the items a
 * pass added, changed and removed in `service`, as each lands; the revision returned moves only
 * for a change whose items could not be named, which the screen reads whole. A screen kept out
 * of sight holds both until it comes back, then takes them once — the items put together.
 *
 * Every other screen keeps reading by `useRevision`, which moves for both.
 */
export function useItemChanges(service: string, apply: (items: ItemChanges) => void): number {
  const { wholeRevisions, subscribe } = useSync();
  const whole = (wholeRevisions[service] ?? 0) + (wholeRevisions.all ?? 0);
  const active = useScreenActive();
  const seen = useRef(whole);
  if (active) seen.current = whole;
  const latest = useRef({ apply, active });
  latest.current = { apply, active };
  const waiting = useRef<ItemChanges | null>(null);
  useEffect(
    () =>
      subscribe(service, (items) => {
        if (latest.current.active) latest.current.apply(items);
        else waiting.current = joinItems(waiting.current, items);
      }),
    [service, subscribe],
  );
  useEffect(() => {
    if (!active || !waiting.current) return;
    const items = waiting.current;
    waiting.current = null;
    latest.current.apply(items);
  }, [active]);
  return seen.current;
}

/** Two passes' items as one: an id named twice is read again once, as changed. */
export function joinItems(a: ItemChanges | null, b: ItemChanges): ItemChanges {
  if (!a) return b;
  const named = new Set([...a.added, ...a.changed, ...a.removed]);
  const twice = new Set([...b.added, ...b.changed, ...b.removed].filter((id) => named.has(id)));
  const keep = (ids: string[]) => ids.filter((id) => !twice.has(id));
  return {
    added: [...keep(a.added), ...keep(b.added)],
    changed: [...new Set([...a.changed, ...b.changed, ...twice])],
    removed: [...keep(a.removed), ...keep(b.removed)],
  };
}

/** "just now", "4 min ago", "yesterday" — how old the library is, in words. */
export function freshness(asOf: number | null): string {
  if (asOf === null) return "Not synced yet";
  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - asOf);
  if (seconds < 60) return "Updated just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `Updated ${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `Updated ${hours} hr ago`;
  const days = Math.floor(hours / 24);
  return days === 1 ? "Updated yesterday" : `Updated ${days} days ago`;
}

/**
 * Whether a service's first copy is still on its way: never synced here, and a pass running
 * (or not yet reported). An empty screen then means "coming", not "you have none" — saying the
 * second while the first is true told a person with years of reminders that they had none.
 */
export function useArriving(service: string): boolean {
  const { report, syncing, error } = useSync();
  if (error) return false;
  const mine = report?.services.find((s) => s.service === service);
  if (
    mine?.outcome === "failed" ||
    mine?.outcome === "unavailable" ||
    mine?.outcome === "paused" ||
    mine?.outcome === "preparing"
  )
    return false;
  if (mine?.lastSyncedAt) return false;
  return syncing || report === null;
}

/** Said while Apple prepares a library, where the pass did not say it in its own words. */
const PREPARING = "Apple is still preparing your photo library. It will appear here when it's ready.";

/** One service as an app shows it: whether it is being fetched, and how it last went. */
export type ServiceState = {
  /** Being fetched now — on its own, or as part of a whole-account pass it has not landed from. */
  working: boolean;
  /** Why it did not update, when it did not. */
  failure: string | null;
  lastSyncedAt: number | null;
  unavailable: boolean;
  /** What Apple said while it is still preparing the service (Photos), in its words: not
   * empty, not failed. */
  preparing: string | null;
};

/**
 * One service's own state, for the app showing it. During a whole-account pass a service
 * counts as working only until its own part has landed — the passes run side by side, and
 * Reminders is done long before Photos is.
 */
export function useServiceState(service: string): ServiceState {
  const { report, syncing, passStartedAt, updating, serviceErrors } = useSync();
  const mine = report?.services.find((s) => s.service === service);
  const lastSyncedAt = mine?.lastSyncedAt ?? null;
  const landed = passStartedAt !== null && lastSyncedAt !== null && lastSyncedAt >= passStartedAt;
  const preparing = mine?.outcome === "preparing" ? (mine.detail ?? PREPARING) : null;
  const working =
    updating.includes(service) ||
    (syncing && !landed && mine?.outcome !== "unavailable" && mine?.outcome !== "paused" && !preparing);
  const failure = serviceErrors[service] ?? (mine?.outcome === "failed" ? (mine.detail ?? "It didn’t update") : null);
  return { working, failure, lastSyncedAt, unavailable: mine?.outcome === "unavailable", preparing };
}
