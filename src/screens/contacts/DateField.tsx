import { dateLocale, dateText } from "../../lib/when";
import { useEffect, useMemo, useState } from "react";
import { t } from "../../lib/i18n";
import { Picker } from "../../components/Picker";

/** The year Apple writes for a date given without one. */
export const NO_YEAR = 1604;

type Parts = { month: number | null; day: number | null; year: number | null };

function parse(value: string | null): Parts {
  const m = /^(\d{1,4})-(\d{2})-(\d{2})/.exec(value ?? "");
  if (!m) return { month: null, day: null, year: null };
  const year = Number(m[1]);
  return { month: Number(m[2]), day: Number(m[3]), year: year === NO_YEAR ? null : year };
}

const pad = (n: number, w: number) => String(n).padStart(w, "0");

/** As Apple writes it: `YYYY-MM-DD`, 1604 for no year, the day kept inside its month. */
function written({ month, day, year }: Parts): string | null {
  if (!month || !day) return null;
  const last = new Date(Date.UTC(year ?? 2000, month, 0)).getUTCDate();
  return `${pad(year ?? NO_YEAR, 4)}-${pad(month, 2)}-${pad(Math.min(day, last), 2)}`;
}

/** Month, day and year in the order this machine writes a date. */
function order(): ("month" | "day" | "year")[] {
  try {
    const parts = new Intl.DateTimeFormat(dateLocale(), { year: "numeric", month: "long", day: "numeric" })
      .formatToParts(new Date(2000, 10, 22))
      .map((p) => p.type)
      .filter((t): t is "month" | "day" | "year" => t === "month" || t === "day" || t === "year");
    return parts.length === 3 ? parts : ["month", "day", "year"];
  } catch {
    return ["month", "day", "year"];
  }
}

/**
 * A date as Contacts takes one — a birthday, an anniversary: a month, a day, and a year only
 * if it is known. Three plain fields rather than the browser's date input, which WebKitGTK
 * drew empty, and which cannot leave the year out.
 */
export function DateField({ value, onChange, label, autoFocus, compact }: {
  value: string | null;
  /** Months by their short names, for a date on one line of a card's details. */
  compact?: boolean;
  onChange: (value: string | null) => void;
  /** What the date is, for each field's name: "Birthday" gives "Birthday, day". */
  label: string;
  autoFocus?: boolean;
}) {
  const [parts, setParts] = useState<Parts>(() => parse(value));
  const [seen, setSeen] = useState(value);
  // A value changed from outside (another date picked, the card re-read) is taken up.
  useEffect(() => {
    if (value !== seen) {
      setSeen(value);
      if (written(parts) !== value) setParts(parse(value));
    }
  }, [value, seen, parts]);
  const months = useMemo(() => {
    const f = new Intl.DateTimeFormat(dateLocale(), { month: compact ? "short" : "long" });
    return Array.from({ length: 12 }, (_, i) => ({ value: String(i + 1), label: f.format(new Date(2000, i, 1)) }));
  }, [compact]);
  const set = (next: Parts) => {
    setParts(next);
    const v = written(next);
    setSeen(v);
    onChange(v);
  };
  const number = (key: "day" | "year", max: number, width: number) => (
    <input
      key={key}
      className={`date-field-${key}`}
      inputMode="numeric"
      autoComplete="off"
      placeholder={key === "day" ? t("contacts.day") : t("contacts.year")}
      aria-label={`${label}, ${key}`}
      value={parts[key] ?? ""}
      autoFocus={autoFocus && key === "day"}
      onChange={(e) => {
        const digits = e.target.value.replace(/\D/g, "").slice(0, width);
        const n = digits ? Math.min(Number(digits), max) : null;
        set({ ...parts, [key]: n && n > 0 ? n : null });
      }}
    />
  );
  const fields = {
    month: (
      <Picker
        key="month"
        className="date-field-month"
        label={t("contacts.fieldMonth", { label })}
        value={parts.month ? String(parts.month) : ""}
        onChange={(m) => set({ ...parts, month: m ? Number(m) : null })}
        options={[{ value: "", label: t("contacts.month") }, ...months]}
      />
    ),
    day: number("day", 31, 2),
    year: number("year", 9999, 4),
  };
  return <span className="date-field">{order().map((k) => fields[k])}</span>;
}

/** "17 March", or "17 March 1998" when the year is known — as a card shows a date. */
export function dateWords(raw: string): string {
  const { month, day, year } = parse(raw);
  if (!month || !day) return raw;
  const date = new Date(year ?? 2000, month - 1, day);
  return dateText(date, year ? { dateStyle: "long" } : { day: "numeric", month: "long" });
}
