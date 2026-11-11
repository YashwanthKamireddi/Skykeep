import { Fragment, useEffect, useRef } from "react";
import { useI18n } from "../lib/i18n";

/**
 * Six-digit verification code.
 *
 * Behaves the way people actually use these: typing advances, backspace on an empty box steps
 * back, arrows move, and pasting the whole code from anywhere fills every box at once.
 */
export function CodeInput({
  value,
  onChange,
  onComplete,
  disabled,
  invalid,
}: {
  value: string;
  onChange: (next: string) => void;
  onComplete: (code: string) => void;
  disabled?: boolean;
  invalid?: boolean;
}) {
  const { t } = useI18n();
  const boxes = useRef<(HTMLInputElement | null)[]>([]);

  const digits = value.padEnd(6, " ").slice(0, 6).split("");
  const firstEmpty = digits.findIndex((d) => !d.trim());
  // The box Tab should land on: the first empty one, or the last if the code is complete.
  const activeIndex = firstEmpty === -1 ? 5 : firstEmpty;

  // Into the first empty box whenever the boxes come back: on arrival, and after a code was
  // checked — they were disabled while it was, and a disabled box drops focus.
  useEffect(() => {
    if (!disabled) boxes.current[activeIndex]?.focus();
    // Only on enabling: following every keystroke would fight the arrows.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [disabled]);

  function setAt(index: number, digit: string) {
    const next = digits.map((d, i) => (i === index ? digit : d)).join("").trimEnd();
    onChange(next);
    return next;
  }

  /** Several digits at once — pasted, or filled by the system — written forward from `index`. */
  function fill(index: number, typed: string) {
    // A whole code always starts at the first box, wherever the caret was.
    const start = typed.length >= 6 ? 0 : index;
    const merged = (value.slice(0, start).replace(/\s/g, "") + typed).slice(0, 6);
    onChange(merged);
    boxes.current[Math.min(merged.length, 5)]?.focus();
    if (merged.length === 6) onComplete(merged);
  }

  function handleChange(index: number, raw: string) {
    let typed = raw.replace(/\D/g, "");
    if (!typed) return;
    // Typing over a box that already holds a digit, without it selected, gives both: keep
    // the new one.
    const old = digits[index].trim();
    if (typed.length === 2 && old && typed.startsWith(old)) typed = typed.slice(1);
    if (typed.length > 1) return fill(index, typed);
    const next = setAt(index, typed);
    if (index < 5) boxes.current[index + 1]?.focus();
    if (next.replace(/\s/g, "").length === 6) onComplete(next);
  }

  function handlePaste(index: number, e: React.ClipboardEvent<HTMLInputElement>) {
    // "481 516" or "481-516", as messages and password managers write it.
    const typed = e.clipboardData.getData("text").replace(/\D/g, "").slice(0, 6);
    e.preventDefault();
    if (typed) fill(index, typed);
  }

  function handleKeyDown(index: number, e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Backspace") {
      e.preventDefault();
      if (digits[index].trim()) {
        setAt(index, " ");
      } else if (index > 0) {
        setAt(index - 1, " ");
        boxes.current[index - 1]?.focus();
      }
    } else if (e.key === "ArrowLeft" && index > 0) {
      e.preventDefault();
      boxes.current[index - 1]?.focus();
    } else if (e.key === "ArrowRight" && index < 5) {
      e.preventDefault();
      boxes.current[index + 1]?.focus();
    }
  }

  return (
    <div
      className={`code${invalid ? " is-invalid" : ""}`}
      role="group"
      aria-label={t("codeEntry.codeAria")}
    >
      {digits.map((digit, i) => (
        <Fragment key={i}>
          {i === 3 && <span className="gap" aria-hidden="true" />}
          <input
            ref={(el) => {
              boxes.current[i] = el;
            }}
            value={digit.trim()}
            onChange={(e) => handleChange(i, e.target.value)}
            onKeyDown={(e) => handleKeyDown(i, e)}
            onPaste={(e) => handlePaste(i, e)}
            onFocus={(e) => e.target.select()}
            onMouseUp={(e) => e.preventDefault()}
            tabIndex={i === activeIndex ? 0 : -1}
            inputMode="numeric"
            autoComplete={i === 0 ? "one-time-code" : "off"}
            maxLength={6}
            disabled={disabled}
            aria-label={t("codeEntry.digitAria", { n: i + 1 })}
            aria-invalid={invalid || undefined}
          />
        </Fragment>
      ))}
    </div>
  );
}
