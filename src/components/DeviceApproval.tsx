import { t, activeLanguage, type Key } from "../lib/i18n";
import { useLayoutEffect, useRef, useState } from "react";
import { Smartphone } from "lucide-react";
import { fadeIn } from "../lib/fade";
import { api, messageOf, type DataAccess } from "../lib/ipc";
import { freshness } from "../lib/sync";
import { Spinner } from "./Spinner";

/** What each service is called on screen, in the order they are listed. */
const NAMES: [string, Key][] = [
  ["notes", "approval.notes"],
  ["reminders", "approval.reminders"],
  ["drive", "approval.drive"],
  ["photos", "approval.photos"],
];

/** "Notes, Reminders and iCloud Drive". */
function listed(services: readonly string[]): string {
  const names = NAMES.filter(([key]) => services.includes(key)).map(([, name]) => t(name));
  if (names.length <= 1) return names[0] ?? t("approval.yourICloud");
  return new Intl.ListFormat(activeLanguage(), { type: "conjunction" }).format(names);
}

/** The states in which a service waits on an approval on the person's devices. */
export function awaitsApproval(access: DataAccess | null, service: string): boolean {
  return Boolean(
    access && (access.state === "waiting" || access.state === "needed" || access.state === "notSent") && access.services.includes(service),
  );
}

/** Sync Now: Apple is asked, the approval put on the person's devices when it is needed. */
function useSyncNow(onAccess: (a: DataAccess) => void) {
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ask = () => {
    setAsking(true);
    setError(null);
    api
      .dataAccessAsk()
      .then(onAccess, (e) => setError(messageOf(e)))
      .finally(() => setAsking(false));
  };
  return { ask, asking, error };
}

/** The line that says where the approval stands, when there is something to say. */
function standing(access: DataAccess, problem: string | null): string | null {
  if (problem) return problem;
  if (access.state === "notSent") {
    return t("approval.notSent");
  }
  if (access.state === "waiting") return t("approval.waiting");
  return null;
}

/**
 * Advanced Data Protection keeps these services end-to-end encrypted, and Apple lets Skykeep
 * sync them for a while each time the person approves on one of their devices. Drawn in place
 * of a screen that has nothing on this computer yet, in the same calm layout as the web-access
 * page beside it.
 */
export function ApprovalScreen({
  access,
  onAccess,
}: {
  access: DataAccess;
  onAccess: (a: DataAccess) => void;
}) {
  const { ask, asking, error } = useSyncNow(onAccess);
  const waiting = access.state === "waiting";
  const line = standing(access, error ?? access.problem);
  return (
    <div className="web-access-off device-approval">
      <Smartphone className="device-approval-glyph" size={30} strokeWidth={1.6} aria-hidden="true" />
      <h1>{t("approval.title")}</h1>
      <p>
        {t("approval.body", { services: listed(access.services) })}
      </p>
      <p className="device-approval-status" role="status" aria-live="polite">
        {waiting && <Spinner small />}
        <span>{line ?? ""}</span>
      </p>
      <button className="primary" onClick={ask} disabled={asking}>
        {t("approval.syncNow")}
      </button>
    </div>
  );
}

/**
 * The same, for a screen whose copy is already on this computer: the screen stays, readable and
 * editable, and one card at the foot of the content says how it syncs and when it last did.
 */
export function ApprovalNotice({
  access,
  lastSyncedAt,
  onAccess,
}: {
  access: DataAccess;
  lastSyncedAt: number | null;
  onAccess: (a: DataAccess) => void;
}) {
  const { ask, asking, error } = useSyncNow(onAccess);
  const card = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => fadeIn(card.current, { duration: 140 }), []);
  const waiting = access.state === "waiting";
  const names = listed(access.services);
  const line = standing(access, error ?? access.problem);
  return (
    <div ref={card} className="toast update-notice device-approval-notice" role="group" aria-label={t("ui.advancedDataProtection")}>
      {waiting ? <Spinner small /> : <Smartphone className="toast-glyph" size={15} strokeWidth={2} aria-hidden="true" />}
      <div className="update-body">
        <p className="update-line" role="status" aria-live="polite">
          {waiting
            ? t("approval.approveToSync", { services: names })
            : t("approval.cardBody", { services: names })}
        </p>
        <p className="update-line device-approval-when">{line ?? freshness(lastSyncedAt)}</p>
        {!waiting && (
          <div className="update-actions">
            <button className="toast-action" onClick={ask} disabled={asking}>
              {t("approval.syncNow")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
