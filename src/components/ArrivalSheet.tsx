import { t } from "../lib/i18n";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { api, type Arrival, type Calendar, type ImportEvent } from "../lib/ipc";
import { Picker } from "./Picker";
import { useModalFocus } from "../lib/focus";
import { fadeIn } from "../lib/fade";
import "../styles/intake.css";

/**
 * One arrived file's sheet (ImportSheet.tsx keeps the queue): what it holds, and one button
 * that adds it. In a chunk of its own, fetched only when a file has arrived — the window opens
 * without reading it.
 */
export default function ArrivalSheet({ arrival: current, onDone: done }: { arrival: Arrival; onDone: () => void }) {
  return createPortal(
    <div className="modal-veil ix-veil">
      {current.kind === "events" ? (
        <EventsSheet arrival={current} onDone={done} />
      ) : current.kind === "contacts" ? (
        <ContactsSheet arrival={current} onDone={done} />
      ) : (
        <Shell label={`Can’t open “${current.file}”`} onCancel={done}>
          <h2>Can’t open “{current.file}”</h2>
          <p className="ix-reason">{current.reason}</p>
          <div className="sheet-actions">
            <button className="primary" autoFocus onClick={done}>
              OK
            </button>
          </div>
        </Shell>
      )}
    </div>,
    document.body,
  );
}

function Shell({ label, onCancel, children }: { label: string; onCancel: () => void; children: React.ReactNode }) {
  const sheetRef = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheetRef);
  // The next file's sheet takes the last one's place on the same dim, where the page-wide
  // fade (lib/fade fadeSheetsIn) does not look: it fades itself in, before it is painted.
  useLayoutEffect(() => fadeIn(sheetRef.current, { duration: 140 }), []);
  return (
    <div
      ref={sheetRef}
      className="modal-sheet ix-sheet"
      role="dialog"
      aria-modal="true"
      aria-label={label}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onCancel();
        }
      }}
    >
      {children}
    </div>
  );
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** When an event is, as Calendar says it: the day, and its time in its own zone. */
function when(e: ImportEvent): string {
  const day: Intl.DateTimeFormatOptions = { weekday: "short", day: "numeric", month: "short", year: "numeric" };
  if (e.allDay) {
    const last = e.endsAt - 86400;
    const first = new Date(e.startsAt * 1000).toLocaleDateString(undefined, day);
    return last > e.startsAt ? `${first} – ${new Date(last * 1000).toLocaleDateString(undefined, day)}` : t("arrival.allDay", { day: first });
  }
  const zone = e.timezone ?? undefined;
  const fmt = (t: number, o: Intl.DateTimeFormatOptions) => {
    try {
      return new Date(t * 1000).toLocaleString(undefined, { ...o, timeZone: zone });
    } catch {
      return new Date(t * 1000).toLocaleString(undefined, o);
    }
  };
  const time: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };
  const start = fmt(e.startsAt, { ...day, ...time });
  const end = e.endsAt > e.startsAt ? `–${fmt(e.endsAt, time)}` : "";
  const here = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const elsewhere = zone && zone !== here ? ` (${zone.replace(/_/g, " ")})` : "";
  return `${start}${end}${elsewhere}`;
}

function repeatPhrase(e: ImportEvent): string | null {
  const r = e.repeat;
  if (!r) return null;
  const unit = ({ daily: "day", weekly: "week", monthly: "month", yearly: "year" } as const)[r.freq];
  const every = t(`calendar.every.${unit}${r.interval === 1 ? "One" : "Many"}`, { count: r.interval });
  return r.count ? t("calendar.times", { repeat: every, count: r.count }) : every;
}

function EventsSheet({ arrival, onDone }: { arrival: Extract<Arrival, { kind: "events" }>; onDone: () => void }) {
  const [calendars, setCalendars] = useState<Calendar[] | null>(null);
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState<{ done: number; failed: string[] } | null>(null);
  const [finished, setFinished] = useState<string | null>(null);
  const fresh = useMemo(() => arrival.events.filter((e) => !e.duplicate), [arrival]);

  useEffect(() => {
    api
      .calendarList()
      .then((all) => {
        // Only the account's own calendars take a new event: not a subscribed feed, not the
        // Birthdays and Holidays calendars Skykeep draws itself.
        const writable = all.filter((c) => c.writable && !c.guid.startsWith("quince:"));
        setCalendars(writable);
        setTarget((t) => t || (writable.find((c) => c.isDefault) ?? writable[0])?.guid || "");
      })
      .catch(() => setCalendars([]));
  }, []);

  async function add() {
    let failed: string[] = [];
    setBusy({ done: 0, failed });
    for (const [i, e] of fresh.entries()) {
      try {
        await api.eventSave({
          guid: null,
          calendarGuid: target,
          title: e.title,
          location: e.location,
          startsAt: e.startsAt,
          endsAt: e.endsAt,
          allDay: e.allDay,
          timezone: e.timezone,
          notes: e.notes,
          url: e.url,
          ...(e.alerts ? { alerts: e.alerts } : {}),
          ...(e.repeat ? { repeat: e.repeat, repeatChanged: true } : {}),
        });
      } catch (err) {
        failed = [...failed, `${e.title}: ${String(err)}`];
      }
      setBusy({ done: i + 1, failed });
    }
    // The calendar's screens redraw from the library once the pass lands.
    api.syncSome(["calendar"]).catch(() => {});
    if (failed.length === 0) return onDone();
    setBusy(null);
    setFinished(
      `${plural(fresh.length - failed.length, "event was", "events were")} added. ${plural(failed.length, "wasn’t", "weren’t")}:\n${failed.join("\n")}`,
    );
  }

  const single = fresh.length === 1;
  const heading =
    fresh.length === 0
      ? arrival.events.length > 0
        ? t("arrival.eventsAlready")
        : t("arrival.nothingToAdd")
      : single
        ? t("arrival.addOneTo", { title: fresh[0].title })
        : t("arrival.addManyTo", { count: fresh.length });
  const options = (calendars ?? []).map((c) => ({ value: c.guid, label: c.title }));

  return (
    // Not put away mid-import: the import finishing would then put away the next file.
    <Shell label={`Import “${arrival.file}”`} onCancel={busy ? () => {} : onDone}>
      <div className="ix-head">
        <h2>{heading}</h2>
        {fresh.length > 0 && calendars && calendars.length > 0 && (
          <Picker value={target} options={options} onChange={setTarget} label={t("ui.calendar")} disabled={!!busy} />
        )}
      </div>
      <p className="ix-file">{arrival.file}</p>
      <ul className="ix-list" aria-label={t("ui.eventsInThisFile")}>
        {arrival.events.map((e, i) => (
          <li key={i} className={e.duplicate ? "is-skipped" : undefined}>
            <span className="ix-title">{e.title}</span>
            <span className="ix-sub">{when(e)}</span>
            {(repeatPhrase(e) || e.duplicate) && (
              <span className="ix-sub">{e.duplicate ? t("ui.alreadyInCalendar") : repeatPhrase(e)}</span>
            )}
          </li>
        ))}
        {arrival.refused.map((r, i) => (
          <li key={`r${i}`} className="is-refused">
            <span className="ix-title">{r.title}</span>
            <span className="ix-sub">Can’t be added. {r.reason}</span>
          </li>
        ))}
      </ul>
      {calendars && calendars.length === 0 && fresh.length > 0 && (
        <p className="ix-reason">{t("ui.thisAccountHasNoCalendarThatCanTakeNewEvents")}</p>
      )}
      {finished && <p className="ix-reason ix-error">{finished}</p>}
      <div className="sheet-actions">
        {finished || fresh.length === 0 ? (
          <button className="primary" autoFocus onClick={onDone}>
            {finished ? t("common.done") : t("common.ok")}
          </button>
        ) : (
          <>
            <button className="quiet" onClick={onDone} disabled={!!busy}>
              {t("arrival.cancel")}
            </button>
            <button className="primary" autoFocus onClick={add} disabled={!!busy || !target}>
              {busy ? t("arrival.addingOf", { done: busy.done + 1 > fresh.length ? fresh.length : busy.done + 1, total: fresh.length }) : single ? t("arrival.addEvent") : t("arrival.addEvents", { count: fresh.length })}
            </button>
          </>
        )}
      </div>
    </Shell>
  );
}

function ContactsSheet({ arrival, onDone }: { arrival: Extract<Arrival, { kind: "contacts" }>; onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const live = useRef(true);
  useEffect(() => () => void (live.current = false), []);
  const fresh = arrival.contacts.filter((c) => !c.known);
  const single = fresh.length === 1;
  const heading =
    fresh.length === 0
      ? arrival.contacts.length === 1
        ? t("arrival.contactAlready", { name: arrival.contacts[0].name })
        : t("arrival.everyoneAlready")
      : single
        ? t("arrival.addContactNamed", { name: fresh[0].name })
        : t("arrival.addContactsCount", { count: fresh.length });

  async function add() {
    setBusy(true);
    setError(null);
    try {
      // The same import the Contacts app's own Import uses: someone already there is left out.
      await api.contactsImport([arrival.path]);
      if (live.current) onDone();
    } catch (err) {
      if (live.current) {
        setBusy(false);
        setError(String(err));
      }
    }
  }

  return (
    <Shell label={`Import “${arrival.file}”`} onCancel={busy ? () => {} : onDone}>
      <h2>{heading}</h2>
      <p className="ix-file">{arrival.file}</p>
      <ul className="ix-list" aria-label={t("ui.contactsInThisFile")}>
        {arrival.contacts.map((c, i) => (
          <li key={i} className={c.known ? "is-skipped" : undefined}>
            <span className="ix-title">{c.name}</span>
            {(c.detail || c.known) && <span className="ix-sub">{c.known ? t("ui.alreadyInContacts") : c.detail}</span>}
          </li>
        ))}
      </ul>
      {error && <p className="ix-reason ix-error">{error}</p>}
      <div className="sheet-actions">
        {fresh.length === 0 ? (
          <button className="primary" autoFocus onClick={onDone}>
            OK
          </button>
        ) : (
          <>
            <button className="quiet" onClick={onDone} disabled={busy}>
              {t("arrival.cancel")}
            </button>
            <button className="primary" autoFocus onClick={add} disabled={busy}>
              {busy ? t("arrival.adding") : single ? t("arrival.addContact") : t("arrival.addContacts", { count: fresh.length })}
            </button>
          </>
        )}
      </div>
    </Shell>
  );
}
