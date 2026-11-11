import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { api, type OutboxStatus, type RefusedChange } from "./ipc";
import { t, type Key } from "./i18n";

const CHANGED = "quince://outbox-changed";
const REFUSED = "quince://outbox-refused";

export type OutboxService = "notes" | "reminders" | "contacts" | "calendar" | "photos";

/**
 * The network came back (the webview's own `online` event): what waits is sent now, not on the
 * sender's backoff. On Linux NetworkManager or connman says the same over D-Bus; this is the
 * word on every platform. Once per window, however many views ask about the outbox.
 */
let woken = false;
function wakeWhenOnline() {
  if (woken || typeof window === "undefined") return;
  woken = true;
  window.addEventListener("online", () => {
    invoke("outbox_wake").catch(() => {});
  });
}

/** What a refused change was, in this app's words — for a reader of the notice. */
export function changeWords(r: Pick<RefusedChange, "service" | "kind">): string | null {
  const words: Record<string, Record<string, Key>> = {
    notes: { save: "outbox.notes.save", create: "outbox.notes.create", move: "outbox.notes.move", delete: "outbox.notes.delete", pin: "outbox.notes.pin" },
    reminders: { create: "outbox.reminders.create", edit: "outbox.reminders.edit", delete: "outbox.reminders.delete", done: "outbox.reminders.done", advance: "outbox.reminders.advance" },
    contacts: { save: "outbox.contacts.save", delete: "outbox.contacts.delete", merge: "outbox.contacts.merge", address: "outbox.contacts.address" },
    calendar: { save: "outbox.calendar.save", delete: "outbox.calendar.delete" },
    photos: { favorite: "outbox.photos.favorite", hidden: "outbox.photos.hidden", deleted: "outbox.photos.deleted", expunged: "outbox.photos.expunged" },
  };
  const key = words[r.service]?.[r.kind];
  return key ? t(key) : null;
}

/**
 * Changes made here that Apple does not have yet, for one service: which records are waiting
 * (shown as "Waiting to sync"), and the changes Apple turned down — kept, with what was typed,
 * until the person sends them again or lets them go.
 *
 * The backend says when this moves (`quince://outbox-changed`), once a pass of sending is over,
 * naming the services it sent for or found waiting. `tick` counts those that name this one, for
 * a view to read its rows again: a Notes save landing re-read Reminders and Calendar too, each
 * whole, twice. A word with no services named (a change resent or let go) is for every view.
 */
export function useOutbox(service: OutboxService) {
  wakeWhenOnline();
  const [status, setStatus] = useState<OutboxStatus>({ waiting: [], refused: [] });
  const [tick, setTick] = useState(0);

  const read = useCallback(() => {
    api.outboxStatus().then(
      (s) => setStatus(s && Array.isArray(s.waiting) ? s : { waiting: [], refused: [] }),
      () => {},
    );
  }, []);

  useEffect(() => {
    read();
    const offs = [
      listen<{ services?: string[] } | null>(CHANGED, (e) => {
        const services = e.payload?.services;
        if (Array.isArray(services) && !services.includes(service)) return;
        read();
        setTick((n) => n + 1);
      }),
      listen(REFUSED, () => read()),
    ];
    return () => offs.forEach((off) => void off.then((f) => f()).catch(() => {}));
  }, [read, service]);

  const waiting = useMemo(
    () => new Set(status.waiting.filter((w) => w.service === service).map((w) => w.record)),
    [status, service],
  );
  const refused = useMemo(() => status.refused.filter((r) => r.service === service), [status, service]);

  const resend = useCallback(async (r: RefusedChange) => {
    await api.outboxResend(r.id).catch(() => {});
    read();
  }, [read]);
  const discard = useCallback(async (r: RefusedChange) => {
    await api.outboxDiscard(r.id).catch(() => {});
    read();
  }, [read]);

  return { waiting, refused, resend, discard, refresh: read, tick };
}
