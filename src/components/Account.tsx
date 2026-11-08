import { useEffect, useRef, useState } from "react";
import { RotateCw } from "lucide-react";
import { Face } from "./Face";
import { api, contactPhotoSrc, type AccountInfo } from "../lib/ipc";

/** Said where the session is held in memory only: what it means, and what fixes it. */
export const NOT_KEPT =
  "Signed in until Skykeep closes: start a system keyring (GNOME Keyring or KWallet) to stay signed in.";

/**
 * Whether the session lasts only as long as Skykeep runs — no keyring would keep it. Asked
 * again after every update, which is when the session is written again.
 */
export function useNotKept(): boolean {
  const { report } = useSync();
  const [notKept, setNotKept] = useState(false);
  useEffect(() => {
    let live = true;
    api.status().then((s) => live && setNotKept(!!s.notKept), () => {});
    return () => {
      live = false;
    };
  }, [report]);
  return notKept;
}
import { freshness, useServiceState, useSync } from "../lib/sync";

/** "Asha Rao" → "AR"; an address → its first letter. */
export function initialsOf(name: string): string {
  const words = name.split(/[\s@._-]+/).filter(Boolean);
  if (words.length === 0) return "";
  if (name.includes("@") || words.length === 1) return words[0][0]!.toUpperCase();
  return (words[0][0]! + words[words.length - 1][0]!).toUpperCase();
}

/**
 * The account's picture: the photo on the person's own card, as Apple shows it; their
 * initials on Apple's grey disc where there is none, or until it has loaded.
 */
export function AccountAvatar({ account, large }: { account: AccountInfo | null; large?: boolean }) {
  const initials = initialsOf(account?.name ?? account?.email ?? "");
  return (
    <span className={`account-avatar${large ? " is-large" : ""}`} aria-hidden="true">
      <Face src={account?.photoContact ? contactPhotoSrc(account.photoContact) : null} initials={initials} />
    </span>
  );
}

/** The app in front, as the account's footer speaks for it: its sync service and its name. */
export type FooterApp = { service: string; name: string };

/**
 * The account at the foot of the sidebar — who is signed in and how fresh their iCloud is —
 * and the way into Settings, as System Settings puts the account at the head of its list.
 *
 * With an app in front it speaks for that app, as Mail's footer does for its mailboxes: how
 * fresh that app is, and a quiet control to update it alone. In Settings it speaks for the
 * whole account.
 */
export function AccountCard({ current, onOpen, app }: { current: boolean; onOpen: () => void; app?: FooterApp | null }) {
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const { report, syncing, error, refreshService, refresh: refreshAll } = useSync();
  const notKept = useNotKept();
  const own = useServiceState(app?.service ?? "");
  // "Updated just now" has to become "2 min ago" without anything else happening.
  const [, tick] = useState(0);
  useEffect(() => {
    api.accountInfo().then(setAccount, () => {});
    const t = window.setInterval(() => tick((n) => n + 1), 30_000);
    return () => window.clearInterval(t);
  }, []);
  let failed: boolean;
  let working: boolean;
  let status: string;
  // The whole sentence, naming the app: the sidebar has room for its short form only, as
  // Mail's footer says "Updated Just Now" under the mailbox it is plainly about.
  let full: string | undefined;
  if (app) {
    failed = own.failure !== null && !own.working;
    working = own.working;
    status = working
      ? "Updating…"
      : failed
        ? own.failure!
        : own.unavailable
          ? "Not available"
          : freshness(own.lastSyncedAt);
    full = working
      ? `Updating ${app.name}…`
      : failed
        ? `${app.name} didn’t update: ${own.failure}`
        : own.unavailable
          ? `${app.name} isn’t available`
          : `${app.name} ${status.replace(/^\w/, (c) => c.toLowerCase())}`;
  } else {
    failed = !!(error || report?.services.some((s) => s.outcome === "failed"));
    working = syncing;
    status = syncing ? "Syncing…" : failed ? "Some things didn't update" : freshness(report?.asOf ?? null);
  }
  // Nothing synced yet, or the service is not there: the dot does not say all is well.
  const idle = app ? own.unavailable || own.lastSyncedAt === null : !report?.asOf;
  // The first name, as the menu bar greets someone: a full name did not fit the sidebar and
  // ended in "…". Settings shows it whole.
  const name = account?.name?.split(/\s+/)[0] ?? account?.email ?? "Settings";
  // Two quiet lines, each one thing, as Mail's sidebar ends: how fresh this app is — the line
  // itself the control that updates it, its dot turning while it does — and below it who is
  // signed in, the way into Settings.
  const asked = useRef(false);
  const refresh = () => {
    asked.current = true;
    void (app ? refreshService(app.service, true) : refreshAll(true));
  };
  // Said aloud, politely: how an update someone asked for ended, and any update that newly
  // failed. The routine updates in the background are left unsaid — a line every few minutes
  // would talk over whatever is being read.
  const [said, setSaid] = useState("");
  const was = useRef({ working, failed });
  useEffect(() => {
    const before = was.current;
    was.current = { working, failed };
    if (working) return;
    if ((before.working && asked.current) || (failed && !before.failed)) {
      asked.current = false;
      setSaid(full ?? status);
    }
  }, [working, failed]); // eslint-disable-line react-hooks/exhaustive-deps
  // One card, as a Mac draws the account at the foot of a sidebar: the picture, the name, and
  // under it how fresh this app is. The card opens Settings; the round button beside it
  // updates now. Updating, a thin ring turns around the picture (a transform only, drawn by
  // the compositor); in trouble, the line says what went wrong in red and a badge sits on the
  // picture.
  const state = failed ? "trouble" : working ? "working" : idle ? "idle" : "clean";
  return (
    <div className={`account-card is-${state}`}>
      <span className="visually-hidden" role="status">{said}</span>
      <button className="sidebar-account" aria-current={current} onClick={onOpen} title={notKept ? NOT_KEPT : `${full ?? status} · Skykeep`}>
        <span className="account-ring" aria-hidden="true">
          <AccountAvatar account={account} />
        </span>
        <span className="account-lines">
          <span className="sidebar-account-name">{name}</span>
          <span className="account-status">{notKept ? "Sign-in not saved" : status}</span>
        </span>
      </button>
      <button
        className="account-update"
        onClick={refresh}
        disabled={working}
        aria-busy={working}
        title={working ? full ?? status : "Update Now"}
        aria-label={working ? full ?? status : `Update now. ${full ?? status}`}
      >
        <RotateCw size={14} strokeWidth={2.2} aria-hidden="true" />
      </button>
    </div>
  );
}
