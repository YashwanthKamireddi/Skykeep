/**
 * A question before a change, asked one way everywhere: a sheet over the dimmed window — a
 * symbol when it helps, the question in bold, a line on what follows, and the answers. Two
 * answers sit side by side, Cancel first; more stack, the usual one on top, as macOS stacks
 * an alert's choices. Escape or a click outside is Cancel, and Cancel has the focus unless
 * another answer is the safe default, so Return never deletes by accident.
 *
 * It replaces the browser's own confirm() — a toolkit dialog in WebKitGTK, unlike anything else
 * in the window — and the four sheets Photos, Calendar and Settings each drew their own way.
 */
import { useCallback, useRef, useState } from "react";
import { EyeOff, Trash2 } from "lucide-react";
import { Portal } from "./Portal";
import { useModalFocus } from "../lib/focus";
import { useI18n } from "../lib/i18n";

type Choice<T> = {
  label: string;
  value: T;
  /** The filled button: what Return does. */
  primary?: boolean;
  /** Red: it destroys. */
  danger?: boolean;
  /** Where the focus starts. */
  focus?: boolean;
};

export function ConfirmSheet<T>({
  title,
  label,
  body,
  symbol,
  choices,
  onAnswer,
  onCancel,
  children,
}: {
  title: string;
  /** Its name for a screen reader, when the title is not it. */
  label?: string;
  body?: React.ReactNode;
  symbol?: "trash" | "hide";
  /** The answers besides Cancel, in order. */
  choices: Choice<T>[];
  onAnswer: (value: T) => void;
  onCancel: () => void;
  children?: React.ReactNode;
}) {
  const { t } = useI18n();
  const sheetRef = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheetRef);
  const stacked = choices.length > 1;
  const focusCancel = !choices.some((c) => c.focus);
  const cancel = (
    <button key="cancel" className="quiet" autoFocus={focusCancel} onClick={onCancel}>
      {t("common.cancel")}
    </button>
  );
  const answers = choices.map((c) => (
    <button
      key={c.label}
      className={`${c.primary ? "primary" : "quiet"}${c.danger ? " danger" : ""} confirm-answer`}
      autoFocus={c.focus}
      onClick={() => onAnswer(c.value)}
    >
      {c.label}
    </button>
  ));
  return (
    <Portal>
      <div className="modal-veil" onClick={onCancel}>
        <div
          ref={sheetRef}
          className="modal-sheet confirm"
          role="alertdialog"
          aria-modal="true"
          aria-label={label ?? title}
          onClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              onCancel();
            }
          }}
        >
          {symbol && (
            <span className="confirm-symbol" aria-hidden="true">
              {symbol === "trash" ? <Trash2 size={28} strokeWidth={1.6} /> : <EyeOff size={28} strokeWidth={1.6} />}
            </span>
          )}
          <h2>{title}</h2>
          {body && <p>{body}</p>}
          {children}
          <div className={`confirm-actions${stacked ? " is-stacked" : ""}`}>
            {stacked ? [...answers, cancel] : [cancel, ...answers]}
          </div>
        </div>
      </div>
    </Portal>
  );
}

type Question = {
  title: string;
  body?: string;
  /** The word on the button that goes ahead. */
  action: string;
  /** Red, for a change that destroys. */
  danger?: boolean;
  symbol?: "trash" | "hide";
};

/** `const [confirm, sheet] = useConfirm()`: `await confirm({...})` is true to go ahead. */
export function useConfirm(): [(q: Question) => Promise<boolean>, React.ReactNode] {
  const [asking, setAsking] = useState<Question | null>(null);
  const answer = useRef<((yes: boolean) => void) | null>(null);
  const confirm = useCallback(
    (q: Question) =>
      new Promise<boolean>((resolve) => {
        answer.current?.(false);
        answer.current = resolve;
        setAsking(q);
      }),
    [],
  );
  const done = (yes: boolean) => {
    answer.current?.(yes);
    answer.current = null;
    setAsking(null);
  };
  const sheet = asking && (
    <ConfirmSheet
      title={asking.title}
      body={asking.body}
      symbol={asking.symbol}
      choices={[{ label: asking.action, value: true, primary: true, danger: asking.danger }]}
      onAnswer={() => done(true)}
      onCancel={() => done(false)}
    />
  );
  return [confirm, sheet];
}
