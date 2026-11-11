import { useEffect, useState } from "react";
import { timeText } from "../lib/when";
import { composing } from "../lib/keys";

/**
 * A time of day, typed as a person says it — "9", "930", "9:30", "9.30pm", "21:15",
 * "noon" — and stepped with the arrows, fifteen minutes a press, an hour with Shift, as
 * macOS's time fields step. Shown in this machine's own clock, 12- or 24-hour.
 *
 * Linux's built-in time input is a row of tiny segments that each want their own keys: it
 * was where people gave up changing an event's time. A plain field takes what is typed and
 * keeps the last good time if it cannot read it.
 */
export function TimeField({
  hours,
  minutes,
  onChange,
  label,
  disabled = false,
  className = "",
}: {
  hours: number;
  minutes: number;
  onChange: (hours: number, minutes: number) => void;
  label: string;
  disabled?: boolean;
  className?: string;
}) {
  const shown = formatTime(hours, minutes);
  const [text, setText] = useState(shown);
  useEffect(() => setText(shown), [shown]);

  const commit = () => {
    const parsed = parseTime(text);
    if (parsed && (parsed[0] !== hours || parsed[1] !== minutes)) onChange(parsed[0], parsed[1]);
    else setText(shown);
  };
  const step = (by: number) => {
    const total = (((hours * 60 + minutes + by) % 1440) + 1440) % 1440;
    onChange(Math.floor(total / 60), total % 60);
  };

  return (
    <input
      type="text"
      inputMode="text"
      className={`time-field ${className}`.trim()}
      aria-label={label}
      value={text}
      disabled={disabled}
      readOnly={disabled}
      spellCheck={false}
      autoComplete="off"
      onChange={(e) => setText(e.target.value)}
      onFocus={(e) => e.currentTarget.select()}
      onBlur={commit}
      onKeyDown={(e) => {
        if (composing(e)) return;
        if (e.key === "Enter") {
          e.preventDefault();
          commit();
        } else if (e.key === "Escape") {
          setText(shown);
        } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
          e.preventDefault();
          step((e.key === "ArrowUp" ? 1 : -1) * (e.shiftKey ? 60 : 15));
        }
      }}
    />
  );
}

const usesTwelveHour = (() => {
  try {
    return new Intl.DateTimeFormat(undefined, { hour: "numeric" }).resolvedOptions().hour12 ?? false;
  } catch {
    return false;
  }
})();

export function formatTime(hours: number, minutes: number): string {
  const d = new Date(2000, 0, 1, hours, minutes);
  return timeText(d, { hour: "numeric", minute: "2-digit" });
}

/** `[hours, minutes]`, or null when the text is not a time. */
export function parseTime(raw: string): [number, number] | null {
  const t = raw.trim().toLowerCase().replace(/\s+/g, " ");
  if (!t) return null;
  if (t === "noon") return [12, 0];
  if (t === "midnight") return [0, 0];
  const m = t.match(/^(\d{1,2})(?:[:.h ]?(\d{2}))?\s*(a|am|a\.m\.|p|pm|p\.m\.)?$/);
  const compact = t.match(/^(\d{3,4})\s*(a|am|p|pm)?$/);
  let h: number;
  let min: number;
  let mer: string | undefined;
  if (compact && !m?.[2]) {
    const digits = compact[1];
    h = Number(digits.slice(0, digits.length - 2));
    min = Number(digits.slice(-2));
    mer = compact[2];
  } else if (m) {
    h = Number(m[1]);
    min = m[2] ? Number(m[2]) : 0;
    mer = m[3];
  } else {
    return null;
  }
  if (min > 59) return null;
  if (mer) {
    if (h < 1 || h > 12) return null;
    const pm = mer.startsWith("p");
    h = (h % 12) + (pm ? 12 : 0);
  } else if (h > 23) {
    return null;
  } else if (usesTwelveHour && h >= 1 && h <= 7) {
    // "3" in a 12-hour clock means the afternoon, as macOS reads it for an event.
    h += 12;
  }
  return [h, min];
}
