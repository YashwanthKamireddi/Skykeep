/**
 * Going to a month, as icloud.com's calendar button does: the years down the side, the chosen
 * year's months as tiles with a photo from each, in the calendar's order. A tile takes the grid
 * to that month. Only months the library has photos in are offered.
 */
import { t } from "../../lib/i18n";
import { dateText } from "../../lib/when";
import { useActiveRef } from "../../lib/screen";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { fitSrc } from "../../lib/ipc";
import { Portal } from "../../components/Portal";
import { grow } from "../../lib/fade";

export type MonthCover = { label: string; year: number; month: number; cover: { id: string } };

/** A month's short name in the reader's language (0 = January). */
const short = (m: number) => dateText(new Date(2023, m, 1), { month: "short" });

export function JumpTo({
  months,
  at,
  current,
  onPick,
  onClose,
}: {
  months: MonthCover[];
  /** The button it opened from. */
  at: DOMRect;
  /** The month at the top of the grid now. */
  current: string | null;
  onPick: (label: string) => void;
  onClose: () => void;
}) {
  const years = useMemo(() => [...new Set(months.map((m) => m.year))].sort((a, b) => b - a), [months]);
  const startYear = months.find((m) => m.label === current)?.year ?? years[0];
  const [year, setYear] = useState(startYear);
  const box = useRef<HTMLDivElement | null>(null);
  const shown = months.filter((m) => m.year === year).sort((a, b) => a.month - b.month);

  // Once, before its first frame is painted: run after it, the picker showed whole for a frame
  // and then began its fade from nothing — a flash. It grows out of the button that opened it.
  useLayoutEffect(() => grow(box.current, { x: at.left + at.width / 2, y: at.top + at.height / 2 }), []);
  const live = useActiveRef();
  useEffect(() => {
    box.current?.focus({ preventScroll: true });
    const down = (e: MouseEvent) => {
      if (!box.current?.contains(e.target as Node)) onClose();
    };
    const key = (e: KeyboardEvent) => {
      if (!live.current) return;
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    document.addEventListener("mousedown", down);
    window.addEventListener("keydown", key, true);
    return () => {
      document.removeEventListener("mousedown", down);
      window.removeEventListener("keydown", key, true);
    };
  }, [onClose]);

  const dpr = window.devicePixelRatio || 1;
  return (
    <Portal>
      <div
        ref={box}
        className="px-jump"
        role="dialog"
        aria-label={t("photos.goToMonth")}
        tabIndex={-1}
        style={{ top: at.bottom + 8, left: Math.max(8, Math.min(at.left - 120, window.innerWidth - 408)) }}
      >
        <div className="px-jump-years" role="tablist" aria-label={t("ui.years")}>
          {years.map((y) => (
            <button key={y} role="tab" aria-selected={y === year} className={`px-jump-year${y === year ? " is-on" : ""}`} onClick={() => setYear(y)}>
              {y}
            </button>
          ))}
        </div>
        <div className="px-jump-body">
          <h2 className="px-jump-title">{year}</h2>
          <div className="px-jump-months">
            {shown.map((m) => (
              <button
                key={m.label}
                className={`px-jump-month${m.label === current ? " is-here" : ""}`}
                aria-label={m.label}
                onClick={() => {
                  onPick(m.label);
                  onClose();
                }}
              >
                <img src={fitSrc(m.cover.id, 128 * dpr)} alt="" decoding="async" draggable={false} />
                <span>{short(m.month)}</span>
              </button>
            ))}
          </div>
        </div>
      </div>
    </Portal>
  );
}
