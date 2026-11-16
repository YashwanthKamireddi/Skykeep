import { t } from "../lib/i18n";
import { useLayoutEffect, useRef, useState } from "react";
import { ArrowDownToLine, TriangleAlert } from "lucide-react";
import { fadeIn } from "../lib/fade";
import { api, messageOf, type UpdateStatus } from "../lib/ipc";
import { useTypingReport, useUpdateStatus } from "../lib/updates";
import { Progress } from "./Spinner";

/** The phases in which a restart to update is near enough that typing is reported. */
const NEAR = new Set<UpdateStatus["phase"]>(["available", "ready", "downloading", "waiting"]);

/**
 * A new version of Skykeep, said once, calmly, in the window in use: never in two windows at
 * once (the backend says which one shows it). Update and Restart downloads it with its
 * progress, waits for a moment nothing would be lost and says what it is waiting for, then
 * installs and restarts; Later puts it away in every window. Drawn among the toasts, at the
 * foot of the content.
 */
export function UpdateNotice() {
  const [status, setStatus] = useUpdateStatus();
  const [error, setError] = useState<string | null>(null);
  useTypingReport(Boolean(status && status.install === "here" && NEAR.has(status.phase)));
  if (!status?.prompt || !status.version) return null;
  const s = status;
  const name = `Skykeep ${s.version}`;

  const apply = () => {
    setError(null);
    setStatus({ ...s, phase: s.phase === "ready" ? "waiting" : "downloading", received: 0, total: null, problem: null });
    api.updateApply().catch((e) => setError(messageOf(e)));
  };
  const later = () => {
    setStatus({ ...s, prompt: false });
    api.updateLater().catch(() => {});
  };

  let line: string;
  let actions: "offer" | "later" | "retry" | "none" = "offer";
  switch (s.phase) {
    case "available":
      line = t("updates.available", { name });
      break;
    case "ready":
      line = t("updates.ready", { name });
      break;
    case "downloading":
      line = t("updates.downloading", { name });
      actions = "later";
      break;
    case "waiting":
      line =
        s.waitingFor === "typing"
          ? t("updates.afterTyping", { name })
          : t("updates.afterSync", { name });
      actions = "later";
      break;
    case "installing":
      line = t("updates.installing", { name });
      actions = "none";
      break;
    case "failed":
      line = s.problem ?? t("updates.failed");
      // A download that failed its signature is not offered again; anything else may be tried again.
      actions = /signature/.test(line) ? "later" : "retry";
      break;
    default:
      return null;
  }
  const failed = s.phase === "failed" || error !== null;
  const share = s.total ? s.received / s.total : null;

  return (
    <NoticeCard failed={failed} onEscape={actions === "none" ? undefined : later}>
      {failed ? (
        <TriangleAlert className="toast-glyph" size={15} strokeWidth={2} aria-hidden="true" />
      ) : (
        <ArrowDownToLine className="toast-glyph update-glyph" size={15} strokeWidth={2} aria-hidden="true" />
      )}
      <div className="update-body">
        <p className="update-line">{error ?? line}</p>
        {s.phase === "downloading" && (
          <div className="update-progress">
            <Progress value={share} label={t("updates.downloading", { name })} />
            {share !== null && <span className="update-percent">{Math.round(share * 100)}%</span>}
          </div>
        )}
        {s.notes && (s.phase === "available" || s.phase === "ready") && (
          <details className="update-notes">
            <summary>{t("updates.whatsNew")}</summary>
            <div className="update-notes-text">{s.notes}</div>
          </details>
        )}
        {actions !== "none" && (
          <div className="update-actions">
            <button className="quiet" onClick={later}>
              {actions === "offer" ? t("ui.later") : actions === "retry" ? t("common.notNow") : s.phase === "failed" ? t("common.ok") : t("ui.later")}
            </button>
            {actions === "offer" && (
              <button className="toast-action" onClick={apply}>
                {t("updates.updateRestart")}
              </button>
            )}
            {actions === "retry" && (
              <button className="toast-action" onClick={apply}>
                {t("updates.tryAgain")}
              </button>
            )}
          </div>
        )}
      </div>
    </NoticeCard>
  );
}

function NoticeCard({ failed, onEscape, children }: { failed: boolean; onEscape?: () => void; children: React.ReactNode }) {
  const card = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => fadeIn(card.current, { duration: 140 }), []);
  return (
    <div
      ref={card}
      className={`toast update-notice${failed ? " is-error" : ""}`}
      role={failed ? "alert" : "group"}
      aria-label={failed ? undefined : "Skykeep Update"}
      onKeyDown={(e) => {
        if (e.key === "Escape" && onEscape) {
          e.stopPropagation();
          onEscape();
        }
      }}
    >
      {children}
    </div>
  );
}
