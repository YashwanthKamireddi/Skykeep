/**
 * The library by Years and by Months, as Photos on a Mac steps out of All Photos: a large tile a
 * year, then a tile a month, each with the photo that stands for it (chosen by the library, see
 * quince-core photo_months). A year opens its months; a month opens All Photos at that month.
 *
 * However large the library, these are a few hundred tiles at most, every one the same shape: laid
 * out from the width alone, so a jump to any one lands exactly, and only the rows near the screen
 * are made, their pictures fetched.
 */
import { t } from "../../lib/i18n";
import { dateLocale } from "../../lib/when";
import { useLayoutEffect, useMemo, useRef, useState, type MutableRefObject } from "react";
import { fitSrc, type PhotoMonth } from "../../lib/ipc";

export type Level = "years" | "months" | "all";
/** A place in the library carried from one level to the next: a year, or a month of it (1–12). */
export type Place = { year: number; month?: number };

/** "October 2026": the grid's own words for a month (PhotosView's monthOf), so the two agree. */
export const monthName = (year: number, month: number) =>
  new Date(year, month - 1, 15).toLocaleDateString(dateLocale(), { month: "long", year: "numeric" });

/** "212 Photos, 4 Videos", as the foot of the library counts. */
const tally = (count: number, videos: number) => {
  const part = (n: number, kind: "photo" | "video") => t(`photos.${kind}${n === 1 ? "One" : "Many"}`, { count: n.toLocaleString() });
  const stills = count - videos;
  if (stills && videos) return t("photos.tallyBoth", { photos: part(stills, "photo"), videos: part(videos, "video") });
  return stills ? part(stills, "photo") : part(videos, "video");
};

type Year = { year: number; count: number; videos: number; key: string };

/**
 * Each year's count and its key photo: the favourite standing for its busiest month that has one,
 * or else the key of its busiest month — the photo most likely to say what that year was.
 */
function yearsOf(months: PhotoMonth[]): Year[] {
  const out = new Map<number, Year & { best: PhotoMonth }>();
  for (const m of months) {
    const y = out.get(m.year);
    if (!y) {
      out.set(m.year, { year: m.year, count: m.count, videos: m.videos, key: m.key, best: m });
      continue;
    }
    y.count += m.count;
    y.videos += m.videos;
    const better = m.keyFavorite !== y.best.keyFavorite ? m.keyFavorite : m.count > y.best.count;
    if (better) {
      y.best = m;
      y.key = m.key;
    }
  }
  return [...out.values()].map(({ best: _best, ...y }) => y);
}

/** Each level's tiles: the narrowest a column may be, the gap between, and their shape. */
const LAYOUT = {
  years: { min: 360, gap: 16, ratio: 3 / 2 },
  months: { min: 232, gap: 12, ratio: 4 / 3 },
};
/** The room the floating bar takes at the top of the scroller (its scroll padding). */
const topRoom = (el: HTMLElement) => parseFloat(getComputedStyle(el).scrollPaddingTop) || 0;

/** The longer side each picture is made at, in device pixels: thumbnails, never originals. */
const YEAR_PX = 1024;
const MONTH_PX = 512;
const SMALL_PX = 256;

export function Periods({
  level,
  months,
  oldest,
  land,
  at,
  onYear,
  onMonth,
}: {
  level: "years" | "months";
  /** Newest first, as the library gives them. */
  months: PhotoMonth[];
  /** The library is sorted oldest first: so are its years and months. */
  oldest: boolean;
  /** Where to land as this level is shown (the place carried from the last one). */
  land: Place | null;
  /** The place at the top of the view now, kept current for a step to another level. */
  at: MutableRefObject<Place | null>;
  onYear: (year: number) => void;
  onMonth: (year: number, month: number) => void;
}) {
  const box = useRef<HTMLDivElement | null>(null);
  const years = useMemo(() => (level === "years" ? yearsOf(months) : []), [level, months]);
  const tiles = useMemo(() => {
    const list =
      level === "years"
        ? years.map((y) => ({ id: `${y.year}`, year: y.year, month: undefined as number | undefined, key: y.key, more: [] as string[], title: `${y.year}`, sub: tally(y.count, y.videos) }))
        : months.map((m) => ({ id: `${m.year}-${m.month}`, year: m.year, month: m.month as number | undefined, key: m.key, more: m.more, title: monthName(m.year, m.month), sub: tally(m.count, m.videos) }));
    return oldest ? list.reverse() : list;
  }, [level, years, months, oldest]);

  /*
   * Laid out here, not by the engine: every tile is the same shape, so its column, its row and
   * where it lies follow from the width alone, and only the rows near the screen are made. Left to
   * the engine (content-visibility and lazy pictures), WebKit made and fetched every tile at once —
   * 2.5 s on twenty years of months — and a tile off screen guessed at its height until drawn, so
   * a jump to a month landed a row out.
   */
  const shape = LAYOUT[level];
  const [width, setWidth] = useState(0);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(800);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return undefined;
    const measure = () => {
      const pad = parseFloat(getComputedStyle(el).paddingLeft) + parseFloat(getComputedStyle(el).paddingRight);
      if (el.clientWidth) setWidth(el.clientWidth - pad);
      setHeight(el.clientHeight);
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, []);
  const across = Math.max(1, Math.floor((width + shape.gap) / (shape.min + shape.gap)));
  const tileW = (width - (across - 1) * shape.gap) / across;
  const rowH = tileW / shape.ratio + shape.gap;
  const rows = Math.ceil(tiles.length / across);
  /** Where the row of tile `i` lies in the scroller: at its top, under the bar. */
  const rowTop = (i: number) => Math.floor(i / across) * rowH;
  // The rows on screen and a screen either side.
  const first = rowH > 0 ? Math.max(0, Math.floor(top / rowH) - Math.ceil(height / rowH)) : 0;
  const last = rowH > 0 ? Math.min(rows, Math.ceil((top + height) / rowH) + Math.ceil(height / rowH)) : 0;

  // The tile that takes Tab: the one landed on, then the one the arrows or a click moved to.
  const [current, setCurrent] = useState(0);
  const landing = land ? tiles.findIndex((t) => t.year === land.year && (land.month === undefined || level === "years" || t.month === land.month)) : -1;
  // A year landed on in Months: its first month in the order shown.
  const target = landing >= 0 ? landing : land ? tiles.findIndex((t) => t.year === land.year) : -1;

  const where = (i: number): Place | null => (tiles[i] ? { year: tiles[i].year, month: tiles[i].month } : null);
  /** A tile focused once it is made (it may be in a row not made yet). */
  const focusing = useRef<number | null>(null);
  const scrollTo = (y: number) => {
    const el = box.current;
    if (!el) return;
    el.scrollTop = y;
    // The rows there are made in this same frame, not a scroll event later.
    setTop(el.scrollTop);
  };

  // Landed on before the first frame: the tile's row at the top under the bar, and the tile
  // focused, so the keys go on from it. At the top when there is nowhere to land.
  useLayoutEffect(() => {
    if (!width) return;
    const i = Math.max(0, target);
    setCurrent(i);
    at.current = where(i);
    scrollTo(target < 0 ? 0 : rowTop(i));
    if (target >= 0) focusing.current = i;
    // Only on arriving at a level (and once it has a width): a sync leaves it be.
  }, [level, land, width > 0]); // eslint-disable-line react-hooks/exhaustive-deps
  useLayoutEffect(() => {
    const i = focusing.current;
    if (i === null) return;
    const tile = box.current?.querySelector<HTMLElement>(`[data-period="${CSS.escape(tiles[i]?.id ?? "")}"]`);
    if (!tile) return;
    focusing.current = null;
    tile.focus({ preventScroll: true });
  });

  // The rows to make follow the scroll at once; and the place, for a step to another level.
  const onScroll = () => {
    const el = box.current;
    if (!el) return;
    setTop(el.scrollTop);
    if (rowH <= 0) return;
    // The tile the keys rest on while its row is on screen; else the first of the top row.
    const y = rowTop(current);
    const seen = y > el.scrollTop - rowH / 2 && y < el.scrollTop + el.clientHeight - topRoom(el) - rowH / 2;
    at.current = where(seen ? current : Math.min(tiles.length - 1, Math.round(el.scrollTop / rowH) * across));
  };

  // The arrows move between tiles as laid out, as Photos' do; Home and End go to either end.
  const onKeyDown = (e: React.KeyboardEvent) => {
    const el = box.current;
    if (!el || e.altKey || e.ctrlKey || e.metaKey) return;
    const from = Math.min(current, tiles.length - 1);
    const step: Record<string, number> = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -across, ArrowDown: across };
    let to: number;
    if (e.key in step) to = from + step[e.key];
    else if (e.key === "Home") to = 0;
    else if (e.key === "End") to = tiles.length - 1;
    else return;
    e.preventDefault();
    // Down from a row with none under it: the last tile, as a short last row is reached.
    if (e.key === "ArrowDown" && to >= tiles.length && Math.floor(from / across) < Math.floor((tiles.length - 1) / across)) to = tiles.length - 1;
    if (to < 0 || to >= tiles.length) return;
    setCurrent(to);
    at.current = where(to);
    // Brought into view as little as it takes, as the grid's arrows do.
    const y = rowTop(to);
    if (y < el.scrollTop) scrollTo(y);
    else if (y + rowH - shape.gap > el.scrollTop + el.clientHeight - topRoom(el)) scrollTo(y + rowH - shape.gap - el.clientHeight + topRoom(el));
    focusing.current = to;
  };

  const dpr = window.devicePixelRatio || 1;
  const big = Math.min(level === "years" ? YEAR_PX : MONTH_PX, Math.round((level === "years" ? 560 : 320) * dpr));
  return (
    <div ref={box} className="px-periods" data-level={level} onScroll={onScroll} onKeyDown={onKeyDown}>
      <div
        className="px-periods-grid"
        role="group"
        aria-label={level === "years" ? t("photos.levels.years") : t("photos.levels.months")}
        data-count={tiles.length}
        style={{ "--across": across, "--gap": `${shape.gap}px`, height: Math.max(0, rows * rowH - shape.gap), paddingTop: first * rowH } as React.CSSProperties}
      >
        {tiles.slice(first * across, last * across).map((t, k) => {
          const i = first * across + k;
          return (
          <button
            key={t.id}
            className="px-period"
            data-period={t.id}
            data-mosaic={t.more.length >= 2 ? t.more.length : undefined}
            aria-label={`${t.title}, ${t.sub}`}
            // The tab stop is the current tile, or the first made when it is scrolled away.
            tabIndex={i === current || ((current < first * across || current >= last * across) && k === 0) ? 0 : -1}
            onFocus={() => {
              if (current !== i) setCurrent(i);
              at.current = where(i);
            }}
            onClick={() => (t.month === undefined ? onYear(t.year) : onMonth(t.year, t.month))}
          >
            <img className="px-period-key" src={fitSrc(t.key, big)} alt="" decoding="async" draggable={false} />
            {t.more.length >= 2 &&
              t.more.map((id) => <img key={id} src={fitSrc(id, SMALL_PX * Math.min(2, dpr))} alt="" decoding="async" draggable={false} />)}
            <span className="px-period-words" aria-hidden="true">
              <strong>{t.title}</strong>
              <span>{t.sub}</span>
            </span>
          </button>
          );
        })}
      </div>
    </div>
  );
}
