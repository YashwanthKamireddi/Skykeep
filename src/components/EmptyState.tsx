import { useArriving } from "../lib/sync";
import { Spinner } from "./Spinner";

/**
 * What a pane says when it has nothing to show, said one way in every app: a symbol, one line,
 * and the one thing to do next when there is one — centred in the pane. While something is on
 * its way it turns into the same line with a spinner, in the same place, so nothing jumps
 * when it lands.
 */
export function EmptyState({
  icon,
  busy,
  action,
  className,
  children,
}: {
  icon?: React.ReactNode;
  /** On its way: a spinner, announced politely. */
  busy?: boolean;
  action?: { label: string; run: () => void; disabled?: boolean };
  /** A screen's own hook for it (tests, layout), never its look. */
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div
      className={`empty-state${className ? ` ${className}` : ""}`}
      role={busy ? "status" : undefined}
      aria-live={busy ? "polite" : undefined}
    >
      {busy ? <Spinner /> : icon && <span className="empty-state-icon" aria-hidden="true">{icon}</span>}
      <p>{children}</p>
      {action && (
        <button className="primary" onClick={action.run} disabled={action.disabled}>
          {action.label}
        </button>
      )}
    </div>
  );
}

/**
 * A first sync, said where the content will be: what is coming, and that it is moving. Shown in
 * place of an empty state until the service has landed once, and then never again. `text` says
 * it in the interface's language; an app that gives none says it in English, naming `what`.
 */
export function Arriving({ service, what, text, className }: { service: string; what: string; text?: string; className?: string }) {
  if (!useArriving(service)) return null;
  return (
    <EmptyState busy className={`arriving${className ? ` ${className}` : ""}`}>
      {text ?? `Getting your ${what} from iCloud…`}
    </EmptyState>
  );
}
