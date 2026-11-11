/**
 * The top of Skykeep's own window: the six apps, each with its own icon, what it holds from
 * the library on this computer, and how fresh it is — one click into the app's own window
 * (docs/apps-as-windows.md). It replaces the list of services that only said how fresh each
 * was: the same states, now where the apps are.
 *
 * What each tile says comes from one local read for all six (`launcher_summary`), asked again
 * when a pass lands, never a request to Apple.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { SYNCED, type ServiceReport } from "../lib/ipc";
import { freshness } from "../lib/sync";
import { appName, useI18n, t } from "../lib/i18n";
import notesIcon from "../assets/apps/notes.svg";
import remindersIcon from "../assets/apps/reminders.svg";
import calendarIcon from "../assets/apps/calendar.svg";
import contactsIcon from "../assets/apps/contacts.svg";
import driveIcon from "../assets/apps/drive.svg";
import photosIcon from "../assets/apps/photos.svg";

type Summary = {
  notes: number;
  remindersToday: number;
  eventsToday: number;
  nextEvent: { title: string; startsAt: number; allDay: boolean } | null;
  contacts: number;
  photos: number;
};

/** The apps, as their windows and the tray name them: the service a pass reports, the key
 *  their window opens by, and the icon their launcher shows. */
const APPS = [
  { service: "notes", key: "notes", name: "Notes", icon: notesIcon },
  { service: "reminders", key: "reminders", name: "Reminders", icon: remindersIcon },
  { service: "calendar", key: "calendar", name: "Calendar", icon: calendarIcon },
  { service: "contacts", key: "contacts", name: "Contacts", icon: contactsIcon },
  { service: "drive", key: "drivews", name: "Drive", icon: driveIcon },
  { service: "photos", key: "photos", name: "Photos", icon: photosIcon },
] as const;

const time = (seconds: number) =>
  new Date(seconds * 1000).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });

/** What a tile says it holds. */
function holds(key: string, s: Summary | null, driveFree: string | null): string {
  if (!s) return " ";
  switch (key) {
    case "notes":
      return s.notes === 1 ? t("launcher.notesCountOne") : t("launcher.notesCount", { count: s.notes.toLocaleString() });
    case "reminders":
      return s.remindersToday
        ? s.remindersToday === 1
          ? t("launcher.remindersDueOne")
          : t("launcher.remindersDue", { count: s.remindersToday.toLocaleString() })
        : t("launcher.nothingDueToday");
    case "calendar":
      return s.nextEvent
        ? `${time(s.nextEvent.startsAt)} · ${s.nextEvent.title}`
        : s.eventsToday
          ? s.eventsToday === 1
            ? t("launcher.eventsTodayOne")
            : t("launcher.eventsToday", { count: s.eventsToday.toLocaleString() })
          : t("launcher.nothingToday");
    case "contacts":
      return s.contacts === 1 ? t("launcher.contactsCountOne") : t("launcher.contactsCount", { count: s.contacts.toLocaleString() });
    case "drivews":
      return driveFree ? t("launcher.driveAvailable", { size: driveFree }) : " ";
    case "photos":
      return s.photos === 1 ? t("launcher.photosCountOne") : t("launcher.photosCount", { count: s.photos.toLocaleString() });
    default:
      return " ";
  }
}

/** How fresh a tile's app is — the states the Sync list said, word for word. */
/**
 * A failed pass in a tile's few words; the whole sentence is the tile's tooltip and part of what
 * it says to a screen reader. The sentence as the line itself ("iCloud is busy right now.
 * Nothing was changed — try again in a minute.") was cut off mid-word in every tile.
 */
function shortly(detail: string | null | undefined): string {
  const d = (detail ?? "").toLowerCase();
  if (d.includes("busy")) return t("launcher.busyRetrying");
  if (d.includes("connection") || d.includes("offline") || d.includes("reach") || d.includes("took too long")) return t("launcher.offlineRetrying");
  return t("launcher.didntUpdate");
}

function fresh(
  r: ServiceReport | undefined,
  syncing: boolean,
): { text: string; full: string; state: "clean" | "working" | "trouble" | "idle" } {
  const said = (text: string, state: "clean" | "working" | "trouble" | "idle") => ({ text, full: text, state });
  if (!r) return said(syncing ? t("common.updating") : t("sync.notSyncedYet"), syncing ? "working" : "idle");
  if (r.outcome === "failed") return { text: shortly(r.detail), full: r.detail ?? t("launcher.didntUpdate"), state: "trouble" };
  if (r.outcome === "paused") return said(t("launcher.syncsWhenApproved", { when: freshness(r.lastSyncedAt ?? null) }), "idle");
  if (r.outcome === "preparing") return said(t("launcher.preparing"), "idle");
  if (syncing && !r.lastSyncedAt) return said(t("common.loading"), "working");
  if (!r.lastSyncedAt) return said(t("sync.notSyncedYet"), "idle");
  return said(freshness(r.lastSyncedAt), syncing ? "working" : "clean");
}

export function Launcher({
  services,
  syncing,
  onSync,
  driveFree,
}: {
  services: ServiceReport[];
  syncing: boolean;
  onSync: () => void;
  driveFree: string | null;
}) {
  const { t } = useI18n();
  const [summary, setSummary] = useState<Summary | null>(null);
  const read = useCallback(() => {
    void invoke<Summary>("launcher_summary").then(setSummary, () => {});
  }, []);
  useEffect(() => {
    read();
    // A pass that landed may have changed a count; the day may have turned since.
    const stop = listen(SYNCED, read);
    window.addEventListener("focus", read);
    return () => {
      void stop.then((unlisten) => unlisten()).catch(() => {});
      window.removeEventListener("focus", read);
    };
  }, [read]);

  // Every app the account has: only what Apple says it does not offer is left out. A pass that
  // has not reached an app yet still shows it, as not synced yet.
  const offered = APPS.filter(
    (a) => !services.some((s) => s.service === a.service && s.outcome === "unavailable"),
  );
  const tiles = useRef<(HTMLButtonElement | null)[]>([]);
  // The tiles are one stop for Tab, the arrows move between them, as a grid of icons does.
  const [at, setAt] = useState(0);
  const move = (to: number) => {
    const next = Math.max(0, Math.min(offered.length - 1, to));
    setAt(next);
    tiles.current[next]?.focus();
  };
  const across = () => {
    const first = tiles.current[0];
    const row = first ? tiles.current.filter((t) => t && t.offsetTop === first.offsetTop).length : 3;
    return Math.max(1, row);
  };

  return (
    <section className="launcher" aria-label={t("launcher.appsAria")}>
      <div className="launcher-head">
        <span className="launcher-hint">{t("launcher.hint")}</span>
        <button className="quiet" onClick={onSync} disabled={syncing}>
          {syncing ? t("sync.syncing") : t("launcher.syncNow")}
        </button>
      </div>
      <div className="launcher-grid" role="group" aria-label={t("launcher.appsGridAria")}>
        {offered.map((a, i) => {
          const f = fresh(
            services.find((s) => s.service === a.service),
            syncing,
          );
          const name = appName(a.service) ?? a.name;
          return (
            <button
              key={a.key}
              ref={(el) => {
                tiles.current[i] = el;
              }}
              className={`launch-tile is-${f.state}`}
              tabIndex={i === at ? 0 : -1}
              onFocus={() => setAt(i)}
              onClick={() => void invoke("app_open", { key: a.key }).catch(() => {})}
              onKeyDown={(e) => {
                const by = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: across(), ArrowUp: -across() }[e.key];
                if (by) {
                  e.preventDefault();
                  move(i + by);
                } else if (e.key === "Home") {
                  e.preventDefault();
                  move(0);
                } else if (e.key === "End") {
                  e.preventDefault();
                  move(offered.length - 1);
                }
              }}
              aria-label={`${name}. ${holds(a.key, summary, driveFree)}. ${f.full}`}
              title={f.full !== f.text ? f.full : undefined}
            >
              <img className="launch-icon" src={a.icon} alt="" draggable={false} />
              <span className="launch-name">{name}</span>
              <span className="launch-holds">{holds(a.key, summary, driveFree)}</span>
              <span className="launch-fresh">
                <span className="launch-dot" aria-hidden="true" />
                <span className="launch-fresh-text">{f.text}</span>
              </span>
            </button>
          );
        })}
      </div>
    </section>
  );
}
