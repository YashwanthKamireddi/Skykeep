/**
 * Photos, as on a Mac: the library edge to edge, newest first, grouped by month, with a zoom
 * for how many across; a photo opens on a dark stage with the arrows to go through them. Everything draws from the local library at once; thumbnails come
 * from the disk cache and are fetched once.
 */
import { AppFoot } from "../components/AppFoot";
import { useActiveRef, useScreenActive } from "../lib/screen";
import { putAway, watchNear } from "../lib/near";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { pickOpen } from "../lib/pick";
import { ChevronLeft, ChevronRight, Ellipsis, GalleryThumbnails, CopyMinus, CopyPlus, Download, Clock, Heart, ImagePlus, Images, Info, Minus, Play, Plus, RotateCcw, Scan, Trash2, EyeOff, Lock, ArrowDownUp, MonitorPlay, Square, RectangleHorizontal, Star, Layers, SquareStack, Folder as FolderIcon, Aperture, Video, X, CalendarDays, Maximize2, Minimize2, Users, type LucideIcon } from "lucide-react";
import {
  api,
  messageOf,
  pictureSrc,
  fitSrc,
  PHOTO_UPLOAD,
  type Photo,
  type PhotoAlbum,
  type PhotoMonth,
  type PhotoOrder,
  type PhotoLibrary,
  type PhotoSource,
  type PhotoView,
  type UploadEvent,
  turnStyle,
  type Turn,
} from "../lib/ipc";
import { Arriving, EmptyState } from "../components/EmptyState";
import { SearchField } from "../components/SearchField";
import { PaneTitle } from "../components/PaneTitle";
import { useArriving, useRevision, useServiceState } from "../lib/sync";
import { dismissToast, toast, toastSaved, useErrorToast, useToastClearance } from "../components/Toast";
import { UploadPanel } from "./photos/UploadPanel";
import { RowMenu, type MenuEntry } from "../components/RowMenu";
import { Portal } from "../components/Portal";
import { useModalFocus } from "../lib/focus";
import { Slideshow } from "./photos/Slideshow";
import { useConfirm } from "../components/Confirm";
import { VideoPlayer } from "./photos/VideoPlayer";
import { JumpTo, type MonthCover } from "./photos/JumpTo";
import { SharedAlbums } from "./photos/SharedAlbums";
import { tintFrom } from "../lib/tint";
import { fadeIn } from "../lib/fade";
import { DownloadOptions, type Want } from "./photos/DownloadOptions";
import { AdjustDate } from "./photos/AdjustDate";
import { useReveal } from "../lib/reveal";
import { Picker } from "../components/Picker";
import { PAGE, PhotoList, PhotoStore } from "./photos/list";
import { Periods, monthName, type Level, type Place } from "./photos/Periods";
import { same } from "../lib/same";
import { carryPicture } from "../lib/dragPicture";

/** The library switcher's choices, named as Photos names them. */
const LIBRARIES = [
  { value: "both", label: "Both Libraries" },
  { value: "personal", label: "Personal Library" },
  { value: "shared", label: "Shared Library" },
];

/** The sidebar, in Photos' order, each with the symbol Photos gives it. */
const VIEWS: { key: PhotoView; label: string; Icon: LucideIcon }[] = [
  { key: "library", label: "Library", Icon: Images },
  { key: "favorites", label: "Favorites", Icon: Heart },
  { key: "recents", label: "Recents", Icon: Clock },
];
/** Photos' Media Types: the library by what kind of picture each is. */
const MEDIA: typeof VIEWS = [
  { key: "videos", label: "Videos", Icon: Video },
  { key: "portrait", label: "Portrait", Icon: Aperture },
  { key: "panoramas", label: "Panoramas", Icon: RectangleHorizontal },
  { key: "screenshots", label: "Screenshots", Icon: Scan },
];
/** Photos' Utilities: what is kept out of the library. */
const UTILITIES: typeof VIEWS = [
  { key: "rated", label: "Rated", Icon: Star },
  { key: "hidden", label: "Hidden", Icon: EyeOff },
  { key: "recentlyDeleted", label: "Recently Deleted", Icon: Trash2 },
];

/** The Library's three levels, as Photos names them in its toolbar. */
const LEVELS: [Level, string][] = [["years", "Years"], ["months", "Months"], ["all", "All Photos"]];
const LEVEL_KEY = "quince.photos.level";

/** How many across at each step of the zoom, as Photos steps it. */
const ACROSS = [10, 7, 5, 3];

const monthOf = (seconds: number) =>
  new Date(seconds * 1000).toLocaleDateString(undefined, { month: "long", year: "numeric" });
/** "Sunday, 18 October", with the year when it is not this one: a day's heading, as Photos' Days. */
const dayOf = (seconds: number, thisYear: number) => {
  const d = new Date(seconds * 1000);
  return d.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long", ...(d.getFullYear() === thisYear ? {} : { year: "numeric" }) });
};
/** From this step of the zoom in (five across and closer), the library is grouped by day. */
const DAYS_FROM = 2;

/** What Photos takes, as icloud.com's picker offers it. */
const KINDS = ["jpg", "jpeg", "heic", "heif", "png", "gif", "tif", "tiff", "webp", "dng", "mov", "mp4", "m4v", "avi"];

/** Photos' album symbol: a card with the cards behind it showing above, as a stack. */
const AlbumGlyph = () => (
  <svg className="px-glyph" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
    <rect x="3" y="9" width="18" height="12" rx="2.5" />
    <path d="M5.5 5.5h13M8 2.5h8" />
  </svg>
);

/** "3 Photos Selected", "1 Video Selected", "5 Items Selected" — Photos' words. */
const pickedWords = (kinds: Photo["kind"][]) => {
  const videos = kinds.filter((k) => k === "video").length;
  const n = kinds.length;
  const noun = videos === 0 ? "Photo" : videos === n ? "Video" : "Item";
  return `${n.toLocaleString()} ${n === 1 ? noun : `${noun}s`} Selected`;
};
/** "340 photos", "12 videos", "352 items" — what a batch being received holds. */
const receivingWords = (n: number, videos: number) => {
  const noun = videos === 0 ? "photo" : videos === n ? "video" : "item";
  return `${n.toLocaleString()} ${n === 1 ? noun : `${noun}s`}`;
};

type Flag = "favorite" | "deleted" | "hidden" | "expunged";

/** A coordinate as a map gives it: 17.7654° N. */
const degrees = (v: number, plus: string, minus: string) => `${Math.abs(v).toFixed(4)}° ${v >= 0 ? plus : minus}`;

/** How a view is sorted, as Photos offers it: by the date taken or the date added, either way. */
type Sort = { by: "taken" | "added"; oldest: boolean };
const SORTS_KEY = "quince.photos.sort";
const defaultSort = (view: PhotoView): Sort => ({ by: view === "recents" ? "added" : "taken", oldest: false });
const sortKey = (p: Photo, by: Sort["by"]) => (by === "added" ? (p.addedAt ?? p.takenAt) : p.takenAt);
const readSorts = (): Partial<Record<PhotoView, Sort>> => {
  try {
    return JSON.parse(window.localStorage.getItem(SORTS_KEY) ?? "{}") ?? {};
  } catch {
    return {};
  }
};

/**
 * How long a photo in Recently Deleted has left, in Photos' words: "29 days left" (or, on a
 * tile, "29 days"), then hours, then minutes; red under four days, as Photos turns it.
 */
const timeLeft = (p: Photo, compact: boolean) => {
  if (!p.expungeAt) return null;
  const ms = Math.max(0, p.expungeAt * 1000 - Date.now());
  const [unit, size] = ms >= 86_400_000 ? ["day", 86_400_000] : ms >= 3_600_000 ? ["hour", 3_600_000] : ["minute", 60_000];
  const n = Math.max(1, Math.floor(ms / size));
  return { words: `${n} ${unit}${n === 1 ? "" : "s"}${compact ? "" : " left"}`, soon: ms < 4 * 86_400_000 };
};

/** A video's length as Photos writes it on its tile: "0:07", "4:32", "1:02:05". */
const clock = (s: number | null) => {
  if (!s) return "";
  // Whole seconds first: the seconds rounded on their own read 59.6 s as "0:60", and an
  // hour-long video ran on in minutes ("75:03"). A clip under half a second is still "0:01".
  const t = Math.max(1, Math.round(s));
  const two = (n: number) => String(n).padStart(2, "0");
  const [h, m, sec] = [Math.floor(t / 3600), Math.floor((t % 3600) / 60), t % 60];
  return h ? `${h}:${two(m)}:${two(sec)}` : `${m}:${two(sec)}`;
};

/** A photo's frame when fitted: its own shape, as large as the square cell lets it be. */
const fitFrame = (p: Photo): React.CSSProperties =>
  p.width >= p.height
    ? { width: "100%", height: `${(100 * p.height) / p.width}%` }
    : { height: "100%", width: `${(100 * p.width) / p.height}%` };

/**
 * The viewer's filmstrip, as icloud.com's: the photos either side as narrow slips, the one shown
 * wider in the middle; a click goes to it. Only the sixty or so nearest are built, and the
 * strip moves at once to keep the one shown in the middle (animated, it flickered on his screen).
 */
const SLIP = 26;
const SHOWN = 54;
const GAP = 2;
const REACH = 30;
function Filmstrip({ photos, at, onPick }: { photos: PhotoList; at: number; onPick: (index: number) => void }) {
  // Enough either side to fill the window's width and more, so a slip is always built (and its
  // picture decoded) before it can be seen: built at the edge of the screen, slips came in
  // blank and filled in as each step was taken.
  const reach = Math.max(REACH, Math.ceil(window.innerWidth / (SLIP + GAP) / 2) + 12);
  const from = Math.max(0, at - reach);
  const to = Math.min(photos.length - 1, at + reach);
  // Each slip has its place by its index in the whole library, so the slips built as the
  // window moves along never jump; the strip is placed to put the shown one's centre in the middle.
  const shift = -(at * (SLIP + GAP) + SHOWN / 2);
  // The slips' photos on their way; one not here yet holds its place as an empty slip.
  useEffect(() => void photos.need(from, to), [photos, from, to]);
  const slips = [];
  for (let i = from; i <= to; i++) {
    const p = photos.at(i);
    const place = { transform: `translateX(${i * (SLIP + GAP) + (i > at ? SHOWN - SLIP : 0)}px)` };
    if (!p) {
      slips.push(<span key={photos.ids[i]} className="px-slip is-waiting" style={place} aria-hidden="true" />);
      continue;
    }
    slips.push(
      <button
        key={p.id}
        data-id={p.id}
        className={`px-slip${i === at ? " is-shown" : ""}`}
        // Placed by transform, eased with its neighbours: the shown one widens as they make
        // room, all on the compositor — before, each step jumped every slip into place.
        style={{ transform: `translateX(${i * (SLIP + GAP) + (i > at ? SHOWN - SLIP : 0)}px)` }}
        aria-label={p.filename ?? "Photo"}
        aria-current={i === at}
        onClick={() => i !== at && onPick(i)}
      >
        {/* At once, never lazily: a slip coming into the strip showed empty for a moment. */}
        {/* One size for every slip, shown or not, so a step never asks for a new picture; decoded
            before it is drawn (they are small), so none appears blank first. */}
        <img src={fitSrc(p, SHOWN * 2 * (window.devicePixelRatio || 1))} alt="" decoding="async" draggable={false} />
      </button>,
    );
  }
  // A wheel or a two-finger swipe over the strip skims through the photos, as in Photos: one
  // photo every 40 pixels of scroll, a fling several.
  const spun = useRef(0);
  const onWheel = (e: React.WheelEvent) => {
    const d = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
    spun.current += e.deltaMode === 1 ? d * 40 : d;
    const by = Math.trunc(spun.current / 40);
    if (!by) return;
    spun.current -= by * 40;
    const to = Math.max(0, Math.min(photos.length - 1, at + by));
    if (to !== at) onPick(to);
  };
  return (
    <div className="px-strip" role="toolbar" aria-label="Photos" onWheel={onWheel}>
      <div className="px-strip-run" style={{ transform: `translateX(${shift}px)` }}>
        {slips}
      </div>
    </div>
  );
}

/**
 * The pictures the viewer shows — each made the size it is shown at (see fitSrc) — already
 * fetched and decoded, by address. A photo whose picture is in here opens and is stepped to at
 * full quality at once, never a smaller copy first.
 */
const decoded = new Set<string>();
/**
 * Photos a sync has just brought into the view on screen, with when: each one's tile fades its
 * picture in as it lands (see Tile), and nothing else moves. Only for a moment — a month built
 * later, as the grid is scrolled to it, draws its photos as it always does.
 */
const arrived = new Map<string, number>();
/** More than this many at once is a batch (an import, a first sync): they simply appear. */
const ARRIVALS = 12;
/** How long an arrival is one: its tile is made within a frame or two of it being drawn. */
const ARRIVAL_MS = 1500;
/** "Receiving 340 photos" stays at least this long, to be read, and at most this long. */
const RECEIVING_SHOWN = 1800;
const RECEIVING_MOST = 30_000;
/** Whether a tile being made now is for a photo that has just arrived. */
function justArrived(id: string): boolean {
  const at = arrived.get(id);
  return at !== undefined && performance.now() - at < ARRIVAL_MS;
}

/**
 * The tile at the top of the view and where it stands, for keeping it there while photos land
 * above it (see PhotosView). The grid's tiles are in the page in the order they are drawn, so
 * the first one reaching below the header is found in a few steps, not by measuring them all.
 */
function anchorOf(el: HTMLElement | null): { id: string; top: number } | null {
  if (!el || el.scrollTop <= 0 || !el.clientHeight) return null;
  const tiles = el.querySelectorAll<HTMLElement>(".px-tile");
  if (!tiles.length) return null;
  const edge = el.getBoundingClientRect().top + (parseFloat(getComputedStyle(el).paddingTop) || 0);
  let [lo, hi] = [0, tiles.length - 1];
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (tiles[mid].getBoundingClientRect().bottom > edge) hi = mid;
    else lo = mid + 1;
  }
  const tile = tiles[lo];
  const id = tile.dataset.id;
  return id ? { id, top: tile.getBoundingClientRect().top } : null;
}

/** The rest on a tile before its picture is fetched (see Tile). */
let resting: number | undefined;
const warming = new Map<string, Promise<boolean>>();
/** The viewer's stage, in CSS pixels, as last measured (a guess from the window before). */
let stageBox = { w: 0, h: 0 };
/** The picture the viewer shows for a photo: the size its stage draws it at. */
function viewSrc(p: Photo): string {
  const box = stageBox.w ? stageBox : { w: Math.max(320, window.innerWidth - 400), h: Math.max(240, window.innerHeight - 140) };
  const r = p.width && p.height ? p.width / p.height : 4 / 3;
  const w = Math.min(box.w, box.h * r, (p.width || 4000) * 1.5);
  return fitSrc(p, Math.max(w, w / r) * (window.devicePixelRatio || 1));
}
/**
 * A small picture of a photo the window has already got and decoded — the grid's tile or the
 * strip's slip, whichever is on the page — to paint under the viewer's picture until that is
 * decoded. Apple's thumbnail by its own address was a picture not yet fetched: on opening, the
 * stage showed empty (black in the dark) for the frames it took to come.
 */
function loadedThumb(id: string): string {
  const esc = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(id) : id;
  for (const sel of [`.px-slip[data-id="${esc}"] img`, `.px-tile[data-id="${esc}"] img`]) {
    const img = document.querySelector<HTMLImageElement>(sel);
    if (img && img.complete && img.naturalWidth) return img.currentSrc || img.src;
  }
  return pictureSrc(id, "thumb");
}
function warm(p: Photo): Promise<boolean> {
  const url = viewSrc(p);
  if (decoded.has(url)) return Promise.resolve(true);
  let ready = warming.get(url);
  if (!ready) {
    const img = new Image();
    img.decoding = "async";
    img.src = url;
    ready = img
      .decode()
      .then(
        () => {
          decoded.add(url);
          return true;
        },
        () => false,
      )
      .finally(() => warming.delete(url));
    warming.set(url, ready);
  }
  return ready;
}

/** A tile's picture: the size it is drawn at — the whole cell when fitted, cropped to fill it
 *  as a square (so its longer side reaches past the cell). */
function tileSrc(p: Photo, cell: number, fit: boolean): string {
  const r = p.width && p.height ? Math.max(p.width, p.height) / Math.min(p.width, p.height) : 1;
  return fitSrc(p, fit ? cell : cell * r);
}

/** What a tile can ask of the screen; one object, kept the same, so tiles need not redraw. */
type Acts = {
  pick: (index: number, e: React.MouseEvent) => void;
  open: (index: number) => void;
  menu: (photo: Photo, e: React.MouseEvent) => void;
  drag: (photo: Photo, e: React.DragEvent) => void;
  dragEnd: () => void;
};

/**
 * One photo in the grid. It redraws only when its own photo, its selection or its place
 * changes: a click that selects one of twenty thousand redraws two tiles, not all of them.
 */
const Tile = memo(function Tile({
  photo,
  index,
  selected,
  tabbable,
  left,
  soon,
  fit,
  cell,
  pics,
  act,
}: {
  photo: Photo;
  index: number;
  selected: boolean;
  tabbable: boolean;
  left: string | null;
  soon: boolean;
  fit: boolean;
  /** A cell's width in device pixels. */
  cell: number;
  /** Its month's: whether it is near the view, so its picture is drawn (see Month: pics). */
  pics: { current: boolean };
  act: Acts;
}) {
  const born = useRef(performance.now());
  // Just arrived from the phone: its picture fades in once, briefly — the one thing that moves.
  const arriving = useRef<boolean | null>(null);
  if (arriving.current === null) arriving.current = justArrived(photo.id);
  // The thumbnail goes under the tile's picture only when that picture cannot be made, or is
  // still not here after a long moment (made from Apple, not the disk). Nearly every picture is made ahead (photos_prepare) and lands at once, and
  // the thumbnail asked for alongside it doubled each tile's requests — every one a trip through
  // the webview's scheme handler — and its bytes, ~69 KB of thumbnail against ~19 KB of
  // picture, decoded and then covered. Set on the element itself: no tile draws again for it.
  const pic = useRef<HTMLSpanElement | null>(null);
  const landed = useRef(false);
  const img = useRef<HTMLImageElement | null>(null);
  const under = useCallback(() => {
    // Only under a picture that is drawn: a tile far from the view keeps neither (see Month).
    if (!landed.current && pic.current && img.current?.hasAttribute("src"))
      pic.current.style.backgroundImage = `url("${pictureSrc(photo.id, "thumb")}")`;
  }, [photo.id]);
  const src = tileSrc(photo, cell, fit);
  const first = useRef(true);
  // Its picture drawn at once when its month is near as it is built, as every tile always was;
  // a new size asked for (the window resized, zoomed) is drawn in its place. Drawn and let go
  // otherwise by its month, on the element itself, so no tile draws again for it (see Month).
  useLayoutEffect(() => {
    const el = img.current;
    if (!el) return;
    if (el.hasAttribute("src") || pics.current) el.src = src;
  }, [src, pics]);
  useEffect(() => {
    // A new size asked for (the window resized, zoomed): the thumbnail may be wanted again.
    if (!first.current) landed.current = false;
    first.current = false;
    if (landed.current) return undefined;
    // Long enough that a grid filling in from the disk never asks for it — a full grid's worth
    // of pictures through the scheme handler at once takes a while, and thumbnails asked for
    // then only queued behind them — short enough to cover a picture made from the network.
    const t = window.setTimeout(under, 1500);
    return () => window.clearTimeout(t);
  }, [src, under]);
  return (
    <button
      // A photo is chosen as an option in a listbox is: aria-selected belongs to an option.
      role="option"
      className="px-tile"
      aria-label={`${photo.filename ?? "Photo"}, ${new Date(photo.takenAt * 1000).toLocaleString()}${
        photo.burst && (photo.burstCount ?? 0) > 1 ? `, burst of ${photo.burstCount} photos` : ""
      }`}
      data-id={photo.id}
      data-index={index}
      aria-selected={selected}
      tabIndex={tabbable ? 0 : -1}
      onClick={(e) => {
        // The keys follow the click, as in Photos: WebKit does not focus a button it clicks,
        // and the arrows went on to whatever had the focus before — the app list, which they
        // moved through instead of the photos.
        e.currentTarget.focus({ preventScroll: true });
        act.pick(index, e);
      }}
      onDoubleClick={() => act.open(index)}
      // A pointer that rests on a photo starts its whole picture on the way, so opening it is
      // instant: by the double-click it is decoded. Only after a moment's rest — passing over
      // the grid fetches nothing.
      onPointerEnter={() => {
        if (photo.kind === "video") return;
        window.clearTimeout(resting);
        resting = window.setTimeout(() => void warm(photo), 140);
      }}
      onPointerLeave={() => window.clearTimeout(resting)}
      onPointerDown={() => {
        if (photo.kind !== "video") void warm(photo);
      }}
      draggable
      onDragStart={(e) => act.drag(photo, e)}
      onDragEnd={act.dragEnd}
      onContextMenu={(e) => {
        e.preventDefault();
        act.menu(photo, e);
      }}
    >
      {/* The picture's own frame: the whole tile when squares, the photo's own shape when
          fitted — what the badges and the selection sit on. */}
      <span
        ref={pic}
        className="px-pic"
        // The thumbnail underneath while the picture made for this tile's size comes (see under).
        style={fit && photo.width && photo.height ? fitFrame(photo) : undefined}
      >
        {/* Drawn once its month is within a screen of the view — not left to the engine's own
            lazy margin, which a fast scroll outran — and made the size it is drawn at, so the
            engine never shrinks it. Let go again two screens off (see Month: pics). */}
        <img
          ref={img}
          data-src={src}
          alt=""
          decoding="async"
          draggable={false}
          // Already made (as it nearly always is, see photos_prepare): there at once. Still on
          // its way: it fades in over the thumbnail rather than snapping from soft to sharp.
          // Never made: the thumbnail stays.
          onLoad={(e) => {
            landed.current = true;
            e.currentTarget.dataset.in = performance.now() - born.current > 80 ? "fade" : "now";
            if (arriving.current) {
              arriving.current = false;
              fadeIn(e.currentTarget, { duration: 120 });
            }
          }}
          onError={(e) => {
            e.currentTarget.dataset.gone = "";
            under();
          }}
        />
        {photo.kind === "video" && (
          <span className="px-badge px-duration">
            <Play size={10} strokeWidth={0} fill="currentColor" aria-hidden="true" />
            {clock(photo.duration)}
          </span>
        )}
        {/* A burst is one photo in the grid, as in Photos: its chosen shot, marked as a stack
            with how many it holds, so it is known for one before it is opened. */}
        {photo.burst && (photo.burstCount ?? 0) > 1 && (
          <span className="px-badge px-burst">
            <SquareStack size={11} strokeWidth={2.2} aria-hidden="true" />
            {photo.burstCount}
          </span>
        )}
        {left && <span className={`px-badge px-left${soon ? " is-soon" : ""}`}>{left}</span>}
        {(photo.rating ?? 0) > 0 && <span className="px-stars" aria-label={`${photo.rating} stars`}>{photo.rating}★</span>}
        {photo.favorite && <Heart className="px-heart" size={13} strokeWidth={0} fill="currentColor" aria-label="Favorite" />}
      </span>
    </button>
  );
});

/**
 * A month of the grid. Only months near the view are built: one further off is an empty
 * block exactly as tall as its rows will be (worked out in CSS from the grid's own width),
 * so a library of tens of thousands opens at once and scrolls without the bar jumping.
 */
/** The ids whose selection differs between two selections, worked out once per pair. */
const diffs = new WeakMap<Set<string>, { from: Set<string>; changed: string[] }>();
function changedBetween(from: Set<string>, to: Set<string>): string[] {
  const known = diffs.get(to);
  if (known && known.from === from) return known.changed;
  const changed: string[] = [];
  for (const id of from) if (!to.has(id)) changed.push(id);
  for (const id of to) if (!from.has(id)) changed.push(id);
  diffs.set(to, { from, changed });
  return changed;
}
/** A month's photo ids, made once per list of them. */
const monthIds = new WeakMap<object, Set<string>>();
function idsOf(items: Item[]): Set<string> {
  let ids = monthIds.get(items);
  if (!ids) {
    ids = new Set(items.map((x) => x.id));
    monthIds.set(items, ids);
  }
  return ids;
}

/**
 * Whether a month draws the same as before: everything it was given is the same, except a
 * selection or a focused tile that changed somewhere else. One click used to draw every built
 * month again — every one was handed the whole selection — 166 ms on a 20,000-photo library.
 */
function sameMonth(a: MonthProps, b: MonthProps): boolean {
  if (a.label !== b.label || a.view !== b.view || a.month !== b.month || a.items !== b.items || a.list !== b.list || a.deleted !== b.deleted || a.eager !== b.eager || a.across !== b.across
    || a.fit !== b.fit || a.cell !== b.cell || a.act !== b.act) return false;
  if (a.chosen !== b.chosen) {
    // Only the photos whose selection changed are looked at (worked out once per change, not
    // once per month), each against this month's own photos.
    const changed = changedBetween(a.chosen, b.chosen);
    const mine = idsOf(a.items);
    for (const id of changed) if (mine.has(id)) return false;
  }
  if (a.anchor !== b.anchor) {
    const first = a.items[0]?.index ?? 0;
    const last = a.items[a.items.length - 1]?.index ?? -1;
    const here = (n: number | null) => n !== null && n >= first && n <= last;
    // The tile that takes Tab is the anchor's, or the first of all when there is none.
    if (here(a.anchor) || here(b.anchor) || ((a.anchor === null) !== (b.anchor === null) && first === 0)) return false;
  }
  return true;
}

/** Where a month's photos lie — its grid and whatever of it is still to be built — on screen. */
function photosArea(sec: HTMLElement): { top: number; height: number } {
  const first = sec.querySelector<HTMLElement>(":scope > .px-grid, :scope > .px-later") ?? sec;
  const top = first.getBoundingClientRect().top;
  return { top, height: sec.getBoundingClientRect().bottom - top };
}

/**
 * Months on their way are built a slice a frame, one month after another, in the order they
 * came near: the work a fast scroll asks for is spread over frames instead of landing in one.
 */
const BUILD_TILES = 28;
const builds: (() => boolean)[] = [];
let buildFrame = 0;
function buildSoon(job: () => boolean) {
  builds.push(job);
  if (!buildFrame) buildFrame = requestAnimationFrame(buildNext);
}
function buildNext() {
  buildFrame = 0;
  const job = builds[0];
  if (!job) return;
  if (job()) builds.shift();
  if (builds.length) buildFrame = requestAnimationFrame(buildNext);
}

/** A place in the grid: the photo there, by id, and where it is in the whole list. */
type Item = { id: string; index: number };

type MonthProps = {
  label: string;
  /** The view it is drawn in (see Month's `whole`). */
  view: string;
  month: string;
  items: Item[];
  list: PhotoList;
  chosen: Set<string>;
  anchor: number | null;
  deleted: boolean;
  eager: boolean;
  across: number;
  fit: boolean;
  cell: number;
  act: Acts;
};

const Month = memo(function Month({
  label,
  view,
  month,
  items,
  list,
  chosen,
  anchor,
  deleted,
  eager,
  across,
  fit,
  cell,
  act,
}: {
  label: string;
  view: string;
  month: string;
  items: Item[];
  list: PhotoList;
  chosen: Set<string>;
  anchor: number | null;
  deleted: boolean;
  eager: boolean;
  across: number;
  fit: boolean;
  cell: number;
  act: Acts;
}) {
  const ref = useRef<HTMLElement | null>(null);
  // How many of its photos are built: all at once for the first months, and for the rest a few
  // rows a frame once the month comes near (see buildSoon). Built whole in one go, a month of a
  // hundred and fifty photos was one task of 50–110 ms — a frame dropped every time a fast
  // scroll reached the next month.
  const [builtTo, setBuilt] = useState(eager ? items.length : 0);
  // Once built whole, it stays whole as it grows: a photo arriving in it is drawn with the rest,
  // where its last row used to go back to the placeholder. Whole for the view it was built in —
  // another view's month of the same name builds a few rows a frame, as a new month does.
  const whole = useRef<string | null>(null);
  if (builtTo >= items.length) whole.current = view;
  const built = whole.current === view ? items.length : builtTo;
  const builtNow = useRef(built);
  builtNow.current = built;
  const step = useRef(across);
  step.current = across;
  /*
   * Its photos, a page at a time, fetched once it comes near (the first months at once) and
   * again whenever it comes back near after they were let go (see photos/list). Until then it
   * has only its ids, and draws as one block exactly as tall as its rows will be.
   */
  const [near, setNear] = useState(eager);
  const seen = useRef<{ version: number; items: Item[] | null; sig: string }>({ version: -1, items: null, sig: "" });
  // Watched once near, or once it has begun to build (its own watcher saw it first).
  const watching = near || built > 0;
  const sig = useSyncExternalStore(list.store.subscribe, () => {
    if (!watching) return "";
    const store = list.store;
    if (seen.current.version === store.version && seen.current.items === items) return seen.current.sig;
    // How many from its start are here, how many in all, and each one's change count: a photo
    // read again or changed here (a heart) draws its tile again, and nothing else does.
    let ready = 0;
    let here = 0;
    let turns = 0;
    for (const { id } of items) {
      if (!store.get(id)) continue;
      if (here === ready) ready++;
      here++;
      turns += store.turns(id);
    }
    seen.current = { version: store.version, items, sig: `${ready}:${here}:${turns}` };
    return seen.current.sig;
  });
  const [ready, here] = watching ? sig.split(":").map((n) => Number(n || 0)) : [0, 0];
  useEffect(() => {
    if (eager && items.length) void list.need(items[0].index, items[items.length - 1].index);
  }, [eager, list, items]);
  /*
   * Its pictures, drawn only near the view: from a screen away, let go two screens away. Each
   * picture drawn is decoded memory the engine keeps for as long as it is there: drawn as
   * months were built, 4000px either side, and never let go, the grid held thirteen screens of
   * them (2,300, about 400 MB) and kept them while another app was in front (#11).
   *
   * Drawn and let go on the elements themselves, never by drawing tiles again: a month's tiles
   * all drawn again as it came near made a fling's longest frame 217 ms (33 ms before). Drawn a
   * slice a frame (buildSoon), and let go only once the screen is idle, so neither ever costs a
   * frame of a scroll. A tile built while its month is near draws its own at once, as before.
   * While the screen is put away (another app in front), what was on and beside the screen
   * stays, so coming back is drawn at once.
   */
  const pics = useRef(eager);
  useEffect(() => {
    const el = ref.current;
    if (!el) return undefined;
    const root = el.closest(".px-scroll");
    let gone = false;
    let drawing = false;
    let letting = 0;
    const draw = () => {
      if (drawing) return;
      drawing = true;
      buildSoon(() => {
        if (gone || !pics.current) return (drawing = false), true;
        const waiting = el.querySelectorAll<HTMLImageElement>("img[data-src]:not([src])");
        for (let i = 0; i < Math.min(BUILD_TILES, waiting.length); i++) waiting[i].src = waiting[i].dataset.src!;
        if (waiting.length > BUILD_TILES) return false;
        drawing = false;
        return true;
      });
    };
    const letGo = () => {
      if (letting) return;
      const idle = (fn: () => void): number =>
        typeof window.requestIdleCallback === "function" ? window.requestIdleCallback(fn, { timeout: 2000 }) : window.setTimeout(fn, 300);
      letting = idle(() => {
        letting = 0;
        if (gone || pics.current) return;
        for (const im of el.querySelectorAll<HTMLImageElement>("img[src]")) {
          im.removeAttribute("src");
          delete im.dataset.in;
          const frame = im.parentElement;
          if (frame) frame.style.backgroundImage = "";
        }
      });
    };
    if (pics.current) draw();
    // Whether it was near the last time the screen was in sight.
    let near = eager;
    const stopNear = watchNear(el, root, "100% 0px", (on) => {
      if (putAway(el)) return;
      near = on;
      if (!on) return;
      pics.current = true;
      draw();
    });
    const stopFar = watchNear(el, root, "200% 0px", (on) => {
      if (on) return;
      // Put away, everything reads as far: only what was not near the screen is let go.
      if (putAway(el) && near) return;
      pics.current = false;
      letGo();
    });
    return () => {
      gone = true;
      stopNear();
      stopFar();
    };
  }, [eager]);
  // One watcher: near the screen (within 4000px), its photos are asked for and it is built a few
  // rows a frame; gone far again before it was built, its building waits until it comes back —
  // a fling to the foot of the library builds the months it lands on, not every month it
  // passed on the way (they queued first, and the screen it landed on waited behind them).
  useEffect(() => {
    const el = ref.current;
    let gone = false;
    let queued = false;
    let close = false;
    const start = () => {
      if (queued || builtNow.current >= items.length) return;
      queued = true;
      buildSoon(() => {
        if (gone) return true;
        if (!close) {
          queued = false;
          return true;
        }
        const rows = Math.max(2, Math.round(BUILD_TILES / step.current));
        const next = Math.min(items.length, builtNow.current + rows * step.current);
        builtNow.current = next;
        setBuilt(next);
        if (next >= items.length) queued = false;
        return next >= items.length;
      });
    };
    if (!el || typeof IntersectionObserver === "undefined" || !items.length) {
      close = true;
      start();
      return () => {
        gone = true;
      };
    }
    const watch = new IntersectionObserver(
      (entries) => {
        // The latest of what came in one go: a layout that moved it out and back within a
        // frame comes as two entries, and the first alone left it out — half built for good.
        close = entries[entries.length - 1].isIntersecting;
        if (!close) return;
        setNear(true);
        void list.need(items[0].index, items[items.length - 1].index);
        start();
      },
      { root: el.closest(".px-scroll"), rootMargin: "4000px 0px" },
    );
    watch.observe(el);
    return () => {
      gone = true;
      watch.disconnect();
    };
  }, [list, items]); // eslint-disable-line react-hooks/exhaustive-deps
  // Built part way when the zoom changed, or its photos part way here: what shows stays on whole
  // rows, and the rest is the block below. Once its grid is showing, it keeps showing: a photo
  // not here yet (one just arrived, fetched as it is drawn) holds its cell as a plain box, and
  // the tiles around it stay as they are. Stopping at it, the grid went back to its placeholder
  // and every tile was made again when it came — the grey flash of a photo arriving. Only a month
  // whose photos have all been let go (far off, see photos/list) goes back to its block.
  const showing = useRef(false);
  const upTo = showing.current && here > 0 ? built : Math.min(built, ready);
  const shownCount = upTo >= items.length ? items.length : Math.floor(upTo / across) * across;
  const live = shownCount > 0;
  showing.current = live;
  return (
    <section ref={ref} className="px-month" aria-label={label || undefined} data-month={month || undefined}>
      {label && <h2 className="px-month-title">{label}</h2>}
      {live && (
        <div className="px-grid" role="listbox" aria-multiselectable="true" aria-label={label || "Photos"}>
          {(shownCount < items.length ? items.slice(0, shownCount) : items).map(({ id, index }) => {
            const photo = list.store.get(id);
            if (!photo) return <div key={id} className="px-ph" aria-hidden="true" />;
            const t = deleted ? timeLeft(photo, true) : null;
            return (
              <Tile
                key={photo.id}
                photo={photo}
                index={index}
                selected={chosen.has(photo.id)}
                tabbable={anchor === index || (anchor === null && index === 0)}
                left={t?.words ?? null}
                soon={t?.soon ?? false}
                fit={fit}
                cell={cell}
                pics={pics}
                act={act}
              />
            );
          })}
        </div>
      )}
      {shownCount < items.length && (
        // Not built yet (far from the screen): one block as tall as its rows will be, its tiles
        // painted on it as plain boxes, so a fast scroll shows photos on their way — never an
        // empty page. One element, not one a photo: a box each was 20,000 on a large library,
        // and every focus and every frame paid for them (photos-scale).
        <div className="px-later" aria-hidden="true" style={{ "--n": items.length - shownCount } as React.CSSProperties}>
          <i className="px-later-in" />
        </div>
      )}
    </section>
  );
}, sameMonth);

export function PhotosView() {
  const revision = useRevision("photos");
  const arriving = useArriving("photos");
  // Apple still preparing the library: said as it is, in place of "you have none".
  const { preparing } = useServiceState("photos");
  const [view, setView] = useState<PhotoView>("library");
  /** An album open instead of a view. */
  const [album, setAlbum] = useState<PhotoAlbum | null>(null);
  const [albums, setAlbums] = useState<[PhotoAlbum, number, (string | null)?][]>([]);
  /** The Albums page open: every album as a card with its cover, as icloud.com's. */
  const [albumsPage, setAlbumsPage] = useState(false);
  /** The Shared Albums page open (Sharing, in the sidebar). */
  const [sharedPage, setSharedPage] = useState(false);
  // Which of the sidebar's branches are open — Albums and Media Types — kept on this machine.
  const [branches, setBranches] = useState<{ albums: boolean; media: boolean }>(() => {
    try {
      return { albums: true, media: true, ...JSON.parse(window.localStorage.getItem("quince.photos.branches") ?? "{}") };
    } catch {
      return { albums: true, media: true };
    }
  });
  const branch = (which: "albums" | "media") =>
    setBranches((b) => {
      const next = { ...b, [which]: !b[which] };
      try {
        window.localStorage.setItem("quince.photos.branches", JSON.stringify(next));
      } catch {
        // For this run only.
      }
      return next;
    });
  /**
   * The view's photos: its index whole, in the order chosen (sorted where they are kept), and
   * the photos themselves from the store as the grid and the viewer come near them. A page
   * landing draws this screen once a frame at most; the months draw only where it landed.
   */
  const store = useMemo(() => new PhotoStore(), []);
  const [photos, setPhotos] = useState<PhotoList | null>(null);
  useSyncExternalStore(store.subscribe, () => store.version);
  /**
   * A burst opened from its chosen photo, as Photos' "View all": the viewer goes through the
   * burst's photos in the order they were shot, counted "3 of 64", until Done (or the viewer
   * closes) brings the library back where it was. `from` is the chosen photo to come back to.
   */
  const [inBurst, setInBurst] = useState<{ from: string; count: number } | null>(null);
  const [counts, setCounts] = useState<Partial<Record<PhotoView, number>>>({});
  /** The library switcher: which library the views show, and whether there is a Shared
   *  Library to choose (the switcher shows only then). */
  const [library, setLibrary] = useState<{ showing: PhotoLibrary; hasShared: boolean }>({ showing: "both", hasShared: false });
  const [error, setError] = useState<string | null>(null);
  useErrorToast(error, () => setError(null));
  const [zoom, setZoom] = useState(1);
  // The Library's level — Years, Months or All Photos — kept on this machine, as Photos keeps it.
  const [level, setLevel] = useState<Level>(() => {
    try {
      const kept = window.localStorage.getItem(LEVEL_KEY);
      return kept === "years" || kept === "months" ? kept : "all";
    } catch {
      return "all";
    }
  });
  // Squares that fill each cell, or each photo whole in its own shape — icloud.com's toggle,
  // kept on this machine.
  const [fit, setFit] = useState(() => {
    try {
      return window.localStorage.getItem("quince.photos.fit") === "1";
    } catch {
      return false;
    }
  });
  const toggleFit = () =>
    setFit((f) => {
      try {
        window.localStorage.setItem("quince.photos.fit", f ? "0" : "1");
      } catch {
        // For this run only.
      }
      return !f;
    });
  const [open, setOpen] = useState<number | null>(null);
  const [info, setInfo] = useState(false);
  /** A slideshow playing: what it shows, and where it begins. */
  const [show, setShow] = useState<{ ids: string[]; start: number } | null>(null);
  /** Photos selected in the grid, as on a Mac: a click selects, a double-click opens. */
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const anchor = useRef<number | null>(null);
  // The anchor is a place in the list, so it follows its photo when the list changes under it
  // (the order turned round, a sync's new photos above it): left where it was, Shift-click and
  // the arrows started from whichever photo had come to stand there.
  const anchorList = useRef(photos);
  if (anchorList.current !== photos) {
    const id = anchor.current !== null ? anchorList.current?.ids[anchor.current] : undefined;
    const at = id !== undefined && photos ? photos.indexOf(id) : -1;
    anchor.current = at >= 0 ? at : null;
    anchorList.current = photos;
  }
  const [uploads, setUploads] = useState<Map<string, UploadEvent>>(new Map());
  /**
   * A batch of photos from another device landing in the view (see read): which, how many of
   * them are videos, and when the last of them landed. Said under the title until the pictures
   * of those near the screen are in — at least a moment, so it is read, and never for long.
   */
  const [receiving, setReceiving] = useState<{ ids: Set<string>; videos: number; at: number } | null>(null);
  useEffect(() => {
    if (!receiving) return undefined;
    const t = window.setInterval(() => {
      const age = performance.now() - receiving.at;
      // Only tiles built (near the screen) and still waiting on their picture; one whose
      // picture cannot be made shows its thumbnail and waits no more.
      const waiting = [...(scroller.current?.querySelectorAll<HTMLImageElement>(".px-tile img:not([data-in]):not([data-gone])") ?? [])]
        .some((img) => receiving.ids.has(img.closest<HTMLElement>(".px-tile")?.dataset.id ?? ""));
      if ((!waiting && age > RECEIVING_SHOWN) || age > RECEIVING_MOST) setReceiving(null);
    }, 250);
    return () => window.clearInterval(t);
  }, [receiving]);
  const [dropping, setDropping] = useState(false);
  const [menu, setMenu] = useState<{ at: { x: number; y: number; alignRight: boolean }; entries: MenuEntry[] } | null>(null);
  /** An album being named in the sidebar: a new one (with the photos to put in it), or a rename. */
  const [naming, setNaming] = useState<{ id: string | null; then: string[]; draft: string; folder?: boolean; parent?: string | null } | null>(null);
  /** Folders open in the sidebar, and the folder whose page is showing (none: the top). */
  const [openFolders, setOpenFolders] = useState<Set<string>>(new Set());
  const [folderPage, setFolderPage] = useState<string | null>(null);

  // Each view keeps the sort chosen for it, on this machine. Albums keep their own order, and
  // Recently Deleted its own, as in Photos.
  // The view the photos on screen belong to. A view chosen changes the title, the months and
  // the order only when its photos arrive, all with them — not a frame before, over the last
  // view's photos.
  const [drawn, setDrawn] = useState<{ view: PhotoView; album: PhotoAlbum | null; query: string }>({ view, album, query: "" });
  const drawnKey = `${drawn.view}/${drawn.album?.id ?? ""}/${drawn.query}`;
  // What is typed in the search field, and — a moment after the typing stops — what is searched.
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState("");
  useEffect(() => {
    const t = window.setTimeout(() => setSearching(query.trim()), 140);
    return () => window.clearTimeout(t);
  }, [query]);
  const endSearch = () => {
    setQuery("");
    setSearching("");
  };
  // What the photos on screen are, for what can be done to them: a search finds across the
  // whole library, so its results take the library's actions — not Recently Deleted's Delete
  // All, Hidden's Unhide or an album's Remove, from the view it was typed over.
  const place: PhotoView = drawn.query ? "library" : view;
  const inAlbum = drawn.query ? null : album;
  /** A shared album open: its photos are looked at, copied and saved, never edited here. */
  const viewOnly = !drawn.query && !!album?.shared;
  const [sorts, setSorts] = useState(readSorts);
  /**
   * The Hidden album is locked, as in Photos: shown only once asked for, and locked again when
   * it is left, when this screen or the window goes out of sight, and at every start (kept in
   * memory only).
   */
  const [hiddenOpen, setHiddenOpen] = useState(false);
  const sortable = !drawn.query && !drawn.album && drawn.view !== "recentlyDeleted" && drawn.view !== "rated";
  const sort = sortable ? (sorts[drawn.view] ?? defaultSort(drawn.view)) : null;
  const chooseSort = (next: Sort) => {
    setSorts((all) => {
      const was = { ...all, [drawn.view]: next };
      try {
        window.localStorage.setItem(SORTS_KEY, JSON.stringify(was));
      } catch {
        // Kept for this run only.
      }
      return was;
    });
  };
  // The order asked of the library for what is being chosen now (the backend sorts: a view is
  // sorted where it is kept, and its index and pages come in that order).
  const wantSortable = !searching && !album && view !== "recentlyDeleted" && view !== "rated";
  const want = wantSortable ? (sorts[view] ?? defaultSort(view)) : null;

  // Reads can pass each other: a sync's read of one view can land after another view was
  // chosen. Only the newest read may draw, or a view shows another's photos under its title.
  const reads = useRef(0);
  /** A burst is being gone through: the library's reads wait until it is left (see viewBurst). */
  const burstOpen = useRef(false);
  const drawnNow = useRef(drawn);
  drawnNow.current = drawn;
  const photosNow = useRef(photos);
  photosNow.current = photos;
  /** The list the last read drew: an arrival is only one over a list read whole, not one changed here. */
  const readList = useRef<PhotoList | null>(null);
  /** Photos changed here a moment ago, by id: one coming back from Apple is no arrival. */
  const ownIds = useRef(new Map<string, number>());
  const ownTouch = (ids: Iterable<string>) => {
    const now = Date.now();
    for (const id of ids) ownIds.current.set(id, now);
    for (const [id, at] of ownIds.current) if (now - at > 120_000) ownIds.current.delete(id);
  };
  /** The files being sent from here, by name: each lands in the library as a photo of that name. */
  const uploadsNow = useRef(uploads);
  uploadsNow.current = uploads;
  const ownNames = () => new Set([...uploadsNow.current.values()].filter((u) => u.state !== "failed" && u.state !== "cancelled").map((u) => u.name));
  /** The tile at the top of the view as a read with new photos was drawn, kept there (see below). */
  const anchorAt = useRef<{ id: string; top: number; list: PhotoList } | null>(null);
  const read = useCallback(async () => {
    if (burstOpen.current) return;
    const mine = ++reads.current;
    // Hidden, locked: nothing of it is read, so nothing of it can be drawn.
    if (view === "hidden" && !album && !searching && !hiddenOpen) {
      setPhotos(new PhotoList({ ids: [], keys: [], marks: [], first: null, last: null }, store));
      setDrawn({ view, album, query: searching });
      return;
    }
    const source: PhotoSource = searching ? { query: searching } : album?.shared ? { shared: album.shared } : album ? { album: album.id } : { view };
    const order: PhotoOrder | null = want;
    const d = drawnNow.current;
    const again = photosNow.current !== null && d.view === view && d.album?.id === album?.id && d.query === searching;
    // Another place: photos being received into the last one are not this one's to speak of.
    if (!again) setReceiving(null);
    const index = api.photosIndex(source, order);
    let indexed = false;
    // Another place chosen: its first screen from its first page, which is small and comes at
    // once, painted before the whole index (a large library's takes a moment) is here.
    if (!again) {
      api.photosPage(source, order, null, PAGE).then(
        (page) => {
          if (indexed || mine !== reads.current) return;
          setPhotos(PhotoList.of(page, store, (p) => sortKey(p, order?.by ?? "taken")));
          setDrawn({ view, album, query: searching });
        },
        () => {},
      );
    }
    const [read, c, a, l] = await Promise.all([index, api.photosCounts(), api.photosAlbums(), api.photosLibrary().catch(() => null)]);
    indexed = true;
    if (mine !== reads.current) return;
    const next = new PhotoList(read, store);
    const before = photosNow.current;
    if (again && before && !next.same(before)) {
      // Photos new to the view being looked at (taken on the phone a moment ago) are fetched
      // before the list with them is drawn, so each lands as its tile with its photo — drawn
      // first, its month showed its placeholder where its photos had been until they came.
      const fresh = next.ids.filter((id) => before.indexOf(id) < 0);
      await store.take(fresh.slice(0, 2 * PAGE));
      if (mine !== reads.current) return;
      // A few arriving where the grid was already drawn from a whole read fade in, each alone;
      // a batch, the person's own, or anything over a partial first page simply appears.
      const own = ownNames();
      const incoming = before === readList.current && !before.partial
        ? fresh.filter((id) => !ownIds.current.has(id) && !own.has(store.get(id)?.filename ?? ""))
        : [];
      const arriving = fresh.length <= ARRIVALS ? incoming : [];
      const at = performance.now();
      // A batch from another device: the line under the title says so while its pictures come
      // in, as Photos' quiet status line does, rather than tile after tile appearing unexplained.
      // Photos landing while it is up are counted in.
      setReceiving((was) => {
        if (incoming.length <= ARRIVALS && !was) return was;
        const added = incoming.filter((id) => !was?.ids.has(id));
        if (!added.length) return was;
        const videos = (was?.videos ?? 0) + added.filter((id) => (next.marks[next.indexOf(id)] & 3) === 1).length;
        return { ids: new Set([...(was?.ids ?? []), ...added]), videos, at };
      });
      for (const [id, when] of arrived) if (at - when > ARRIVAL_MS) arrived.delete(id);
      for (const id of arriving) arrived.set(id, at);
      const keep = anchorOf(scroller.current);
      anchorAt.current = keep && { ...keep, list: next };
    }
    // Read again (a sync landed, a change was made): what is held is fetched again where the
    // grid is, and only a list that changed is drawn again.
    if (again) store.refresh();
    store.use(next);
    readList.current = before && next.same(before) ? before : next;
    setPhotos((was) => (next.same(was) ? was : next));
    const near = again ? store.near : 0;
    void next.need(near - PAGE, near + PAGE);
    // The same view read again is the same view drawn: a new object here built every month
    // afresh and faded the grid in again at each landing.
    setDrawn((was) => (was.view === view && was.album === album && was.query === searching ? was : { view, album, query: searching }));
    // The sidebar's counts and albums, drawn again only when one of them changed.
    const counted = Object.fromEntries(c);
    setCounts((was) => (same(was, counted) ? was : counted));
    setAlbums((was) => (same(was, a) ? was : a));
    if (l) setLibrary((was) => (was.showing === l.showing && was.hasShared === l.hasShared ? was : l));
    // An album deleted elsewhere is not left open, empty.
    if (album && !album.shared && !a.some(([x]) => x.id === album.id)) setAlbum(null);
  }, [view, album, searching, want?.by, want?.oldest, hiddenOpen, library.showing]); // eslint-disable-line react-hooks/exhaustive-deps
  // A pass that lands while another screen is in front is read when this one comes back, once,
  // not at every landing: a screen out of sight is still mounted (lib/screen), and each pass
  // lands a service at a time.
  const active = useScreenActive();
  // Whether Photos is the screen in front, read inside listeners that outlive a render.
  const frontRef = useActiveRef();
  const hiddenLocked = view === "hidden" && !album && !searching && !hiddenOpen;
  // Locked again once it is left — another place, an album, a search, another screen — and
  // whenever the window goes out of sight (minimised, closed to the tray, another workspace).
  const inHidden = view === "hidden" && !album && !searching && !albumsPage && !sharedPage && active;
  useEffect(() => {
    if (!inHidden) setHiddenOpen(false);
  }, [inHidden]);
  useEffect(() => {
    const away = () => document.hidden && setHiddenOpen(false);
    document.addEventListener("visibilitychange", away);
    return () => document.removeEventListener("visibilitychange", away);
  }, []);
  // A hidden photo open when it locks closes with it.
  useEffect(() => {
    if (hiddenLocked) setOpen(null);
  }, [hiddenLocked]);
  // Locked here, locked in the app too: nothing of Hidden is handed out until it is unlocked
  // again with the password.
  useEffect(() => {
    if (!hiddenOpen) void api.photosLockHidden().catch(() => {});
  }, [hiddenOpen]);
  /** The password sheet that unlocks Hidden is open. */
  const [unlocking, setUnlocking] = useState(false);
  const seen = useRef(revision);
  if (active) seen.current = revision;
  const due = seen.current;
  useEffect(() => {
    read().catch((e) => setError(messageOf(e)));
  }, [read, due]);

  // Grouped by month, as the library reads — or, zoomed in close, by day, as Photos' Days do;
  // an album is one grid in its own order. Each group knows its month, for the month picker and
  // the line under the title.
  const byDay = zoom >= DAYS_FROM;
  const months = useMemo(() => {
    const out: { key: string; label: string; month: string; items: Item[] }[] = [];
    // An album keeps its own order, and Recently Deleted is in the order things were deleted:
    // grouped by the month they were taken, the same month came back again and again. Both are
    // one grid, as icloud.com shows them.
    if (!photos) return out;
    if (drawn.album || drawn.view === "recentlyDeleted") return [{ key: "", label: "", month: "", items: photos.ids.map((id, index) => ({ id, index })) }];
    const thisYear = new Date().getFullYear();
    const rated = drawn.view === "rated" && !drawn.album;
    // Named once a month (or day), not once a photo: fifty thousand dates formatted on every
    // read was most of laying the grid out.
    const names = new Map<number, [string, string]>();
    const seen = new Map<string, number>();
    let lastKey = NaN;
    let lastLabel = "";
    let lastMonth = "";
    photos.ids.forEach((id, index) => {
      // Rated is grouped by how many stars, as icloud.com's Rated page is.
      let label: string;
      let month: string;
      if (rated) {
        const stars = photos.marks[index] >> 3;
        label = `${stars} ${stars === 1 ? "Star" : "Stars"}`;
        month = "";
      } else {
        const at = photos.keys[index];
        if (at !== lastKey) {
          lastKey = at;
          const d = new Date(at * 1000);
          const bucket = byDay ? d.getFullYear() * 400 + d.getMonth() * 32 + d.getDate() : d.getFullYear() * 12 + d.getMonth();
          let named = names.get(bucket);
          if (!named) {
            const m = monthOf(at);
            named = [m, byDay ? dayOf(at, thisYear) : m];
            names.set(bucket, named);
          }
          [lastMonth, lastLabel] = named;
        }
        label = lastLabel;
        month = lastMonth;
      }
      const last = out[out.length - 1];
      if (last && last.label === label) last.items.push({ id, index });
      else {
        // A name seen before (search results are not in date order) is told apart by how many
        // times it has come.
        const times = (seen.get(label) ?? 0) + 1;
        seen.set(label, times);
        out.push({ key: times > 1 ? `${label}#${times}` : label, label, month, items: [{ id, index }] });
      }
    });
    return out;
  }, [photos, drawn, byDay]);

  const shown = open !== null ? photos?.at(open) ?? null : null;
  /**
   * The picture last painted on the stage, kept beneath the next one until that one is decoded:
   * a step never shows a blank, nor the soft thumbnail before the sharp picture (the flicker on
   * every switch). The next is decoded where it stands, by the engine, as it would be anyway —
   * holding the step back until a copy off the page was decoded cost a fifth of its frames.
   */
  // The viewer's date, time and place among the photos, as its bar shows them.
  const when = shown && (
    <div className="px-viewer-when">
      <strong>{new Date(shown.takenAt * 1000).toLocaleDateString(undefined, { dateStyle: "long" })}</strong>
      {place === "recentlyDeleted" && !inAlbum && timeLeft(shown, false) ? (
        <span className={timeLeft(shown, false)!.soon ? "is-soon" : undefined}>{timeLeft(shown, false)!.words}</span>
      ) : (
        <span>
          {new Date(shown.takenAt * 1000).toLocaleTimeString(undefined, { timeStyle: "short" })}
          {/* Where it is among them, as icloud.com says it: "133 of 169". */}
          {photos && photos.length > 1 && open !== null && !inBurst && (
            <span className="px-viewer-place">
              {" · "}
              {(open + 1).toLocaleString()} of {photos.length.toLocaleString()}
            </span>
          )}
        </span>
      )}
    </div>
  );
  const painted = useRef<Photo | null>(null);
  const underEl = useRef<HTMLImageElement | null>(null);
  // The last picture painted, held under a step until the next one is decoded at the size the
  // stage draws it: the previous sharp photo, never the soft thumbnail, and never an empty frame.
  const under = shown && shown.kind !== "video" && painted.current && painted.current.id !== shown.id && painted.current.kind !== "video" && !decoded.has(viewSrc(shown)) ? painted.current : null;
  if (!shown || shown.kind === "video") painted.current = null;
  // The viewer follows the photo, not its place: when the list moves under it — its date
  // changed, a sync reordered the library — it finds the photo again rather than showing
  // whichever came to stand where it was.
  const openId = useRef<string | null>(null);
  const lastList = useRef(photos);
  useLayoutEffect(() => {
    const moved = lastList.current !== photos;
    lastList.current = photos;
    if (open === null || !photos) {
      openId.current = null;
      return;
    }
    // Only a list that changed sends it looking; a step to another photo is a new photo.
    if (moved && openId.current && photos.ids[open] !== openId.current) {
      const again = photos.indexOf(openId.current);
      if (again >= 0) {
        setOpen(again);
        return;
      }
    }
    // Gone from the end of the list (hidden, deleted or unfavourited elsewhere): the photo
    // before it, as a delete here does — left past the end, the viewer drew nothing while the
    // grid stayed hidden behind it.
    if (open >= photos.length) {
      setOpen(photos.length > 0 ? photos.length - 1 : null);
      return;
    }
    openId.current = photos.ids[open] ?? null;
  }, [open, photos]);

  /*
   * The viewer opens as Photos opens a photo: it grows out of its tile while the dark comes up
   * behind it, and closing sends it back into the tile of the photo now shown. Transform and
   * opacity only, so the compositor does it all; with reduced motion it simply appears.
   */
  const viewer = useRef<HTMLDivElement | null>(null);
  const stage = useRef<HTMLDivElement | null>(null);
  const origin = useRef<DOMRect | null>(null);
  const closing = useRef(false);
  /**
   * The grid behind a photo that has finished opening is not painted (visibility): WebKit
   * painted every tile under the opaque viewer on each step to the next photo, 80 ms a step.
   * Only while the photo rests — never while it opens or closes. Hidden with content-visibility
   * instead, the grid was dropped from the page: an open showed it one frame and then an empty
   * background under the fading backdrop, and a close brought it back over several frames,
   * dropping out again in one (his recording: 26 → 61 → 118 → 33 → 145) — the flicker on every
   * open and close. Visibility keeps its layout and its decoded pictures, so it is back whole in
   * the very frame the close begins.
   */
  const gridBehind = (on: boolean) => {
    const grid = scroller.current;
    if (!grid) return;
    if (on) grid.setAttribute("data-behind", "");
    else grid.removeAttribute("data-behind");
  };
  useEffect(() => {
    if (open === null) gridBehind(false);
  }, [open === null]); // eslint-disable-line react-hooks/exhaustive-deps
  // A close still running: its animations and its fallback, so a photo opened in the middle
  // of it is not shut by the close finishing, nor left faded out by it.
  const closingRun = useRef<{ anims: Animation[]; timer: number } | null>(null);
  /** Every tile lifted for a photo's flight put back. */
  const land = () => document.querySelectorAll(".px-tile[data-lifted]").forEach((t) => t.removeAttribute("data-lifted"));
  const stopClosing = () => {
    const run = closingRun.current;
    if (!run) return;
    closingRun.current = null;
    closing.current = false;
    window.clearTimeout(run.timer);
    run.anims.forEach((a) => {
      a.onfinish = null;
      a.cancel();
    });
    // What the close wrote as the elements' own style (see glide) goes with it: the viewer stays,
    // whole, as it was before the close began.
    const el = viewer.current;
    if (el) {
      el.style.opacity = "";
      el.style.visibility = "";
      for (const part of el.querySelectorAll<HTMLElement>(".px-backdrop, .px-viewer-bar, .px-nav, .px-info, .px-strip")) part.style.opacity = "";
      const img = el.querySelector<HTMLElement>(".px-pan, .px-video");
      if (img) img.style.transform = "";
    }
  };
  // The photo's own box, laid out by the engine itself against the stage (a size container):
  // as large as fits, never past 1.5× its own pixels, its shape exact. Known before it loads,
  // so its thumbnail fills that box at once and the sharper picture lands without anything
  // moving; and a window being resized redraws it every frame with no script in the way.
  // A photo's shape as its picture turned out, where that is not the shape Apple records:
  // Apple keeps the sensor's width and height, before the turn the camera wrote into the file,
  // and an engine that honours that turn (WebView2 does) drew an upright photo inside a frame
  // lying on its side, with the thumbnail painted under it showing at both sides, stretched.
  const [trueShape, setTrueShape] = useState<ReadonlyMap<string, number>>(() => new Map());
  const learnShape = (p: Photo, img: HTMLImageElement) => {
    if (!img.naturalWidth || !img.naturalHeight || !p.width || !p.height) return;
    const seen = img.naturalWidth / img.naturalHeight;
    if (Math.abs(seen - p.width / p.height) < 0.02 || trueShape.get(p.id) === seen) return;
    setTrueShape((was) => new Map(was).set(p.id, seen));
  };
  const fitted = (p: Photo): React.CSSProperties => {
    // The grid's thumbnail, already decoded, painted under the picture: never a black stage.
    const under = { backgroundImage: `url("${loadedThumb(p.id)}")` };
    if (!p.width || !p.height) return under;
    // Both sides set, each the smaller of the room across and the room down in the photo's own
    // proportion: WebKit sized a replaced image's height from its width alone, ignoring
    // aspect-ratio, and drew every photo square.
    const ratio = trueShape.get(p.id) ?? p.width / p.height;
    const r = ratio.toFixed(6);
    const long = Math.max(p.width, p.height) * 1.5;
    const [w, h] = ratio >= 1 ? [long, long / ratio] : [long * ratio, long];
    return {
      ...under,
      width: `min(100cqw, ${r} * 100cqh, ${Math.round(w)}px)`,
      height: `min(100cqw / ${r}, 100cqh, ${Math.round(h)}px)`,
      maxWidth: "none",
      maxHeight: "none",
    };
  };
  /** A photo's shape as shown, upright (its picture's own, once learned): the box a film is
   *  turned into. */
  const shapeOf = (p: Photo) => ({ w: trueShape.get(p.id) ?? (p.width && p.height ? p.width / p.height : 4 / 3), h: 1 });
  const still = () => typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  // A photo opens and closes at once, with no animation: every way of animating it — the photo
  // growing out of its tile, the backdrop fading, the bars fading, the grid behind it hidden or
  // shown during the move — still showed him a flicker in WebKitGTK on Hyprland. Opened, the
  // grid behind it is left unpainted (gridBehind); it is painted again before the viewer goes.
  useLayoutEffect(() => {
    if (open === null) return;
    origin.current = null;
    const el = viewer.current;
    if (el) el.style.visibility = "";
    gridBehind(true);
  }, [open !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  // The window takes the viewer's near-black while it is open, and the app's colour again
  // after: a resize then never shows a lighter band beside the photo.
  useLayoutEffect(() => {
    if (open === null) return undefined;
    tintFrom(viewer.current?.querySelector(".px-backdrop") ?? null);
    return () => tintFrom(document.body);
  }, [open !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  /** The photo the viewer was on as it closed: the library comes back with it chosen and in view. */
  const backTo = useRef<number | null>(null);
  const openNow = useRef(open);
  openNow.current = open;
  const close = useCallback(() => {
    gridBehind(false);
    land();
    backTo.current = openNow.current;
    setOpen(null);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  // A step lands only on a photo that is here — the next ones are fetched well ahead (below) —
  // so the stage never shows nothing; a key held at the edge of what is here waits for it.
  const step = useCallback(
    (by: number) => {
      const at = openNow.current;
      if (at === null || !photos) return;
      const to = Math.max(0, Math.min(photos.length - 1, at + by));
      if (to === at) return;
      if (!photos.at(to)) {
        void photos.need(to - PAGE / 2, to + PAGE / 2);
        return;
      }
      setOpen(to);
    },
    [photos],
  );

  /*
   * Zooming into a photo, as Photos does: double-click zooms to where was clicked and back;
   * Ctrl + scroll (a trackpad pinch arrives as that) zooms about the pointer; dragging pans.
   * One transform on the photo's frame, so it is the compositor's work. Past the fit, the
   * full picture is laid over the screen-sized one once it has loaded.
   */
  // The lens lives outside React: a gesture writes the frame's transform straight to it, once a
  // frame, so a pan or a pinch never redraws the screen (which made it crawl). Only crossing
  // into or out of zoom is state — the cursor and the double-click's meaning change with it.
  const lensRef = useRef({ k: 1, x: 0, y: 0 });
  // The photo the zoom is on: stepping to the next one draws it at its fit from its first
  // render. As a plain flag it stayed on for that render, and the next photo's whole original
  // was asked for (for a HEIC, downloaded and decoded) before the zoom was put back.
  const [zoomedId, setZoomedId] = useState<string | null>(null);
  const zoomed = zoomedId !== null && zoomedId === shown?.id;
  const pan = useRef<HTMLDivElement | null>(null);
  const frame = useRef<number | null>(null);
  // What is on the screen now, and a glide on its way to the lens.
  const drawn_ = useRef({ k: 1, x: 0, y: 0 });
  const gliding = useRef<number | null>(null);
  /**
   * The lens drawn by the page itself, once a frame: never a CSS transition, never will-change,
   * never a 3D transform. Each of those hands the picture to WebKitGTK's compositor (a layer
   * made as a zoom began and dropped as it ended), and on his screen that flickered — 88
   * frames in one recording of zooming. A double-click's or a key's zoom glides over a moment,
   * stepped from here; a pinch or a drag follows the hand directly.
   */
  const draw = (l: { k: number; x: number; y: number }) => {
    drawn_.current = l;
    const el = pan.current;
    if (el) el.style.transform = l.k > 1 || l.x || l.y ? `translate(${l.x}px, ${l.y}px) scale(${l.k})` : "";
    if (zoomSlider.current && zoomSlider.current !== document.activeElement) zoomSlider.current.value = String(l.k);
  };
  const paint = (next: { k: number; x: number; y: number }, glide = false) => {
    lensRef.current = next;
    setZoomedId(next.k > 1 ? (shown?.id ?? null) : null);
    if (gliding.current !== null) {
      cancelAnimationFrame(gliding.current);
      gliding.current = null;
    }
    const still = typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (glide && !still) {
      const from = drawn_.current;
      const start = performance.now();
      const step = (now: number) => {
        const t = Math.min(1, (now - start) / 220);
        const e = 1 - (1 - t) ** 3;
        draw({ k: from.k + (next.k - from.k) * e, x: from.x + (next.x - from.x) * e, y: from.y + (next.y - from.y) * e });
        gliding.current = t < 1 ? requestAnimationFrame(step) : null;
      };
      gliding.current = requestAnimationFrame(step);
      return;
    }
    if (frame.current !== null) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = null;
      draw(lensRef.current);
    });
  };
  const viewBurst = async (p: Photo) => {
    if (!p.burst) return;
    try {
      const list = new PhotoList(await api.photosIndex({ burst: p.burst }, null), store);
      await list.need(0, list.length - 1);
      reads.current++; // a sync's read landing now must not put the library back over it
      store.use(list);
      setPhotos(list);
      setInBurst({ from: p.id, count: list.length });
      // Opened by the photo, not a place: the list is drawn as it is shown.
      setReopen(p.id);
      burstOpen.current = true;
    } catch (e) {
      setError(messageOf(e));
    }
  };
  const leaveBurst = async (keepOpen: boolean) => {
    const back = inBurst?.from;
    setInBurst(null);
    burstOpen.current = false;
    await read().catch(() => {});
    if (keepOpen && back) setReopen(back);
  };
  /** A photo to open again once the library is back (after a burst). */
  const [reopen, setReopen] = useState<string | null>(null);
  useEffect(() => {
    if (!reopen || !photos) return;
    const at = photos.indexOf(reopen);
    setReopen(null);
    if (at >= 0) setOpen(at);
  }, [reopen, photos]);
  // No picture anywhere in Photos — tile, slip, cover — ever shows the engine's broken box: one
  // listener (errors do not bubble, so it listens on the way down) marks any that fails, and the
  // style draws nothing for it; what is under it shows. The viewer tries the thumbnail first.
  useEffect(() => {
    const failed = (e: Event) => {
      const img = e.target;
      if (img instanceof HTMLImageElement && img.closest(".px, .px-viewer, .px-jump, .px-show") && !img.classList.contains("px-shown")) {
        img.dataset.failed = "";
      }
    };
    document.addEventListener("error", failed, true);
    return () => document.removeEventListener("error", failed, true);
  }, []);
  // However the viewer went (closed, its last photo deleted, another view chosen), no tile is
  // left lifted out after it.
  useEffect(() => {
    if (open !== null) return undefined;
    const id = requestAnimationFrame(land);
    return () => cancelAnimationFrame(id);
  }, [open === null]); // eslint-disable-line react-hooks/exhaustive-deps
  // Closing the viewer inside a burst brings the library back.
  useEffect(() => {
    if (open === null && inBurst) void leaveBurst(false);
  }, [open === null]); // eslint-disable-line react-hooks/exhaustive-deps
  // Closed after going through the photos, as Photos: the one last shown is the one chosen in
  // the library, and the grid has it in view with the keys' focus on it — not the photo it was
  // opened from, forty photos and two months back up the grid.
  useLayoutEffect(() => {
    const at = backTo.current;
    backTo.current = null;
    if (open !== null || at === null || inBurst) return;
    const id = photos?.ids[at];
    if (!id) return;
    anchor.current = at;
    setChosen((c) => (c.size === 1 && c.has(id) ? c : new Set([id])));
    reach(at);
  }, [open === null]); // eslint-disable-line react-hooks/exhaustive-deps
  /**
   * The window full screen for a photo, as icloud.com's viewer offers: the button, and Escape
   * leaving full screen before it closes the photo. Closing the photo leaves it too.
   */
  const [fullscreen, setFullscreen] = useState(false);
  const toggleFullscreen = async (on = !fullscreen) => {
    try {
      setFullscreen(await api.windowFullscreen(on));
    } catch (e) {
      setError(messageOf(e));
    }
  };
  useEffect(() => {
    if (open === null && fullscreen) void toggleFullscreen(false);
  }, [open === null]); // eslint-disable-line react-hooks/exhaustive-deps
  /**
   * Full screen is the photo alone, as icloud.com's: black to every edge, its bar out of sight
   * until the pointer goes to the top (or a key's focus lands in it), the thumbnails along the
   * bottom only when asked for — remembered for next time.
   */
  const [thumbs, setThumbs] = useState(() => {
    try {
      return localStorage.getItem("quince.photos.fullThumbs") === "1";
    } catch {
      return false;
    }
  });
  const toggleThumbs = () =>
    setThumbs((v) => {
      try {
        localStorage.setItem("quince.photos.fullThumbs", v ? "0" : "1");
      } catch {
        /* a convenience only */
      }
      return !v;
    });
  // A toast says its piece above the filmstrip, never on it.
  useToastClearance(open !== null && (photos?.length ?? 0) > 1 && (!fullscreen || thumbs) ? (fullscreen ? 52 : 70) : 0);
  const [barUp, setBarUp] = useState(true);
  const barTimer = useRef<number | undefined>(undefined);
  const fullBar = useRef<HTMLElement | null>(null);
  // Shown as full screen begins, then away after a moment unless the pointer is on it.
  const barAway = (after = 1600) => {
    window.clearTimeout(barTimer.current);
    barTimer.current = window.setTimeout(() => {
      if (fullBar.current?.matches(":hover") || fullBar.current?.querySelector(":focus-visible")) return;
      setBarUp(false);
    }, after);
  };
  useEffect(() => {
    if (!fullscreen) return window.clearTimeout(barTimer.current);
    setBarUp(true);
    barAway();
    return () => window.clearTimeout(barTimer.current);
  }, [fullscreen]); // eslint-disable-line react-hooks/exhaustive-deps
  // It comes back with a page-drawn fade, never the compositor's (see lib/fade).
  useLayoutEffect(() => {
    if (fullscreen && barUp) fadeIn(fullBar.current, { duration: 120 });
  }, [barUp, fullscreen]);
  const zoomSlider = useRef<HTMLInputElement | null>(null);
  /** Zoom about the stage's middle, as the bar's slider and its − and + do. */
  const zoomMiddle = (k: number, glide = true) => {
    const box = stage.current?.getBoundingClientRect();
    if (box) zoomAt(k, box.left + box.width / 2, box.top + box.height / 2, glide);
  };
  /** A Live Photo's moving part playing over it. */
  const [moving, setMoving] = useState<string | null>(null);
  useEffect(() => setMoving(null), [shown?.id]);
  /** How the film being played is turned, as its file records it (see turnStyle). */
  const [turn, setTurn] = useState<{ key: string; value: Turn; zoom: number } | null>(null);
  /** Videos Apple keeps no streaming copy of: played from the original instead. */
  const [noCopy, setNoCopy] = useState<Set<string>>(() => new Set());
  // A video plays from Apple's streaming copy (H.264, its header first: it starts at once), not
  // the original as shot — often a gigabyte of HEVC this computer can barely decode.
  const film = shown?.kind === "video" ? `${shown.id}/${noCopy.has(shown.id) ? "original" : "video"}` : null;
  const turnFor = film ?? (moving && moving === shown?.id ? `${moving}/video` : null);
  useEffect(() => {
    if (!turnFor || turn?.key === turnFor) return undefined;
    let live = true;
    const [id, which] = turnFor.split("/") as [string, "original" | "video"];
    // Its turn and its margin, both read from the film's header.
    const zoom = api.photoVideoZoom(id, which).catch(() => 1);
    api.photoVideoTurn(id, which).then(
      async (value) => {
        const z = await zoom;
        if (live) setTurn({ key: turnFor, value, zoom: z });
      },
      // Not known: drawn as served, as it was before.
      () => live && setTurn({ key: turnFor, value: [1, 0, 0, 1], zoom: 1 }),
    );
    return () => {
      live = false;
    };
  }, [turnFor]); // eslint-disable-line react-hooks/exhaustive-deps
  /** Videos this computer's engine could not play; each opens in the system's player instead. */
  const [unplayable, setUnplayable] = useState<Set<string>>(() => new Set());
  // The window's player reads only http(s): videos come from the app's own loopback server.
  const [videoBase, setVideoBase] = useState<string | null>(null);
  /** What this computer's player can decode: why a video it cannot play is a still. */
  const [videoSupport, setVideoSupport] = useState<{ missing: string[] } | null>(null);
  useEffect(() => {
    api.videoBase().then(setVideoBase, () => setVideoBase(null));
    api.videoSupport().then(setVideoSupport, () => setVideoSupport(null));
  }, []);
  const unzoom = (glide = true) => paint({ k: 1, x: 0, y: 0 }, glide);
  // Kept so the image's edge never comes in past the middle of the stage.
  const bound = (k: number, x: number, y: number) => {
    const el = pan.current;
    if (!el || k <= 1) return { k: 1, x: 0, y: 0 };
    const mx = ((k - 1) * el.offsetWidth) / 2;
    const my = ((k - 1) * el.offsetHeight) / 2;
    return { k, x: Math.max(-mx, Math.min(mx, x)), y: Math.max(-my, Math.min(my, y)) };
  };
  // Zoom to k, keeping the point under (sx, sy) — screen coordinates — where it is.
  const zoomAt = (k: number, sx: number, sy: number, glide = true) => {
    const el = pan.current;
    if (!el) return;
    // The frame's centre as laid out, before any transform: the transform on screen may be a
    // frame behind the lens while a pinch runs.
    const host = (el.offsetParent as HTMLElement | null)?.getBoundingClientRect();
    if (!host) return;
    const was = lensRef.current;
    const cx = host.left + el.offsetLeft + el.offsetWidth / 2;
    const cy = host.top + el.offsetTop + el.offsetHeight / 2;
    const [dx, dy] = [sx - cx, sy - cy];
    const px = (dx - was.x) / was.k;
    const py = (dy - was.y) / was.k;
    const next = Math.max(1, Math.min(5, k));
    paint(bound(next, dx - next * px, dy - next * py), glide);
  };
  // Back to the fit as another photo comes up, before it is drawn: put back a frame later,
  // the next photo was painted first at the last one's zoom and pan.
  useLayoutEffect(() => {
    if (gliding.current !== null) cancelAnimationFrame(gliding.current);
    if (frame.current !== null) cancelAnimationFrame(frame.current);
    gliding.current = frame.current = null;
    lensRef.current = { k: 1, x: 0, y: 0 };
    draw(lensRef.current);
    setZoomedId(null);
  }, [shown?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  // The stage, measured as it opens and as the window is resized: the picture the viewer shows
  // is made for its size (viewSrc), and asked for again only when that size crosses a step.
  const [, setStep] = useState("");
  useLayoutEffect(() => {
    const el = stage.current;
    if (!el) return undefined;
    const measure = () => {
      // Out of sight (another screen in front) it has no box: nothing to measure.
      if (!el.clientWidth) return;
      const cs = getComputedStyle(el);
      stageBox = {
        w: el.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight),
        h: el.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom),
      };
      if (shown && shown.kind !== "video") setStep(viewSrc(shown));
      // Where the photo itself sits on the stage, for the LIVE mark laid on its corner: read a
      // frame later, once the picture has its box. Only for a Live Photo, and written only when
      // it moved — read and set on every step, it restyled the whole stage each time and cost
      // the stepping a fifth of its frames.
      if (shown?.kind !== "live") return;
      requestAnimationFrame(() => {
        const p = el.querySelector<HTMLElement>(".px-pan");
        if (!p) return;
        const x = `${p.offsetLeft}px`;
        const y = `${p.offsetTop}px`;
        if (el.style.getPropertyValue("--photo-x") !== x) el.style.setProperty("--photo-x", x);
        if (el.style.getPropertyValue("--photo-y") !== y) el.style.setProperty("--photo-y", y);
      });
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, [open !== null, shown?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  // A pinch or Ctrl + wheel: the page must not zoom too, so this listener is not passive.
  useEffect(() => {
    const el = stage.current;
    if (!el) return undefined;
    const wheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      zoomAt(lensRef.current.k * Math.exp(-e.deltaY * 0.01), e.clientX, e.clientY, false);
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  });
  const dragging = useRef<{ x: number; y: number; lx: number; ly: number } | null>(null);
  // The photos either side are fetched and decoded while this one shows, so a step lands on
  // a whole picture at once. (No slide between them: moving a picture as it changes is what
  // tore in WebKitGTK.)
  useEffect(() => {
    if (open === null || !photos) return;
    // The photos either side, well past where a held key gets to before they land.
    void photos.need(open - PAGE / 2, open + PAGE / 2);
    for (const by of [1, -1, 2, -2]) {
      const p = photos.at(open + by);
      if (p && p.kind !== "video") void warm(p);
    }
  }, [open, photos, open !== null && photos?.at(open + 1), open !== null && photos?.at(open - 1)]); // eslint-disable-line react-hooks/exhaustive-deps

  // The viewer's keys: arrows to go through, Escape to close, I for the details.
  const live = useActiveRef();
  useEffect(() => {
    if (open === null) return undefined;
    const current = photos?.at(open) ?? null;
    const onKey = (e: KeyboardEvent) => {
      if (!live.current) return;
      // Keys typed into a field — the caption — are words, not commands.
      if ((e.target as HTMLElement | null)?.closest?.("input:not([type=range]), textarea, [contenteditable]")) return;
      // The zoom slider's own arrows move it; every other key still works with it focused.
      if ((e.target as HTMLElement | null)?.matches?.("input[type=range]") && e.key.startsWith("Arrow")) return;
      if (e.key === "ArrowRight") step(1);
      else if (e.key === "ArrowLeft") step(-1);
      else if (e.key === "Escape" && lensRef.current.k > 1) unzoom();
      else if (e.key === "Escape" && fullscreen) void toggleFullscreen(false);
      else if (e.key === "Escape") close();
      // Cmd/Ctrl + "." closes too, as icloud.com's one-up view does.
      else if ((e.ctrlKey || e.metaKey) && e.key === ".") close();
      // Ctrl+C: the photo shown, on the clipboard — unless words in the Info are selected.
      else if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "c" && current && current.kind !== "video" && !window.getSelection()?.toString()) void copy([current.id]);
      // Z: fit, or the photo at full size about its middle.
      else if (e.key.toLowerCase() === "z" && !e.ctrlKey && !e.metaKey && current?.kind !== "video") {
        const box = pan.current?.getBoundingClientRect();
        if (box) zoomAt(lensRef.current.k > 1 ? 1 : 2.5, box.left + box.width / 2, box.top + box.height / 2);
      }
      else if ((e.ctrlKey || e.metaKey) && (e.key === "=" || e.key === "+" || e.key === "-" || e.key === "0") && current?.kind !== "video") {
        const box = pan.current?.getBoundingClientRect();
        if (!box) return;
        const k = e.key === "0" ? 1 : lensRef.current.k * (e.key === "-" ? 1 / 1.5 : 1.5);
        zoomAt(k, box.left + box.width / 2, box.top + box.height / 2);
      }
      // Space, as in Photos: a video plays and pauses (its player takes the key first), a Live
      // Photo plays its moving part, and a still goes back to the library it was opened from.
      else if (e.key === " " && current?.kind === "video" && !(videoBase && !unplayable.has(current.id))) void play(current);
      else if (e.key === " " && current?.kind === "live" && videoBase) setMoving((m) => (m === current.id ? null : current.id));
      else if (e.key === " " && current?.kind === "image" && !(e.target as HTMLElement | null)?.closest?.("button, [role=radio], [role=slider]")) close();
      else if (e.key.toLowerCase() === "i" && !e.metaKey && !e.ctrlKey) setInfo((v) => !v);
      // As in Photos: "." favorites, Delete sends it to Recently Deleted.
      // F as well as ".", and H as well as Ctrl+L, as icloud.com takes them.
      else if ((e.key === "." || (e.key.toLowerCase() === "f" && !e.ctrlKey && !e.metaKey)) && current && place !== "recentlyDeleted") void change(current, "favorite", !current.favorite);
      else if (((e.ctrlKey || e.metaKey) ? e.key.toLowerCase() === "l" : e.key.toLowerCase() === "h") && current && place !== "recentlyDeleted") void change(current, "hidden", place !== "hidden");
      else if ((e.key === "Delete" || e.key === "Backspace") && current && inAlbum) void removeFrom([current.id]);
      else if ((e.key === "Delete" || e.key === "Backspace") && current && place !== "recentlyDeleted") void change(current, "deleted", true);
      else if ((e.key === "Delete" || e.key === "Backspace") && current && place === "recentlyDeleted" && !inAlbum) void change(current, "expunged", true);
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  /**
   * Favorite, Delete and Recover show at once, as on the phone; Apple's answer only matters if
   * it refuses, which puts the photo back as it was and says why.
   */
  const [ask, asking] = useConfirm();
  const change = (p: Photo, flag: Flag, on: boolean) => changeAll([p.id], flag, on);
  /** A flag set on photos by id — what is here changes at once, the rest when it is read. */
  const changeAll = async (list: string[], flag: Flag, on: boolean) => {
    if (list.length === 0 || viewOnly) return;
    const n = list.length;
    // Asked first, in icloud.com's words: a delete reaches every device.
    if (
      flag === "deleted" && on &&
      !(await ask({
        title: n === 1 ? "Delete this photo?" : `Delete ${n} photos?`,
        body: `${n === 1 ? "It" : "They"} will be deleted from iCloud Photos on all your devices. ${n === 1 ? "It stays" : "They stay"} in Recently Deleted for 30 days.`,
        action: "Delete",
        danger: true,
        symbol: "trash",
      }))
    ) {
      return;
    }
    // For good, in Photos' words: it cannot be undone.
    if (
      flag === "expunged" &&
      !(await ask({
        title: n === 1 ? "Are you sure you want to delete this item permanently?" : `Are you sure you want to delete these ${n} items permanently?`,
        body: "This action cannot be undone.",
        action: "Delete",
        danger: true,
        symbol: "trash",
      }))
    ) {
      return;
    }
    // Hiding asks too, in Photos' words: where the photos go.
    if (
      flag === "hidden" && on &&
      !(await ask({
        title: n === 1 ? "Hide this photo?" : `Hide ${n} photos?`,
        body: `${n === 1 ? "It" : "They"} will be hidden from your library, and can be found in the Hidden album.`,
        action: "Hide",
        symbol: "hide",
      }))
    ) {
      return;
    }
    const before = photos;
    const ids = new Set(list);
    const was = list.map((id) => store.get(id)).filter((p): p is Photo => Boolean(p));
    // Which changes take a photo out of the view it is in.
    const leaves =
      flag === "expunged" ? true
      : inAlbum ? flag !== "favorite" && on
      : flag === "deleted" ? (on ? place !== "recentlyDeleted" : place === "recentlyDeleted")
      : flag === "hidden" ? (on ? place !== "hidden" : place === "hidden")
      : !on && place === "favorites";
    if (leaves) setPhotos((all) => all?.without(ids) ?? all);
    else store.patch(ids, (x) => ({ ...x, [flag]: on }));
    if (leaves) {
      setChosen((c) => new Set([...c].filter((id) => !ids.has(id))));
      // The viewer goes on to the next photo, or the one before at the end, or closes.
      setOpen((at) => {
        if (at === null || !photos) return at;
        const left = photos.length - n;
        return left <= 0 ? null : Math.min(at, left - 1);
      });
    }
    try {
      // The whole selection in one call: the library changes at once, Apple hears it batched.
      ownTouch(list);
      await api.photosSetFlag(list, flag, on);
      read().catch(() => {});
    } catch (e) {
      setPhotos(before);
      store.put(was);
      setError(messageOf(e));
      read().catch(() => {});
    }
  };

  /*
   * Albums, as Photos keeps them: every change shows at once and goes to iCloud behind it; a
   * refusal puts the list back and says why. A new album sorts first, as Photos puts it.
   */
  const createAlbum = async (name: string, then: string[], folder = false, parent: string | null = null) => {
    const before = albums;
    const temp: PhotoAlbum = { id: `new:${Date.now()}`, name, position: -1, changeTag: null, folder, parent };
    setAlbums((list) => [[temp, then.length], ...list]);
    try {
      const made = await api.photosAlbumCreate(name, parent ?? undefined, folder);
      setAlbums((list) => list.map((x) => (x[0].id === temp.id ? [made, x[1]] : x)));
      ownTouch(then);
      if (then.length) await api.photosAlbumAdd(made.id, then);
      read().catch(() => {});
    } catch (e) {
      setAlbums(before);
      setError(messageOf(e));
    }
  };

  const renameAlbum = async (a: PhotoAlbum, name: string) => {
    if (name === a.name) return;
    const before = albums;
    setAlbums((list) => list.map((x) => (x[0].id === a.id ? [{ ...x[0], name }, x[1]] : x)));
    if (album?.id === a.id) setAlbum({ ...a, name });
    try {
      await api.photosAlbumRename(a.id, name);
    } catch (e) {
      setAlbums(before);
      if (album?.id === a.id) setAlbum(a);
      setError(messageOf(e));
    }
  };

  const deleteAlbum = async (a: PhotoAlbum) => {
    // A folder, in icloud.com's words: it goes with every album in it.
    const sure = await ask({
      title: a.folder ? `Delete the folder “${a.name}”?` : `Delete the album “${a.name}”?`,
      body: a.folder
        ? "This will also delete all albums inside this folder. The photos in them stay in your library."
        : "The photos in it stay in your library.",
      action: "Delete",
      danger: true,
      symbol: "trash",
    });
    if (!sure) return;
    const before = albums;
    // Everything under a folder goes with it, here as on iCloud.
    const gone = new Set([a.id]);
    for (let grew = true; grew; ) {
      grew = false;
      for (const [x] of albums) {
        if (x.parent && gone.has(x.parent) && !gone.has(x.id)) {
          gone.add(x.id);
          grew = true;
        }
      }
    }
    setAlbums((list) => list.filter((x) => !gone.has(x[0].id)));
    if (folderPage && gone.has(folderPage)) setFolderPage(a.parent ?? null);
    if (album?.id === a.id) setAlbum(null);
    try {
      await api.photosAlbumDelete(a.id);
    } catch (e) {
      setAlbums(before);
      setError(messageOf(e));
    }
  };

  const addTo = async (a: PhotoAlbum, ids: string[]) => {
    const before = albums;
    setAlbums((list) => list.map((x) => (x[0].id === a.id ? [x[0], x[1] + ids.length] : x)));
    try {
      ownTouch(ids);
      await api.photosAlbumAdd(a.id, ids);
      toast({ message: ids.length === 1 ? `Added to ${a.name}` : `${ids.length} added to ${a.name}`, tone: "done" });
      read().catch(() => {});
    } catch (e) {
      setAlbums(before);
      setError(messageOf(e));
    }
  };

  /** Out of the album open; the photos stay in the library. */
  const removeFrom = async (gone: string[]) => {
    if (!album || viewOnly) return;
    const n = gone.length;
    const sure = await ask({
      title: `Remove ${n === 1 ? "this photo" : `${n} photos`} from the album?`,
      body: `${n === 1 ? "It stays" : "They stay"} in your library.`,
      action: "Remove",
    });
    if (!sure) return;
    const before = photos;
    const ids = new Set(gone);
    setPhotos((list) => list?.without(ids) ?? list);
    setAlbums((list) => list.map((x) => (x[0].id === album.id ? [x[0], Math.max(0, x[1] - n)] : x)));
    setOpen((at) => {
      if (at === null || !photos) return at;
      const left = photos.length - n;
      return left <= 0 ? null : Math.min(at, left - 1);
    });
    try {
      await api.photosAlbumRemove(album.id, [...ids]);
      read().catch(() => {});
    } catch (e) {
      setPhotos(before);
      setError(messageOf(e));
    }
  };

  // The selection in the order shown, by id, looked up where each one is — not by walking the
  // whole library on every click. Its photos need not be here: an action on it fetches any
  // that are not (store.load).
  const selected = useMemo(() => {
    if (!photos) return [];
    return [...chosen]
      .map((id) => [id, photos.indexOf(id)] as const)
      .filter(([, i]) => i >= 0)
      .sort((x, y) => x[1] - y[1])
      .map(([id]) => id);
  }, [photos, chosen]);
  const kindsOf = (ids: string[]) => ids.map((id) => photos?.kind(photos.indexOf(id)) ?? "image");
  const anyPlain = (ids: string[]) => ids.some((id) => !(photos?.favorite(photos.indexOf(id)) ?? false));

  // A photo found by the search across everything: the whole library shown, and the photo
  // opened in the viewer once the library's photos are here.
  const revealing = useRef<string | null>(null);
  useReveal("photos", (r) => {
    revealing.current = r.id;
    setAlbum(null);
    setAlbumsPage(false);
    setSharedPage(false);
    endSearch();
    setView("library");
  });
  useEffect(() => {
    const id = revealing.current;
    if (!id || view !== "library" || album) return;
    const at = photos?.indexOf(id) ?? -1;
    if (at < 0) return;
    revealing.current = null;
    void openAt(at);
  }, [photos, view, album]); // eslint-disable-line react-hooks/exhaustive-deps

  const openAt = async (index: number) => {
    if (!photos) return;
    // Opened from a tile it is here; from a search, its page is fetched first.
    if (!photos.at(index)) await photos.need(index - PAGE / 2, index + PAGE / 2);
    const p = photos.at(index);
    if (!p) return;
    // Open at full quality: the whole picture decoded first — from disk, a moment — but never
    // waited on for longer than a blink (a click must answer at once); past that it opens on
    // its thumbnail and screen-sized picture, and the whole one follows.
    if (p.kind !== "video") await Promise.race([warm(p), new Promise((r) => window.setTimeout(r, 150))]);
    stopClosing();
    const tile = document.querySelector<HTMLElement>(`.px-tile[data-id="${CSS.escape(p.id)}"]`);
    origin.current = tile?.getBoundingClientRect() ?? null;
    land();
    setOpen(index);
  };

  // A click as Photos takes it: alone it selects that one, with Ctrl (⌘) it adds or takes
  // away, with Shift it selects everything from the last one chosen.
  const pick = (index: number, e: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }) => {
    const id = photos?.ids[index];
    if (!photos || !id) return;
    if (e.shiftKey && anchor.current !== null) {
      const [a, b] = [Math.min(anchor.current, index), Math.max(anchor.current, index)];
      const span = photos.ids.slice(a, b + 1);
      setChosen((c) => new Set([...(e.ctrlKey || e.metaKey ? c : []), ...span]));
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      setChosen((c) => {
        const next = new Set(c);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        return next;
      });
    } else {
      setChosen(new Set([id]));
    }
    anchor.current = index;
  };

  // The tile to land on from another by an arrow: the nearest in that direction as laid out,
  // so moving down from the last row of a month goes into the next month's first row.
  const neighbour = (from: number, key: string): number | null => {
    const count = photos?.length ?? 0;
    if (key === "ArrowLeft") return from > 0 ? from - 1 : null;
    if (key === "ArrowRight") return from < count - 1 ? from + 1 : null;
    // Up and down follow the grid as laid out: the same column a row away, and across a
    // month's edge into the next month's first row or the last month's last. Worked out
    // from the months themselves, so it reaches months that are not built yet.
    const k = months.findIndex((m) => m.items.length && from >= m.items[0].index && from <= m.items[m.items.length - 1].index);
    if (k < 0) return null;
    const m = months[k];
    const at = from - m.items[0].index;
    const col = at % across;
    if (key === "ArrowDown") {
      if (at + across < m.items.length) return from + across;
      // Still a row below in this month, but shorter: its last photo.
      if (Math.floor(at / across) < Math.floor((m.items.length - 1) / across)) return m.items[m.items.length - 1].index;
      const next = months[k + 1];
      return next ? next.items[Math.min(col, next.items.length - 1)].index : null;
    }
    if (at - across >= 0) return from - across;
    const last = months[k - 1];
    if (!last) return null;
    const lastRow = Math.floor((last.items.length - 1) / across) * across;
    return last.items[Math.min(lastRow + col, last.items.length - 1)].index;
  };

  // The tile at an index, focused and in view; built first if its month is not yet.
  const reach = (to: number) => {
    const sel = `.px-scroll .px-tile[data-index="${to}"]`;
    let tries = 0;
    const go = () => {
      const tile = document.querySelector<HTMLElement>(sel);
      if (tile) {
        tile.focus({ preventScroll: true });
        tile.scrollIntoView({ block: "nearest" });
        return;
      }
      const k = months.findIndex((m) => m.items.some((x) => x.index === to));
      document.querySelectorAll<HTMLElement>(".px-scroll .px-month")[k]?.scrollIntoView({ block: "nearest" });
      if (++tries < 12) requestAnimationFrame(go);
    };
    go();
  };

  // The grid's keys, while no photo is open and nothing is being typed.
  useEffect(() => {
    if (open !== null) return undefined;
    const onKey = (e: KeyboardEvent) => {
      if (!live.current) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest?.("input, textarea, [contenteditable], .row-menu, [role=dialog]") || naming) return;
      // The arrows in the sidebar move through the sidebar (Library.tsx), never the grid.
      if (t?.closest?.(".px-side") && /^(Arrow|Home$|End$)/.test(e.key)) return;
      const list = photos?.ids ?? [];
      const ctrl = e.ctrlKey || e.metaKey;
      // Ctrl+A is the photos' wherever the focus rests on this screen — the app list keeps it
      // after Photos is chosen there — never the page's text.
      const selectAll = ctrl && e.key.toLowerCase() === "a";
      if (!selectAll && !t?.closest?.(".px") && t !== document.body) return;
      // Ctrl+[ — and Escape, with nothing selected — step out a level, as Photos' Back does:
      // All Photos to Months, Months to Years.
      if (periodsHere && level !== "years" && !e.altKey && !e.shiftKey && !chosen.size
        && ((ctrl && (e.key === "[" || e.code === "BracketLeft")) || (!ctrl && e.key === "Escape"))) {
        e.preventDefault();
        goLevel(level === "all" ? "months" : "years");
        return;
      }
      // Years or Months showing: the grid behind them takes no keys.
      if (away) return;
      if (ctrl && e.key.toLowerCase() === "a") {
        setChosen(new Set(list));
      } else if (ctrl && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "c" && selected.length && place !== "recentlyDeleted") {
        // Ctrl+C: the selected photo on the clipboard, to paste into any app.
        void copy(selected);
      } else if (e.key === "Escape" && chosen.size) {
        setChosen(new Set());
      } else if ((e.key === "Enter" || e.key === " ") && t?.closest?.(".px-tile[data-index]") && !t.closest(".px-tile")!.matches('[aria-selected="true"]')) {
        // The photo the keys rest on — Tab lands on one before any is chosen — opens.
        e.preventDefault();
        const at = Number(t.closest(".px-tile")!.getAttribute("data-index"));
        setChosen(new Set([list[at]]));
        anchor.current = at;
        openAt(at);
      } else if ((e.key === "Enter" || e.key === " ") && selected.length) {
        openAt(photos!.indexOf(selected[0]));
      } else if (e.key.startsWith("Arrow") && list.length) {
        const from = anchor.current ?? (e.key === "ArrowLeft" || e.key === "ArrowUp" ? list.length : -1);
        const to = from < 0 ? 0 : from >= list.length ? list.length - 1 : neighbour(from, e.key);
        if (to === null) return e.preventDefault();
        if (e.shiftKey && anchor.current !== null) setChosen((c) => new Set([...c, list[to]]));
        else setChosen(new Set([list[to]]));
        anchor.current = to;
        reach(to);
      } else if ((e.key === "Delete" || e.key === "Backspace") && selected.length && !viewOnly) {
        if (inAlbum) void removeFrom(selected);
        else if (place !== "recentlyDeleted") void changeAll(selected, "deleted", true);
        else void changeAll(selected, "expunged", true);
      } else if (!ctrl && (e.key === "=" || e.key === "+" || e.key === "-")) {
        // = and − size the thumbnails, as in icloud.com's grid.
        zoomBy(e.key === "-" ? -1 : 1);
      } else if (!ctrl && e.key.toLowerCase() === "i" && selected.length) {
        // I: the selected photo, opened with its details.
        setInfo(true);
        openAt(photos!.indexOf(selected[0]));
      } else if ((ctrl ? e.key.toLowerCase() === "l" : e.key.toLowerCase() === "h") && selected.length && place !== "recentlyDeleted") {
        // ⌘L in Photos: hide, or in Hidden, unhide.
        void changeAll(selected, "hidden", place !== "hidden");
      } else if ((e.key === "." || (!ctrl && e.key.toLowerCase() === "f")) && selected.length && place !== "recentlyDeleted") {
        // As Photos: if any is not a favorite, all become favorites; else all stop being.
        void changeAll(selected, "favorite", anyPlain(selected));
      } else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  /*
   * Sweeping out a rectangle from the space around the photos selects every photo it touches,
   * as on a Mac — with Ctrl (⌘) or Shift, adding to what was already chosen. Worked out from
   * the grid's geometry, not by measuring tiles: months far from the screen are not built, and
   * a sweep that scrolls the view still reaches them. The view scrolls on its own while the
   * pointer is held past its top or bottom edge.
   */
  const sweepBox = useRef<HTMLDivElement | null>(null);
  const swept = useRef(false);
  const sweepStart = (e: React.PointerEvent<HTMLDivElement>) => {
    swept.current = false;
    const el = scroller.current;
    if (!el || e.button !== 0 || e.pointerType === "touch") return;
    const t = e.target as HTMLElement;
    if (t.closest(".px-tile, button, a, input, .px-sweep") || !photos?.length) return;
    // Past the scrollbar is the scrollbar's.
    if (e.clientX >= el.getBoundingClientRect().left + el.clientWidth) return;
    const box0 = el.getBoundingClientRect();
    const start = { x: e.clientX - box0.left, y: e.clientY - box0.top + el.scrollTop };
    const base = e.ctrlKey || e.metaKey || e.shiftKey ? new Set(chosen) : new Set<string>();
    let pointer = { x: e.clientX, y: e.clientY };
    let moved = false;
    let tick = 0;
    // Nothing drawn yet: the first frame sets the selection even to none, so a sweep from the
    // space over no photo lets go of what was chosen (as "" it matched, and the old one stayed).
    let last: string | null = null;
    // What the sweep has chosen so far, as it set it (the state is a render behind).
    let picked: Set<string> | null = null;
    // The grid's geometry, read once: each group's block (built or not), and the cells' pitch.
    const groups = [...el.querySelectorAll<HTMLElement>(".px-month")].map((sec, k) => {
      const r = photosArea(sec);
      return { top: r.top - box0.top + el.scrollTop, height: r.height, k };
    });
    const style = getComputedStyle(el);
    const inner = el.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
    const [gap, pad] = fit ? [14, 14] : [2, 0];
    const pitch = (inner - 2 * pad + gap) / across;
    const left0 = parseFloat(style.paddingLeft) + pad;
    const draw = () => {
      const box = el.getBoundingClientRect();
      const now = { x: Math.max(0, Math.min(el.clientWidth, pointer.x - box.left)), y: pointer.y - box.top + el.scrollTop };
      const [x1, x2] = [Math.min(start.x, now.x), Math.max(start.x, now.x)];
      const [y1, y2] = [Math.min(start.y, now.y), Math.max(start.y, now.y)];
      const shown = sweepBox.current;
      if (shown) {
        shown.hidden = false;
        shown.style.cssText = `left:${x1}px;top:${y1}px;width:${x2 - x1}px;height:${y2 - y1}px`;
      }
      const c1 = Math.max(0, Math.floor((x1 - left0) / pitch));
      const c2 = Math.min(across - 1, Math.floor((x2 - left0) / pitch));
      const next = new Set(base);
      if (c2 >= c1) {
        for (const g of groups) {
          if (g.top > y2 || g.top + g.height < y1) continue;
          const items = months[g.k]?.items;
          if (!items?.length) continue;
          const rows = Math.ceil(items.length / across);
          const rowH = g.height / rows;
          const r1 = Math.max(0, Math.floor((y1 - g.top) / rowH));
          const r2 = Math.min(rows - 1, Math.floor((y2 - g.top) / rowH));
          for (let r = r1; r <= r2; r++) {
            for (let c = c1; c <= c2; c++) {
              // Only the photo itself, not the hairline gap after it.
              if (x2 < left0 + c * pitch || x1 > left0 + c * pitch + pitch - gap) continue;
              const it = items[r * across + c];
              if (it) next.add(it.id);
            }
          }
        }
      }
      const key = [...next].join();
      if (key !== last) {
        last = key;
        picked = next;
        setChosen(next);
      }
    };
    const loop = () => {
      tick = 0;
      const box = el.getBoundingClientRect();
      const top = box.top + (parseFloat(style.paddingTop) || 0) - 12;
      const by = pointer.y < top ? -Math.min(40, (top - pointer.y) / 2 + 4) : pointer.y > box.bottom - 24 ? Math.min(40, (pointer.y - box.bottom + 24) / 2 + 4) : 0;
      if (by) el.scrollTop += by;
      draw();
      if (by) tick = requestAnimationFrame(loop);
    };
    const move = (ev: PointerEvent) => {
      pointer = { x: ev.clientX, y: ev.clientY };
      if (!moved && Math.hypot(ev.clientX - e.clientX, ev.clientY - e.clientY) < 4) return;
      moved = true;
      if (!tick) tick = requestAnimationFrame(loop);
    };
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      cancelAnimationFrame(tick);
      // A move not drawn yet — the button let go within the frame — is drawn now: the
      // selection is where the pointer was let go, never a frame behind it.
      if (moved && tick) draw();
      tick = 0;
      if (sweepBox.current) sweepBox.current.hidden = true;
      if (moved) {
        swept.current = true;
        // The keys go on from the sweep's last photo.
        const ids = [...(picked ?? chosenNow.current)];
        const at = ids.length ? Math.max(...ids.map((id) => photos?.indexOf(id) ?? -1)) : -1;
        anchor.current = at >= 0 ? at : anchor.current;
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  };
  const chosenNow = useRef(chosen);
  chosenNow.current = chosen;

  // A slideshow of what is selected when that is more than one photo, else of the whole view
  // from this photo on, as Photos starts one.
  const startShow = (from: Photo) => {
    const ids = selected.length > 1 && chosen.has(from.id) ? selected : (photos?.ids ?? []);
    setMenu(null);
    setShow({ ids, start: Math.max(0, ids.indexOf(from.id)) });
  };

  const albumEntries = (ids: string[]): MenuEntry[] => [
    ...albums.filter(([a]) => !a.folder && a.id !== album?.id && !a.id.startsWith("new:")).map(([a]) => ({ label: a.name, run: () => void addTo(a, ids) })),
    ...(albums.length ? [null] : []),
    { label: "New Album", run: () => setNaming({ id: null, then: ids, draft: "" }) },
  ];

  const photoMenu = (p: Photo, at: { x: number; y: number; alignRight: boolean }) => {
    // A right-click on a selected photo acts on the whole selection; on any other, it
    // selects that one first, as Photos does.
    const group = chosen.has(p.id) && selected.length > 1 ? selected : [p.id];
    if (!chosen.has(p.id)) {
      setChosen(new Set([p.id]));
      anchor.current = photos?.indexOf(p.id) ?? null;
    }
    if (group.length > 1) {
      const n = group.length;
      const plain = anyPlain(group);
      setMenu({
        at,
        entries: viewOnly
          ? [
              { label: `Download ${n} Items`, run: () => void downloadAs(group, "originals") },
              { label: "More Download Options", run: () => void store.load(group).then(setOptions, (e) => setError(messageOf(e))) },
              null,
              { label: "Start Slideshow", run: () => startShow(p) },
            ]
          : place === "recentlyDeleted" && !inAlbum
            ? [
                { label: `Recover ${n} Items`, run: () => void changeAll(group, "deleted", false) },
                null,
                { label: `Delete ${n} Items`, danger: true, run: () => void changeAll(group, "expunged", true) },
              ]
            : [
                { label: plain ? "Favorite" : "Unfavorite", run: () => void changeAll(group, "favorite", plain) },
                null,
                { label: `Download ${n} Items`, run: () => void downloadAs(group, "originals") },
                { label: "More Download Options", run: () => void store.load(group).then(setOptions, (e) => setError(messageOf(e))) },
                null,
                { label: "Start Slideshow", run: () => startShow(p) },
                { label: "Add to Album", submenu: albumEntries(group) },
                null,
                place === "hidden" && !inAlbum
                  ? { label: `Unhide ${n} Photos`, run: () => void changeAll(group, "hidden", false) }
                  : { label: `Hide ${n} Photos`, run: () => void changeAll(group, "hidden", true) },
                inAlbum
                  ? { label: `Remove ${n} Photos from Album`, danger: true, run: () => void removeFrom(group) }
                  : { label: `Delete ${n} Photos`, danger: true, run: () => void changeAll(group, "deleted", true) },
              ],
      });
      return;
    }
    const entries: MenuEntry[] = viewOnly
      ? [
          ...(p.kind !== "video" ? [{ label: "Copy", run: () => void copy([p.id]) }] : []),
          { label: "Download", run: () => void download(p) },
          { label: "More Download Options", run: () => setOptions([p]) },
          { label: "Start Slideshow", run: () => startShow(p) },
        ]
      : place === "recentlyDeleted" && !inAlbum
        ? [
            { label: "Recover", run: () => void change(p, "deleted", false) },
            null,
            { label: "Delete", danger: true, run: () => void change(p, "expunged", true) },
          ]
        : [
            { label: p.favorite ? "Unfavorite" : "Favorite", run: () => void change(p, "favorite", !p.favorite) },
            // A picture to paste anywhere; a video has none to give.
            ...(p.kind !== "video" ? [{ label: "Copy", run: () => void copy([p.id]) }] : []),
            { label: "Download", run: () => void download(p) },
            { label: "More Download Options", run: () => setOptions([p]) },
            { label: "Start Slideshow", run: () => startShow(p) },
            null,
            { label: "Add to Album", submenu: albumEntries([p.id]) },
            null,
            place === "hidden" && !inAlbum
              ? { label: "Unhide Photo", run: () => void change(p, "hidden", false) }
              : { label: "Hide Photo", run: () => void change(p, "hidden", true) },
            inAlbum
              ? { label: "Remove from Album", danger: true, run: () => void removeFrom([p.id]) }
              : { label: "Delete", danger: true, run: () => void change(p, "deleted", true) },
          ];
    setMenu({ at, entries });
  };

  // The tiles' handlers, one object for the life of the screen; each call reaches the newest.
  /*
   * Dragging photos onto an album in the sidebar puts them in it, as on a Mac: what is dragged
   * is the photo, or the whole selection when the photo is part of it.
   */
  const dragging2 = useRef<string[] | null>(null);
  const [dropOn, setDropOn] = useState<string | null>(null);
  const dragFrom = (p: Photo, e: React.DragEvent) => {
    const ids = chosen.has(p.id) ? selected : [p.id];
    if (!chosen.has(p.id)) {
      setChosen(new Set([p.id]));
      anchor.current = photos?.indexOf(p.id) ?? null;
    }
    dragging2.current = ids;
    e.dataTransfer.effectAllowed = "copy";
    e.dataTransfer.setData("application/x-quince-photos", ids.join(","));
    // A stack of photos with how many, as Photos drags them.
    const img = e.currentTarget.querySelector("img");
    if (img) {
      const ghost = document.createElement("div");
      ghost.className = "px-ghost";
      const pic = img.cloneNode() as HTMLImageElement;
      ghost.appendChild(pic);
      if (ids.length > 1) {
        const n = document.createElement("span");
        n.textContent = String(ids.length);
        ghost.appendChild(n);
      }
      carryPicture(e, ghost, 36, 36);
    }
  };
  const dropTarget = (a: PhotoAlbum) => (a.folder ? {} : {
    onDragOver: (e: React.DragEvent) => {
      if (!dragging2.current || a.id.startsWith("new:") || album?.id === a.id) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      if (dropOn !== a.id) setDropOn(a.id);
    },
    onDragLeave: (e: React.DragEvent) => {
      // Passing over the row's own symbol or name is not leaving it. Where the engine does not
      // say where the pointer went, the row's own bounds do.
      if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
      const box = e.currentTarget.getBoundingClientRect();
      if (e.clientX > box.left && e.clientX < box.right && e.clientY > box.top && e.clientY < box.bottom) return;
      setDropOn((d) => (d === a.id ? null : d));
    },
    onDrop: (e: React.DragEvent) => {
      const ids = dragging2.current;
      dragging2.current = null;
      setDropOn(null);
      if (!ids || a.id.startsWith("new:")) return;
      e.preventDefault();
      void addTo(a, ids);
    },
  });

  const acts = useRef({ pick, openAt, photoMenu, dragFrom, zoomBy });
  acts.current = { pick, openAt, photoMenu, dragFrom, zoomBy };
  const act = useMemo<Acts>(
    () => ({
      pick: (i, e) => acts.current.pick(i, e),
      open: (i) => acts.current.openAt(i),
      menu: (p, e) => acts.current.photoMenu(p, { x: e.clientX, y: e.clientY, alignRight: false }),
      drag: (p, e) => acts.current.dragFrom(p, e),
      dragEnd: () => {
        dragging2.current = null;
        setDropOn(null);
      },
    }),
    [],
  );

  // Enter submits and closes the field, and closing it blurs it, which would submit again
  // with the same name still in hand: one album per name typed.
  const namedOnce = useRef(false);
  useEffect(() => {
    if (naming) namedOnce.current = false;
  }, [naming === null]); // eslint-disable-line react-hooks/exhaustive-deps
  const finishNaming = () => {
    if (!naming || namedOnce.current) return;
    namedOnce.current = true;
    const name = naming.draft.trim();
    setNaming(null);
    if (!name) return;
    if (naming.id === null) void createAlbum(name, naming.then, naming.folder ?? false, naming.parent ?? null);
    else {
      const a = albums.find(([x]) => x.id === naming.id)?.[0];
      if (a) void renameAlbum(a, name);
    }
  };

  const nameField = (
    <input
      className="field px-naming"
      aria-label="Album name"
      autoFocus
      value={naming?.draft ?? ""}
      placeholder="Untitled Album"
      onChange={(e) => setNaming((n) => (n ? { ...n, draft: e.target.value } : n))}
      onKeyDown={(e) => {
        if (e.key === "Enter") finishNaming();
        else if (e.key === "Escape") {
          namedOnce.current = true;
          setNaming(null);
        }
        e.stopPropagation();
      }}
      onBlur={finishNaming}
      onFocus={(e) => e.currentTarget.select()}
    />
  );

  /*
   * Title and caption, as Photos' Info has them: typed in place, Return (in the title) or
   * leaving them saves both at once — moving from one to the other is not leaving — and Escape
   * puts back what was there. Shown at once; put back if iCloud refuses.
   */
  const titleField = useRef<HTMLInputElement | null>(null);
  const captionField = useRef<HTMLTextAreaElement | null>(null);
  const grow = (el: HTMLTextAreaElement) => {
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  };
  const wordKeys = (e: React.KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>, was: string | null | undefined) => {
    if (e.key === "Enter" && (e.currentTarget.tagName === "INPUT" || !e.shiftKey)) {
      e.preventDefault();
      e.currentTarget.blur();
    } else if (e.key === "Escape") {
      e.currentTarget.value = was ?? "";
      e.currentTarget.blur();
      e.stopPropagation();
    }
  };
  const wordsLeft = (p: Photo, e: React.FocusEvent) => {
    if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
    void words(p, titleField.current?.value ?? p.title ?? "", captionField.current?.value ?? p.caption ?? "");
  };
  const keywords = async (p: Photo, words: string[]) => {
    store.patch([p.id], (x) => ({ ...x, keywords: words }));
    try {
      ownTouch([p.id]);
      await api.photoSetKeywords(p.id, words);
    } catch (e) {
      store.put([p]);
      setError(messageOf(e));
    }
  };

  const rate = async (p: Photo, rating: number) => {
    const before = photos;
    const leaves = view === "rated" && rating === 0;
    if (leaves) setPhotos((all) => all?.without(new Set([p.id])) ?? all);
    else store.patch([p.id], (x) => ({ ...x, rating }));
    try {
      ownTouch([p.id]);
      await api.photoSetRating(p.id, rating);
      read().catch(() => {});
    } catch (e) {
      setPhotos(before);
      store.put([p]);
      setError(messageOf(e));
    }
  };

  const words = async (p: Photo, title: string, caption: string) => {
    const [t, c] = [title.trim(), caption.trim()];
    if (t === (p.title ?? "") && c === (p.caption ?? "")) return;
    store.patch([p.id], (x) => ({ ...x, title: t || null, caption: c || null }));
    try {
      ownTouch([p.id]);
      await api.photoSetText(p.id, t, c);
    } catch (e) {
      store.put([p]);
      setError(messageOf(e));
    }
  };

  const play = async (p: Photo) => {
    try {
      await api.photoPlay(p.id);
    } catch (e) {
      setError(messageOf(e));
    }
  };

  /**
   * Copy, as in Photos: the photo on the clipboard as a picture, to paste into any app. One
   * picture — the clipboard holds one, on Linux and Windows alike — so with several chosen it
   * is the first photo of them (a video is never copied). Quiet: "Copied" once it is there, and
   * a word while it is made only when that takes a moment (a large HEIC, or one not here yet).
   */
  const copying = useRef(0);
  const copy = async (ids: string[]) => {
    const mine = ++copying.current;
    let picked: Photo | undefined;
    try {
      picked = (await store.load(ids)).find((x) => x.kind !== "video");
    } catch (e) {
      setError(messageOf(e));
      return;
    }
    if (!picked) return;
    let waitNote = 0;
    const slow = window.setTimeout(() => {
      waitNote = toast({ message: "Copying photo", duration: null });
    }, 300);
    try {
      const png = await api.photoCopy(picked.id);
      // Not on Linux or Windows, where the app has put it there itself: the webview's own
      // clipboard, which takes a PNG as the system's picture.
      if (png.byteLength) await navigator.clipboard.write([new ClipboardItem({ "image/png": new Blob([png], { type: "image/png" }) })]);
      if (mine === copying.current) toast({ message: ids.length > 1 ? "Copied the first photo" : "Copied", tone: "done" });
    } catch (e) {
      if (mine === copying.current) setError(messageOf(e));
    } finally {
      window.clearTimeout(slow);
      if (waitNote) dismissToast(waitNote);
    }
  };

  const download = async (p: Photo) => {
    try {
      const where = await api.photoDownload(p.id);
      toastSaved(where);
    } catch (e) {
      setError(messageOf(e));
    }
  };

  /** Adjust Date and Time open for this photo. */
  const [adjusting, setAdjusting] = useState<Photo | null>(null);
  const redate = async (p: Photo, takenAt: number) => {
    setAdjusting(null);
    store.patch([p.id], (x) => ({ ...x, takenAt }));
    try {
      ownTouch([p.id]);
      await api.photoSetDate(p.id, takenAt);
      read().catch(() => {});
    } catch (e) {
      store.put([p]);
      setError(messageOf(e));
    }
  };

  /** Download Options open for these photos. */
  const [options, setOptions] = useState<Photo[] | null>(null);
  const downloadAs = async (list: string[], want: Want) => {
    setOptions(null);
    let last = "";
    try {
      for (const id of list) last = await api.photoDownload(id, want);
      toastSaved(last, list.length);
    } catch (e) {
      setError(messageOf(e));
    }
  };

  // Uploads report as they go; a finished one never goes back, as events can pass each other.
  // The library itself is redrawn by the sync each landed photo starts.
  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    listen<UploadEvent>(PHOTO_UPLOAD, (event) => {
      const u = event.payload;
      // "planned" says a batch is all listed; it rides on its first file's id, and kept, it
      // took that file's place — the first photo of each batch went missing from the panel.
      if (u.state === "planned") return;
      setUploads((was) => {
        const before = was.get(u.id);
        if (before && (before.state === "done" || before.state === "failed" || before.state === "cancelled")) return was;
        return new Map(was).set(u.id, u);
      });
    })
      .then((unlisten) => {
        const quietly = () => {
          try {
            void Promise.resolve(unlisten() as unknown).catch(() => {});
          } catch {
            // Nothing was listening.
          }
        };
        if (cancelled) quietly();
        else stop = quietly;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);
  const dismissUploads = useCallback(() => setUploads(new Map()), []);

  const add = useCallback(async (paths: string[]) => {
    if (paths.length === 0) return;
    try {
      // With an album open, what is added goes into it too, as Photos files an import.
      await api.photosUpload(paths, album?.id);
    } catch (e) {
      setError(messageOf(e));
    }
  }, [album]);

  const choose = async () => {
    try {
      const chosen = await pickOpen({
        multiple: true,
        title: "Add to iCloud Photos",
        filters: [{ name: "Photos and Videos", extensions: KINDS }],
      });
      if (chosen) await add(chosen);
    } catch (e) {
      setError(messageOf(e));
    }
  };

  // Photos and videos dropped on the window go into the library. Outside the app shell there
  // is no webview to ask; the button still works.
  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          // Photos of the library's own being dragged to an album are not files arriving.
          if (dragging2.current) return;
          // Only while Photos is the screen in front: a file dropped onto a note, with Photos
          // kept mounted behind it, went up to Photos as well. Drive guards the same way.
          if (!frontRef.current) {
            setDropping(false);
            return;
          }
          if (event.payload.type === "over") setDropping(true);
          else if (event.payload.type === "leave") setDropping(false);
          else if (event.payload.type === "drop") {
            setDropping(false);
            void add(event.payload.paths);
          }
        })
        .then((unlisten) => {
          if (cancelled) unlisten();
          else stop = unlisten;
        })
        .catch(() => {});
    } catch {
      // No drag and drop here.
    }
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [add]);

  // Under the title, the stretch of time shown — "September – October 2026" — as Photos has it.
  const span = useMemo(() => {
    if (!photos || photos.length === 0 || photos.first === null || photos.last === null) return null;
    const [a, b] = [new Date(photos.first * 1000), new Date(photos.last * 1000)];
    const month = (d: Date, year: boolean) => d.toLocaleDateString(undefined, year ? { month: "long", year: "numeric" } : { month: "long" });
    if (a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth()) return month(b, true);
    return `${month(a, a.getFullYear() !== b.getFullYear())} – ${month(b, true)}`;
  }, [photos]);
  // At the foot of the library, what it holds: "132 Photos, 2 Videos".
  const tally = useMemo(() => {
    if (!photos || photos.length === 0) return null;
    let videos = 0;
    for (const m of photos.marks) if ((m & 3) === 1) videos++;
    const stills = photos.length - videos;
    const part = (n: number, one: string) => `${n.toLocaleString()} ${n === 1 ? one : `${one}s`}`;
    return [stills && part(stills, "Photo"), videos && part(videos, "Video")].filter(Boolean).join(", ");
  }, [photos]);

  // Scrolled, the line under the title names the month at the top of the view, as Photos
  // does; read once a frame at most, and set only when it changes.
  const [here, setHere] = useState<string | null>(null);
  /** The month picker, open under its button. */
  const [jumping, setJumping] = useState<DOMRect | null>(null);
  // Each month the grid shows, with a photo from it, for the picker: the grid's own months, so
  // the two always agree.
  const monthCovers = useMemo<MonthCover[]>(() => {
    if (drawn.album || drawn.view === "rated") return [];
    // By day, a month is several groups: the picker lists it once, with its first photo.
    const seen = new Set<string>();
    return months
      .filter((m) => m.month && m.items.length && !seen.has(m.month) && seen.add(m.month))
      .map((m) => {
        const d = new Date(photos!.keys[m.items[0].index] * 1000);
        return { label: m.month, year: d.getFullYear(), month: d.getMonth(), cover: { id: m.items[0].id } };
      });
  }, [months, drawn]); // eslint-disable-line react-hooks/exhaustive-deps
  /**
   * The grid scrolled so `offset` (how far what is wanted sits below the top, measured afresh)
   * is nothing. Months drawn as they come into view take their real height, and those above the
   * one chosen move it. Chromium's scroll anchoring holds it in place; WebKit has none, and a
   * month twenty years down landed a year off. So, for a moment after an instant jump, it is held
   * at the top — let go at once if the person scrolls or presses a key themselves.
   */
  const scrollHeld = (el: HTMLElement, offset: () => number | null, instant: boolean) => {
    const first = offset();
    if (first === null) return;
    const now = instant || still();
    el.scrollTo({ top: el.scrollTop + first, behavior: now ? "auto" : "smooth" });
    if (!now) return;
    let frames = 0;
    let held = true;
    const letGo = () => { held = false; };
    const opts = { once: true, passive: true } as const;
    el.addEventListener("wheel", letGo, opts);
    el.addEventListener("pointerdown", letGo, opts);
    el.addEventListener("touchstart", letGo, opts);
    window.addEventListener("keydown", letGo, { once: true });
    const hold = () => {
      const off = held ? offset() : null;
      if (off === null) return;
      if (Math.abs(off) > 1) el.scrollTop += off;
      if (++frames < 30) requestAnimationFrame(hold);
      else {
        el.removeEventListener("wheel", letGo);
        el.removeEventListener("pointerdown", letGo);
        el.removeEventListener("touchstart", letGo);
        window.removeEventListener("keydown", letGo);
      }
    };
    requestAnimationFrame(hold);
  };
  const goToMonth = (label: string, instant = false) => {
    const el = scroller.current;
    const at = el?.querySelector<HTMLElement>(`.px-month[data-month="${CSS.escape(label)}"]`);
    // Measured against the scroller itself: offsetTop counts from whatever positioned box
    // happens to hold the month, which is not always the scroller.
    if (el && at) scrollHeld(el, () => (at.isConnected ? at.getBoundingClientRect().top - el.getBoundingClientRect().top - 4 : null), instant);
  };
  /**
   * How far below the grid's top the tile at an index is: its own box, or — its month not built
   * yet — where its row will be in the month's block, whose height is exactly its rows'.
   */
  const tileBelow = (index: number): number | null => {
    const el = scroller.current;
    if (!el) return null;
    const box = el.getBoundingClientRect();
    const tile = el.querySelector<HTMLElement>(`.px-tile[data-index="${index}"]`);
    if (tile) return tile.getBoundingClientRect().top - box.top;
    const g = months.findIndex((m) => m.items.length && index >= m.items[0].index && index <= m.items[m.items.length - 1].index);
    const sec = el.querySelectorAll<HTMLElement>(".px-month")[g];
    if (!sec) return null;
    const row = Math.floor((index - months[g].items[0].index) / across);
    const r = photosArea(sec);
    return r.top - box.top + (row * r.height) / Math.ceil(months[g].items.length / across);
  };
  /** The tile at an index brought to the top of the grid, under the bar. */
  const goToPhoto = (index: number) => {
    const el = scroller.current;
    if (!el) return;
    const pad = parseFloat(getComputedStyle(el).scrollPaddingTop) || 0;
    scrollHeld(el, () => { const t = tileBelow(index); return t === null ? null : t - pad; }, true);
  };
  const scroller = useRef<HTMLDivElement | null>(null);
  const looking = useRef(false);
  // Read a few times a second, not every frame, and by halving rather than going down every
  // month: at the foot of three hundred months, measuring each one on every frame was work
  // the scroll itself was waiting for.
  const onScroll = useCallback(() => {
    if (looking.current) return;
    looking.current = true;
    window.setTimeout(() => {
      looking.current = false;
      const el = scroller.current;
      if (!el || album) return setHere(null);
      if (el.scrollTop < 24) return setHere(null);
      const edge = el.getBoundingClientRect().top + 96;
      const months = el.querySelectorAll<HTMLElement>(".px-month");
      let [lo, hi] = [0, months.length - 1];
      while (lo < hi) {
        const mid = (lo + hi) >> 1;
        if (months[mid].getBoundingClientRect().bottom > edge) hi = mid;
        else lo = mid + 1;
      }
      setHere(months[lo]?.dataset.month || months[lo]?.getAttribute("aria-label") || null);
    }, 120);
  }, [album]);
  useEffect(() => setHere(null), [view, album]);

  /*
   * Years, Months and All Photos, as Photos on a Mac steps through its Library: only the
   * Library has them (every other view, an album and a search are their own grid). Years and
   * Months are drawn in place of the grid, which stays as it was beneath them — built and kept
   * current by every sync, its scroll put back — so coming back to All Photos is instant and
   * finds it where it was.
   */
  const periodsHere = drawn.view === "library" && !drawn.album && !drawn.query;
  const away = periodsHere && level !== "all";
  const awayRef = useRef(away);
  awayRef.current = away;
  // The library's months, with their counts and key photos, asked of the library whenever Years
  // or Months is shown and again whenever the library changes under them.
  const [periodMonths, setPeriodMonths] = useState<PhotoMonth[] | null>(null);
  useEffect(() => {
    if (!away) return undefined;
    let mine = true;
    api.photosMonths().then(
      (m) => mine && setPeriodMonths((was) => (same(was, m) ? was : m)),
      (e) => mine && setError(messageOf(e)),
    );
    return () => {
      mine = false;
    };
  }, [away, photos]);
  /** Where a level just shown lands: the place carried from the last one (a new object each time). */
  const [landAt, setLandAt] = useState<Place | null>(null);
  /** The place at the top of Years or Months now (kept by them). */
  const periodAt = useRef<Place | null>(null);
  /** All Photos as it was left: its scroll, and the month at its top. */
  const leftAll = useRef<{ top: number; place: Place | null } | null>(null);
  /** All Photos about to be shown, and the place to show it at. */
  const toAll = useRef<Place | null | undefined>(undefined);
  /** The month at the top of All Photos. */
  const gridPlace = (): Place | null => {
    // Sorted by Date Added the grid's months are when photos came in; Years and Months are by
    // when they were taken, so the place is the month the photo at the top was taken in.
    if (sort?.by === "added" && photos) {
      const el = scroller.current;
      const box = el?.getBoundingClientRect();
      const pad = el ? parseFloat(getComputedStyle(el).scrollPaddingTop) || 0 : 0;
      const top = box && [...el!.querySelectorAll<HTMLElement>(".px-tile")].find((t) => t.getBoundingClientRect().bottom > box.top + pad);
      const p = top ? photos.at(Number(top.dataset.index)) : photos.at(0);
      if (!p) return null;
      const d = new Date(p.takenAt * 1000);
      return { year: d.getFullYear(), month: d.getMonth() + 1 };
    }
    const label = here ?? months[0]?.month;
    const g = months.find((m) => m.month === label && m.items.length);
    if (!g || !photos) return null;
    const d = new Date(photos.keys[g.items[0].index] * 1000);
    return { year: d.getFullYear(), month: d.getMonth() + 1 };
  };
  const goLevel = (next: Level, to?: Place | null) => {
    if (next === level) return;
    const from = level === "all" ? gridPlace() : periodAt.current;
    const place = to === undefined ? from : to;
    if (level === "all") {
      leftAll.current = { top: scroller.current?.scrollTop ?? 0, place: from };
      // Nothing stays chosen out of sight, for a key to act on.
      setChosen(new Set());
      anchor.current = null;
    }
    setJumping(null);
    if (next === "all") toAll.current = place;
    else setLandAt(place ? { year: place.year, month: next === "months" ? place.month : undefined } : null);
    setLevel(next);
    try {
      window.localStorage.setItem(LEVEL_KEY, next);
    } catch {
      // For this run only.
    }
  };
  // All Photos shown again: at the month chosen, or exactly where it was left when that is the
  // same month, or a year it was in (Escape and back again), before its first frame is painted.
  useLayoutEffect(() => {
    const to = toAll.current;
    toAll.current = undefined;
    const el = scroller.current;
    if (level !== "all" || to === undefined || !el) return;
    const left = leftAll.current;
    if (left && (!to || (left.place?.year === to.year && (to.month === undefined || left.place.month === to.month)))) {
      el.scrollTop = left.top;
      return;
    }
    if (!to) return;
    // Sorted by Date Added, a month taken is spread through the months photos came in: All
    // Photos opens at the photo that stands for it in Months.
    if (sort?.by === "added") {
      const of = (periodMonths ?? []).filter((m) => m.year === to.year && (to.month === undefined || m.month === to.month));
      const key = (sort.oldest ? of[of.length - 1] : of[0])?.key;
      const index = key && photos ? photos.indexOf(key) : -1;
      if (index >= 0) goToPhoto(index);
      return;
    }
    // A year alone: its first month as the grid shows them.
    const label = to.month
      ? monthName(to.year, to.month)
      : months.find((m) => m.month && photos && new Date(photos.keys[m.items[0].index] * 1000).getFullYear() === to.year)?.month;
    if (label) goToMonth(label, true);
  }, [level]); // eslint-disable-line react-hooks/exhaustive-deps
  // Info closes with the photo: the next photo opens plain, as he expects it to.
  useEffect(() => {
    if (open === null) setInfo(false);
  }, [open === null]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    setChosen(new Set());
    anchor.current = null;
  }, [view, album]);

  // Zoom from the keys as in Photos: Ctrl and + or −.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!live.current) return;
      if (!(e.ctrlKey || e.metaKey) || e.altKey || viewer.current || awayRef.current) return;
      if (e.key === "=" || e.key === "+") acts.current.zoomBy(1);
      else if (e.key === "-") acts.current.zoomBy(-1);
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // A view chosen whose photos have not come yet. Nearly always they come within a frame or
  // two and nothing in between is shown; when the app is busy and they take longer, the new
  // view's title is up at once with its tiles as plain boxes — never the last view lingering
  // under the new choice, which read as the app not having heard the click.
  const switching = drawn.view !== view || (drawn.album?.id ?? null) !== (album?.id ?? null) || drawn.query !== searching;
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    setSlow(false);
    if (!switching) return undefined;
    const t = window.setTimeout(() => setSlow(true), 120);
    return () => window.clearTimeout(t);
  }, [switching, view, album?.id, searching]);
  /**
   * An album opened from the Albums or Shared Albums page. The view the grid last drew was out
   * of sight behind that page, so keeping it up while the album comes (as a switch between
   * views does, for its first moment) showed something not looked at — Recently Deleted's
   * title and photos for a frame, in a recording of a shared album opening. The album's own
   * title and plain tiles are up at once instead.
   */
  const [fromPage, setFromPage] = useState(false);
  useEffect(() => {
    if (!switching) setFromPage(false);
  }, [switching]);
  const waiting = switching && (slow || fromPage);
  const shownAs = waiting ? { view, album, query: searching } : drawn;
  const title = shownAs.query
    ? `“${shownAs.query}”`
    : shownAs.album
    ? (album?.id === shownAs.album.id ? album.name : shownAs.album.name)
    : [...VIEWS, ...MEDIA, ...UTILITIES].find((v) => v.key === shownAs.view)?.label ?? "Photos";
  const waitingFor = waiting ? Math.min(84, album ? (albums.find(([a]) => a.id === album.id)?.[1] ?? 21) : (counts[view] ?? 21)) : 0;
  // A view arrives at its top, not wherever the last one was scrolled to — and settles in with a
  // short fade (drawn frame by frame by the page, never the compositor's, which glitched on his
  // screen), from part-shown so there is never an empty moment between the two.
  const firstDraw = useRef(true);
  useLayoutEffect(() => {
    if (scroller.current) scroller.current.scrollTop = 0;
    if (firstDraw.current) {
      firstDraw.current = false;
      return;
    }
    fadeIn(scroller.current, { from: 0.35, duration: 160 });
  }, [drawn]);
  const across = ACROSS[zoom];
  /*
   * A zoom keeps its place, as Photos': the selected photo — or, with none in view, the photo at
   * the top left — stays where it was on the screen while the grid around it changes size.
   * Before, every step of the zoom left the view at a scroll offset that now meant a different
   * month, and the photo being looked at was gone.
   */
  const keepAt = useRef<{ index: number; top: number } | null>(null);
  function zoomBy(by: number) {
    const next = Math.max(0, Math.min(ACROSS.length - 1, zoom + by));
    if (next === zoom) return;
    const el = scroller.current;
    if (el && el.clientHeight) {
      const box = el.getBoundingClientRect();
      const pad = parseFloat(getComputedStyle(el).paddingTop) || 0;
      const seen = (t: Element | null) => {
        const r = t?.getBoundingClientRect();
        return r && r.bottom > box.top + pad - 8 && r.top < box.bottom ? r : null;
      };
      const mine = anchor.current !== null ? el.querySelector(`.px-tile[data-index="${anchor.current}"]`) : null;
      const tile = (seen(mine) ? mine : document.elementFromPoint(box.left + 8, box.top + pad + 8)?.closest(".px-tile")) as HTMLElement | null;
      if (tile && el.scrollTop > 0) keepAt.current = { index: Number(tile.dataset.index), top: tile.getBoundingClientRect().top - box.top };
    }
    setZoom(next);
  }
  useLayoutEffect(() => {
    const k = keepAt.current;
    keepAt.current = null;
    const el = scroller.current;
    if (!k || !el) return;
    const below = tileBelow(k.index);
    if (below !== null) el.scrollTop = Math.max(0, below + el.scrollTop - k.top);
  }, [across]); // eslint-disable-line react-hooks/exhaustive-deps
  /*
   * Photos landing above the view leave it on the photos it showed, as a chat keeps its place
   * when older messages load: the tile at its top is put back where it was, before the frame is
   * painted. The engine's own anchoring is off here (photos.css says why), and WebKit has none.
   * At the very top nothing is kept: the new photos are what is there to be seen.
   */
  useLayoutEffect(() => {
    const k = anchorAt.current;
    const el = scroller.current;
    if (!k || k.list !== photos || !el) return;
    anchorAt.current = null;
    const esc = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(k.id) : k.id;
    const tile = el.querySelector<HTMLElement>(`.px-tile[data-id="${esc}"]`);
    if (!tile) return;
    const by = tile.getBoundingClientRect().top - k.top;
    if (Math.abs(by) >= 0.5) el.scrollTop += by;
  }, [photos]);
  // A cell's width in device pixels, for the size each tile's picture is made at; in steps of
  // 32, so a window being resized does not redraw every tile on every frame.
  const [gridWidth, setGridWidth] = useState(0);
  // The header's height, for the room the grid leaves under it: one row in a wide window, two
  // when the controls go under the title.
  const bar = useRef<HTMLElement | null>(null);
  useLayoutEffect(() => {
    const el = bar.current;
    const main = el?.parentElement;
    if (!el || !main) return undefined;
    // Written only when it changes, and a frame later: the room it makes changes the grid's own
    // box, which its own watcher sees — in the same frame, a loop the engine reports.
    let frame = 0;
    const measure = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (!el.clientWidth) return;
        const h = `${Math.ceil(el.getBoundingClientRect().height)}px`;
        if (main.style.getPropertyValue("--bar") !== h) main.style.setProperty("--bar", h);
      });
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => {
      cancelAnimationFrame(frame);
      watch.disconnect();
    };
  }, []);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return undefined;
    // Never measured while out of sight: with no width, the grid laid itself out empty and lost
    // its place.
    const measure = () => el.clientWidth && setGridWidth(Math.ceil(el.clientWidth / 32) * 32);
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, []);
  // The screen's pixel density, followed as it changes (the window moved to another screen, the
  // page zoomed): no size watcher sees that, and tiles stayed at the old density's pictures.
  const [dpr, setDpr] = useState(() => window.devicePixelRatio || 1);
  useEffect(() => {
    const at = window.matchMedia?.(`(resolution: ${dpr}dppx)`);
    if (!at) return undefined;
    const changed = () => setDpr(window.devicePixelRatio || 1);
    at.addEventListener("change", changed);
    return () => at.removeEventListener("change", changed);
  }, [dpr]);
  const cell = Math.round(((gridWidth || 1200) / across) * dpr);
  // The app makes, ahead, every picture the grid and the viewer are about to show at these
  // sizes (photos_prepare): the first look at a photo is as sharp as the hundredth. Told again
  // when they change, once the window has settled.
  useEffect(() => {
    const t = window.setTimeout(() => {
      const box = stageBox.w ? stageBox : { w: Math.max(320, window.innerWidth - 400), h: Math.max(240, window.innerHeight - 140) };
      // The view on screen has its pictures made first (Hidden is not in the library's).
      // Hidden, locked, has none made: the library's are.
      api.photosPrepare({ cell, fit, stageW: box.w, stageH: box.h, dpr, view: hiddenLocked ? "library" : view }).catch(() => {});
    }, 400);
    return () => window.clearTimeout(t);
  }, [cell, fit, open !== null, view, dpr, hiddenLocked]); // eslint-disable-line react-hooks/exhaustive-deps

  // A new album or folder, at the top or in a folder: named in place in the sidebar.
  const startNew = (parent: string | null, folder: boolean) => {
    if (!branches.albums) branch("albums");
    if (parent) setOpenFolders((open) => new Set(open).add(parent));
    setNaming({ id: null, then: [], draft: "", folder, parent });
  };
  const newIn = (parent: string | null, at: { x: number; y: number; alignRight: boolean }) =>
    setMenu({
      at,
      entries: [
        { label: "New Album", run: () => startNew(parent, false) },
        { label: "New Folder", run: () => startNew(parent, true) },
      ],
    });
  const openFolder = (id: string | null) => {
    setAlbumsPage(true);
    setFolderPage(id);
    endSearch();
    setOpen(null);
  };

  /** The albums and folders in one folder (or at the top), folders open to theirs, nested. */
  const albumRows = (parent: string | null, depth: number): React.ReactNode[] => {
    const rows: React.ReactNode[] = [];
    const indent = { "--depth": depth } as React.CSSProperties;
    if (naming?.id === null && (naming.parent ?? null) === parent) {
      rows.push(<span key="new" className="px-depth" style={indent}>{nameField}</span>);
    }
    for (const [a, n, cover] of albums) {
      if ((a.parent ?? null) !== parent) continue;
      if (naming?.id === a.id) {
        rows.push(<span key={a.id} className="px-depth" style={indent}>{nameField}</span>);
        continue;
      }
      if (a.folder) {
        const isOpen = openFolders.has(a.id);
        rows.push(
          <div key={a.id} className="px-parent-row px-depth" style={indent}>
            <button
              className="px-twisty"
              aria-label={isOpen ? `Hide ${a.name}` : `Show ${a.name}`}
              aria-expanded={isOpen}
              onClick={() =>
                setOpenFolders((open) => {
                  const next = new Set(open);
                  if (next.has(a.id)) next.delete(a.id);
                  else next.add(a.id);
                  return next;
                })
              }
            >
              <ChevronRight className="px-chevron" size={12} strokeWidth={2.4} aria-hidden="true" />
            </button>
            <button
              className="source-row px-source px-parent"
              aria-current={albumsPage && folderPage === a.id}
              onClick={() => openFolder(a.id)}
              onDoubleClick={() => setNaming({ id: a.id, then: [], draft: a.name })}
              onContextMenu={(e) => {
                e.preventDefault();
                if (a.id.startsWith("new:")) return;
                setMenu({
                  at: { x: e.clientX, y: e.clientY, alignRight: false },
                  entries: [
                    { label: "New Album", run: () => startNew(a.id, false) },
                    { label: "New Folder", run: () => startNew(a.id, true) },
                    null,
                    { label: "Rename Folder", run: () => setNaming({ id: a.id, then: [], draft: a.name }) },
                    null,
                    { label: "Delete Folder", danger: true, run: () => void deleteAlbum(a) },
                  ],
                });
              }}
            >
              <FolderIcon size={16} strokeWidth={1.8} className="px-glyph" aria-hidden="true" />
              <span className="source-name px-label">{a.name}</span>
            </button>
          </div>,
        );
        if (isOpen) rows.push(...albumRows(a.id, depth + 1));
        continue;
      }
      rows.push(
        <button
          key={a.id}
          className="source-row px-source px-depth"
          style={indent}
          aria-current={!albumsPage && album?.id === a.id}
          onClick={() => {
            setAlbum(a);
            setAlbumsPage(false);
            setSharedPage(false);
            endSearch();
            setOpen(null);
          }}
          onDoubleClick={() => setNaming({ id: a.id, then: [], draft: a.name })}
          {...dropTarget(a)}
          data-drop={dropOn === a.id || undefined}
          onContextMenu={(e) => {
            e.preventDefault();
            if (a.id.startsWith("new:")) return;
            setMenu({
              at: { x: e.clientX, y: e.clientY, alignRight: false },
              entries: [
                { label: "Rename Album", run: () => setNaming({ id: a.id, then: [], draft: a.name }) },
                null,
                { label: "Delete Album", danger: true, run: () => void deleteAlbum(a) },
              ],
            });
          }}
        >
          {cover ? (
            <img className="px-cover" src={pictureSrc(cover, "thumb")} alt="" loading="lazy" decoding="async" draggable={false} />
          ) : (
            <AlbumGlyph />
          )}
          <span className="source-name px-label">{a.name}</span>
          {n ? <span className="count px-count">{n}</span> : null}
        </button>,
      );
    }
    return rows;
  };

  const source = (v: (typeof VIEWS)[number]) => (
    <button
      key={v.key}
      className="source-row px-source"
      aria-current={!album && !albumsPage && !sharedPage && view === v.key}
      onClick={() => {
        setView(v.key);
        setAlbum(null);
        setAlbumsPage(false);
        setSharedPage(false);
        endSearch();
        setOpen(null);
      }}
    >
      <v.Icon className="px-glyph" size={16} strokeWidth={1.8} aria-hidden="true" />
      <span className="source-name px-label">{v.label}</span>
      {counts[v.key] && !(v.key === "hidden" && !hiddenOpen) ? <span className="count px-count">{counts[v.key]}</span> : null}
    </button>
  );

  // In a narrow window the sidebar folds into one pop-up above the library, as Notes' folders
  // do: every place the sidebar goes, in its order. Beside 480px of window the sidebar took
  // half of it and left the photos a strip.
  const goTo = (to: string) => {
    const [kind, id] = [to.slice(0, to.indexOf(":")), to.slice(to.indexOf(":") + 1)];
    endSearch();
    setOpen(null);
    if (kind === "view") {
      setView(id as PhotoView);
      setAlbum(null);
      setAlbumsPage(false);
      setSharedPage(false);
    } else if (kind === "album") {
      const a = albums.find(([x]) => x.id === id)?.[0];
      if (!a) return;
      setAlbum(a);
      setAlbumsPage(false);
      setSharedPage(false);
    } else if (kind === "page" && id === "albums") {
      setAlbumsPage(true);
      setSharedPage(false);
      setFolderPage(null);
    } else if (kind === "page" && id === "shared") {
      setSharedPage(true);
      setAlbumsPage(false);
    }
  };
  const placeNow = sharedPage ? "page:shared" : albumsPage ? "page:albums" : album ? `album:${album.id}` : `view:${view}`;
  const places = [
    ...VIEWS.map((v) => ({ value: `view:${v.key}`, label: v.label })),
    { value: "page:albums", label: "Albums" },
    ...albums.filter(([a]) => !a.folder && !a.id.startsWith("new:")).map(([a]) => ({ value: `album:${a.id}`, label: `\u2003${a.name}` })),
    ...MEDIA.filter((m) => (counts[m.key] ?? 0) > 0 || (!album && !albumsPage && view === m.key)).map((v) => ({ value: `view:${v.key}`, label: v.label })),
    ...UTILITIES.map((v) => ({ value: `view:${v.key}`, label: v.label })),
    { value: "page:shared", label: "Shared Albums" },
  ];

  return (
    <div className={`px${dropping ? " is-dropping" : ""}`} data-drop-label={album ? `Add to ${album.name}` : "Add to Library"}>
      <div className="px-compact">
        <Picker label="Show" value={placeNow} options={places} onChange={goTo} />
      </div>
      <aside className="px-side" aria-label="Photos">
        <p className="side-heading px-side-heading">Photos</p>
        {/* Photos' library switcher, only for an account with a Shared Library: the views,
            their counts and the months then show the library chosen. */}
        {library.hasShared && (
          <Picker
            className="px-library"
            label="Library"
            value={library.showing}
            options={LIBRARIES}
            onChange={(v) => {
              const which = v as PhotoLibrary;
              api.photosSetLibrary(which).then(
                () => setLibrary((was) => ({ ...was, showing: which })),
                (e) => setError(String(e)),
              );
            }}
          />
        )}
        {VIEWS.map(source)}
        {/* Collections, as icloud.com lists them: Albums and Media Types open to what they
            hold, then Rated, Hidden and Recently Deleted. */}
        <p className="side-heading px-side-heading px-side-gap">Collections</p>
        <div className="px-parent-row">
          {/* The chevron opens the list here; the row itself opens the Albums page. */}
          <button className="px-twisty" aria-label={branches.albums ? "Hide Albums" : "Show Albums"} aria-expanded={branches.albums} onClick={() => branch("albums")}>
            <ChevronRight className="px-chevron" size={12} strokeWidth={2.4} aria-hidden="true" />
          </button>
          <button
            className="source-row px-source px-parent"
            aria-current={albumsPage && folderPage === null}
            onClick={() => {
              setAlbumsPage(true);
              setSharedPage(false);
              setFolderPage(null);
              endSearch();
              setOpen(null);
            }}
          >
            <AlbumGlyph />
            <span className="source-name px-label">Albums</span>
          </button>
          <button className="icon-button small px-side-add" aria-label="New Album" title="New Album or Folder" aria-haspopup="menu" onClick={(e) => {
            const b = e.currentTarget.getBoundingClientRect();
            newIn(null, { x: b.left, y: b.bottom + 4, alignRight: false });
          }}>
            <Plus size={14} strokeWidth={2} aria-hidden="true" />
          </button>
        </div>
        {branches.albums && (
          <div className="px-albums px-children" role="group" aria-label="Albums">
            {albumRows(null, 0)}
          </div>
        )}
        <button className="source-row px-source px-parent" aria-expanded={branches.media} onClick={() => branch("media")}>
          <ChevronRight className="px-chevron" size={12} strokeWidth={2.4} aria-hidden="true" />
          <Layers size={16} strokeWidth={1.8} className="px-glyph" aria-hidden="true" />
          <span className="source-name px-label">Media Types</span>
        </button>
        {branches.media && (
          <div className="px-children" role="group" aria-label="Media Types">
            {/* As icloud.com lists them: only the kinds the library has. */}
            {MEDIA.filter((m) => (counts[m.key] ?? 0) > 0 || (!album && !albumsPage && view === m.key)).map(source)}
          </div>
        )}
        {UTILITIES.map(source)}
        {/* Sharing, as icloud.com lists it: the shared albums this account has made or joined. */}
        <p className="side-heading px-side-heading px-side-gap">Sharing</p>
        <button
          className="source-row px-source"
          aria-current={sharedPage}
          onClick={() => {
            setSharedPage(true);
            setAlbumsPage(false);
            endSearch();
            setOpen(null);
          }}
        >
          <Users className="px-glyph" size={16} strokeWidth={1.8} aria-hidden="true" />
          <span className="source-name px-label">Shared Albums</span>
        </button>
        <AppFoot />
      </aside>

      <section className="px-main" aria-label={sharedPage ? "Shared Albums" : albumsPage ? "Albums" : title} data-page={albumsPage || sharedPage ? "albums" : undefined}>
        {sharedPage && (
          <SharedAlbums
            onError={(m) => setError(m)}
            onOpen={(a) => {
              // The grid underneath still holds the last view's photos (Recently Deleted, say):
              // let go of first, so they never show for a frame before the album's own.
              setPhotos(null);
              setFromPage(true);
              setAlbum({ id: `shared:${a.zone}`, name: a.title ?? "Shared Album", position: 0, changeTag: null, shared: a.zone });
              setAlbumsPage(false);
              setSharedPage(false);
              endSearch();
              setOpen(null);
            }}
          />
        )}
        {albumsPage && !sharedPage && (
          <div className="px-albums-page">
            {(() => {
              // This page's level: the top, or a folder's own albums and folders.
              const here = albums.filter(([a]) => (a.parent ?? null) === folderPage);
              const folder = folderPage ? albums.find(([a]) => a.id === folderPage)?.[0] : undefined;
              const held = (id: string) => albums.filter(([a]) => a.parent === id).length;
              return (
                <>
                  <header className="px-albums-head">
                    {folder && (
                      <div className="toolbar-group">
                        <button className="tool px-cap" aria-label="Back" title="Back" onClick={() => openFolder(folder.parent ?? null)}>
                          <ChevronLeft size={17} strokeWidth={2} aria-hidden="true" />
                        </button>
                      </div>
                    )}
                    <PaneTitle className="px-heading" countClassName="px-span" count={here.length === 1 ? "1 Item" : `${here.length} Items`}>
                      {folder ? folder.name : "Albums"}
                    </PaneTitle>
                    <div className="toolbar-group">
                      <button
                        className="tool px-cap"
                        aria-label="New Album or Folder"
                        title="New Album or Folder"
                        aria-haspopup="menu"
                        onClick={(e) => {
                          const b = e.currentTarget.getBoundingClientRect();
                          newIn(folderPage, { x: b.right, y: b.bottom + 6, alignRight: true });
                        }}
                      >
                        <Plus size={16} strokeWidth={2} aria-hidden="true" />
                      </button>
                    </div>
                  </header>
                  {here.length === 0 ? (
                    <EmptyState className="px-empty" icon={folder ? <FolderIcon size={28} strokeWidth={1.6} /> : <AlbumGlyph />}>
                      {folder ? "Make an album in this folder with +." : "Make an album with +, then drag photos onto it."}
                    </EmptyState>
                  ) : (
                    <div className="px-cards" role="list">
                      {here.map(([a, n, cover]) => (
                        <button
                          key={a.id}
                          role="listitem"
                          className={`px-card${a.folder ? " is-folder" : ""}`}
                          onClick={() => {
                            if (a.folder) return openFolder(a.id);
                            // As a shared album opens: the last view's photos never show first.
                            if (album?.id !== a.id) {
                              setPhotos(null);
                              setFromPage(true);
                            }
                            setAlbum(a);
                            setAlbumsPage(false);
                            setSharedPage(false);
                          }}
                          {...dropTarget(a)}
                          data-drop={dropOn === a.id || undefined}
                          onContextMenu={(e) => {
                            e.preventDefault();
                            if (a.id.startsWith("new:")) return;
                            const kind = a.folder ? "Folder" : "Album";
                            setMenu({
                              at: { x: e.clientX, y: e.clientY, alignRight: false },
                              entries: [
                                { label: `Rename ${kind}`, run: () => {
                                  if (!branches.albums) branch("albums");
                                  if (a.parent) setOpenFolders((open) => new Set(open).add(a.parent!));
                                  setNaming({ id: a.id, then: [], draft: a.name });
                                } },
                                null,
                                { label: `Delete ${kind}`, danger: true, run: () => void deleteAlbum(a) },
                              ],
                            });
                          }}
                        >
                          <span className="px-card-pic">
                            {a.folder ? (
                              <FolderIcon className="px-glyph" size={44} strokeWidth={1.4} aria-hidden="true" />
                            ) : cover ? (
                              <img src={pictureSrc(cover, "thumb")} alt="" loading="lazy" decoding="async" draggable={false} />
                            ) : (
                              <AlbumGlyph />
                            )}
                          </span>
                          <span className="px-card-name">{a.name}</span>
                          <span className="px-card-count">
                            {a.folder ? (held(a.id) === 1 ? "1 album" : `${held(a.id)} albums`) : n === 1 ? "1 item" : `${n} items`}
                          </span>
                        </button>
                      ))}
                    </div>
                  )}
                </>
              );
            })()}
          </div>
        )}
        {/* As in Photos on macOS 26: the title and the controls float over the library, which
            scrolls beneath them and fades out at the top edge. */}
        <header className="px-bar" ref={bar}>
          <PaneTitle
            className="px-heading"
            countClassName="px-span"
            count={
              selected.length > 0
                ? pickedWords(kindsOf(selected))
                : drawn.query
                  ? photos?.length === 1 ? "1 Result" : `${(photos?.length ?? 0).toLocaleString()} Results`
                  : receiving
                    ? (
                        <>
                          {(here ?? span) && <>{here ?? span} · </>}
                          <span className="px-receiving" role="status">
                            Receiving {receivingWords(receiving.ids.size, receiving.videos)}
                          </span>
                        </>
                      )
                    : (away ? span : (here ?? span)) || undefined
            }
          >
            {title}
          </PaneTitle>
          <div className="px-tools">
          {periodsHere && (
            <div className="segmented toolbar-group px-levels" role="group" aria-label="Library">
              {LEVELS.map(([key, label]) => (
                <button key={key} className="segment" aria-pressed={level === key} onClick={() => goLevel(key)}>
                  {label}
                </button>
              ))}
            </div>
          )}
          {place === "recentlyDeleted" && !inAlbum && photos && photos.length > 0 && (
            // Recently Deleted's own toolbar, in Photos' words: Recover and Delete for what is
            // selected, Delete All when nothing is.
            <div className="toolbar-group px-words">
              {selected.length > 0 ? (
                <>
                  <button className="tool has-label is-accent px-word" onClick={() => void changeAll(selected, "deleted", false)}>
                    Recover
                  </button>
                  <button className="tool has-label danger px-word" onClick={() => void changeAll(selected, "expunged", true)}>
                    Delete
                  </button>
                </>
              ) : (
                <button className="tool has-label danger px-word" onClick={() => void changeAll(photos.ids, "expunged", true)}>
                  Delete All
                </button>
              )}
            </div>
          )}
          {!away && <div className="toolbar-group">
            <button
              className="tool px-cap is-glyph-state"
              aria-label={fit ? "Show Square Photos" : "Show Whole Photos"}
              aria-pressed={fit}
              title={fit ? "Square photos" : "Whole photos"}
              onClick={toggleFit}
            >
              {fit ? <Square size={15} strokeWidth={1.9} aria-hidden="true" /> : <RectangleHorizontal size={16} strokeWidth={1.9} aria-hidden="true" />}
            </button>
          </div>}
          {sort && (
            <div className="toolbar-group">
              <button
                className="tool px-cap"
                aria-label="Sort"
                aria-haspopup="menu"
                title="Sort"
                onClick={(e) => {
                  const box = e.currentTarget.getBoundingClientRect();
                  setMenu({
                    at: { x: box.right, y: box.bottom + 6, alignRight: true },
                    entries: [
                      { label: "Date Captured", checked: sort.by === "taken", run: () => chooseSort({ ...sort, by: "taken" }) },
                      { label: "Date Added", checked: sort.by === "added", run: () => chooseSort({ ...sort, by: "added" }) },
                      null,
                      { label: "Newest First", checked: !sort.oldest, run: () => chooseSort({ ...sort, oldest: false }) },
                      { label: "Oldest First", checked: sort.oldest, run: () => chooseSort({ ...sort, oldest: true }) },
                    ],
                  });
                }}
              >
                <ArrowDownUp size={16} strokeWidth={1.9} aria-hidden="true" />
              </button>
            </div>
          )}
          {monthCovers.length > 1 && !away && (
            <div className="toolbar-group">
              <button
                className="tool px-cap"
                aria-label="Go to a Month"
                aria-haspopup="dialog"
                aria-expanded={jumping !== null}
                title="Go to a month"
                onClick={(e) => setJumping(jumping ? null : e.currentTarget.getBoundingClientRect())}
              >
                <CalendarDays size={16} strokeWidth={1.9} aria-hidden="true" />
              </button>
            </div>
          )}
          {jumping && <JumpTo months={monthCovers} at={jumping} current={here ?? monthCovers[0]?.label ?? null} onPick={goToMonth} onClose={() => setJumping(null)} />}
          {!away && <div className="toolbar-group" role="group" aria-label="Zoom">
            <button className="tool px-cap" aria-label="Zoom Out" title="Zoom Out (Ctrl −)" disabled={zoom === 0} onClick={() => zoomBy(-1)}>
              <Minus size={15} strokeWidth={2.2} aria-hidden="true" />
            </button>
            <button className="tool px-cap" aria-label="Zoom In" title="Zoom In (Ctrl +)" disabled={zoom === ACROSS.length - 1} onClick={() => zoomBy(1)}>
              <Plus size={15} strokeWidth={2.2} aria-hidden="true" />
            </button>
          </div>}
          {/* Adds to the library — so not offered in a shared album, where it would read as
              adding to the album. */}
          {!viewOnly && (
            <div className="toolbar-group">
              <button className="tool px-cap px-add" onClick={() => void choose()} title="Add photos and videos" aria-label="Add Photos">
                <ImagePlus size={16} strokeWidth={1.9} aria-hidden="true" />
              </button>
            </div>
          )}
          {/* Search, as icloud.com's toolbar has it: names, titles, captions, keywords, years and
              months, from the library on this machine — nothing typed leaves it. */}
          <SearchField
            inToolbar
            className="px-search"
            aria-label="Search Photos"
            placeholder="Search Photos"
            value={query}
            onChange={setQuery}
            onClear={endSearch}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                endSearch();
                e.currentTarget.blur();
              }
            }}
          />
          </div>
        </header>


        {hiddenLocked && (
          <EmptyState
            className="px-empty px-locked"
            icon={<Lock size={28} strokeWidth={1.6} />}
            action={{ label: "View Hidden Photos", run: () => setUnlocking(true) }}
          >
            Enter your Apple Account password to view this album.
          </EmptyState>
        )}
        {unlocking && hiddenLocked && (
          <UnlockSheet
            onCancel={() => setUnlocking(false)}
            onUnlocked={() => {
              setUnlocking(false);
              setHiddenOpen(true);
            }}
          />
        )}
        {!hiddenLocked && photos && photos.length === 0 && !drawn.query && (
          <Arriving service="photos" what="photos" className="px-empty" />
        )}
        {!hiddenLocked && photos && photos.length === 0 && preparing && !drawn.query && (
          <EmptyState className="px-empty px-preparing" icon={<Images size={28} strokeWidth={1.6} />}>
            {preparing}
          </EmptyState>
        )}
        {!hiddenLocked && photos && photos.length === 0 && !(arriving && !drawn.query) && !(preparing && !drawn.query) && (
          // One line, saying what the place is for, with its own symbol from the sidebar.
          <EmptyState
            className="px-empty"
            icon={(() => {
              const Icon = drawn.query ? null : drawn.album ? null : [...VIEWS, ...MEDIA, ...UTILITIES].find((v) => v.key === drawn.view)?.Icon;
              return drawn.album ? <AlbumGlyph /> : Icon ? <Icon size={28} strokeWidth={1.6} /> : undefined;
            })()}
          >
            {drawn.query
              ? `No photos match “${drawn.query}”. Search finds names, titles, captions, keywords, years and months.`
              : drawn.album
              ? "Photos you add to this album show here."
              : ({
                  library: "Your photos and videos show here.",
                  recents: "Photos you add show here first.",
                  favorites: "Click the heart on a photo to keep it here.",
                  videos: "Your videos show here.",
                  portrait: "Your Portrait photos show here.",
                  panoramas: "Your panoramas show here.",
                  screenshots: "Your screenshots show here.",
                  rated: "Photos you give stars to show here.",
                  hidden: "Photos you hide wait here, out of your library.",
                  recentlyDeleted: "Photos you delete stay here for 30 days.",
                } as Partial<Record<PhotoView, string>>)[drawn.view] ?? "Your photos show here."}
          </EmptyState>
        )}

        <div
          className="px-scroll"
          ref={scroller}
          onScroll={onScroll}
          onPointerDown={sweepStart}
          onClick={(e) => {
            // A click on the space between photos lets the selection go, as in Photos — not the
            // click that ends a sweep.
            if (swept.current) {
              swept.current = false;
              return;
            }
            if (!(e.target as HTMLElement).closest(".px-tile")) setChosen(new Set());
          }} data-fit={fit || undefined} data-away={away || undefined} style={{ "--across": across } as React.CSSProperties}>
          {waiting && (
            <div className="px-month" aria-busy="true">
              <h2 className="px-month-title px-month-title-ph" aria-hidden="true">&nbsp;</h2>
              <div className="px-grid px-later" aria-hidden="true">
                {Array.from({ length: waitingFor }, (_, i) => (
                  <div key={i} className="px-ph" />
                ))}
              </div>
            </div>
          )}
          {!waiting && months.map((m, k) => (
            <Month
              // By its name, never its place: a new month at the top moved every month's place,
              // and each was made again from nothing — every tile and every picture.
              key={m.key}
              label={m.label}
              view={drawnKey}
              month={m.month}
              items={m.items}
              list={photos!}
              chosen={chosen}
              anchor={anchor.current}
              deleted={drawn.view === "recentlyDeleted" && !drawn.album}
              eager={k < 3}
              across={across}
              fit={fit}
              cell={cell}
              act={act}
            />
          ))}
          {tally && <p className="px-tally">{tally}</p>}
          <div className="px-sweep" ref={sweepBox} aria-hidden="true" hidden />
        </div>
        {away && periodMonths && (
          <Periods
            level={level as "years" | "months"}
            months={periodMonths}
            oldest={sort?.oldest ?? false}
            land={landAt}
            at={periodAt}
            onYear={(year) => goLevel("months", { year })}
            onMonth={(year, month) => goLevel("all", { year, month })}
          />
        )}

        {shown && (
          <div
            className={`px-viewer${fullscreen ? " is-full" : ""}${fullscreen && thumbs ? " has-thumbs" : ""}${info ? " has-info" : ""}`}
            ref={viewer}
            role="dialog"
            aria-label={shown.filename ?? "Photo"}
            onPointerMove={
              fullscreen
                ? (e) => {
                    // The bar comes up as the pointer reaches the top, as icloud.com's does.
                    if (e.clientY < 56) {
                      if (!barUp) setBarUp(true);
                      window.clearTimeout(barTimer.current);
                    } else if (barUp) barAway(700);
                  }
                : undefined
            }
          >
            <div className="px-backdrop" aria-hidden="true" />
            {fullscreen ? (
            <header
              key="full"
              className={`px-full-bar${barUp ? "" : " is-away"}`}
              ref={fullBar}
              // Kept up while a key's focus is in it; a click's focus does not hold it.
              onFocus={(e) => {
                if (!(e.target as HTMLElement).matches(":focus-visible")) return;
                setBarUp(true);
                window.clearTimeout(barTimer.current);
              }}
              onBlur={() => barAway()}
            >
              <div className="px-full-side">
                <button className="px-full-btn" aria-label="Exit Full Screen" title="Exit Full Screen (Esc)" onClick={() => void toggleFullscreen(false)}>
                  <Minimize2 size={16} />
                </button>
                {photos && photos.length > 1 && (
                  <button
                    className="px-full-btn"
                    aria-label={thumbs ? "Hide Thumbnails" : "Show Thumbnails"}
                    aria-pressed={thumbs}
                    title={thumbs ? "Hide Thumbnails" : "Show Thumbnails"}
                    onClick={toggleThumbs}
                  >
                    <GalleryThumbnails size={17} />
                  </button>
                )}
                {shown.kind !== "video" && (
                  <div className="px-zoom">
                    <button className="px-full-btn is-small" aria-label="Zoom Out" title="Zoom Out" onClick={() => zoomMiddle(lensRef.current.k / 1.5)}>
                      <Minus size={13} />
                    </button>
                    <input
                      ref={zoomSlider}
                      type="range"
                      min={1}
                      max={5}
                      step={0.01}
                      defaultValue={lensRef.current.k}
                      aria-label="Zoom"
                      onInput={(e) => zoomMiddle(Number(e.currentTarget.value), false)}
                    />
                    <button className="px-full-btn is-small" aria-label="Zoom In" title="Zoom In" onClick={() => zoomMiddle(lensRef.current.k * 1.5)}>
                      <Plus size={13} />
                    </button>
                  </div>
                )}
              </div>
              {when}
              <div className="px-full-side is-end">
                <button className="px-full-btn" aria-label="Info" aria-pressed={info} title="Info (I)" onClick={() => setInfo((v) => !v)}>
                  <Info size={17} />
                </button>
                {place === "recentlyDeleted" && !inAlbum ? (
                  <>
                    <button className="px-full-btn" aria-label="Recover" title="Recover" onClick={() => void change(shown, "deleted", false)}>
                      <RotateCcw size={17} />
                    </button>
                    <button className="px-full-btn" aria-label="Delete Permanently" title="Delete permanently (Delete)" onClick={() => void change(shown, "expunged", true)}>
                      <Trash2 size={17} />
                    </button>
                  </>
                ) : viewOnly ? (
                  <button className="px-full-btn" aria-label="Download" title="Download the original" onClick={() => void download(shown)}>
                    <Download size={17} />
                  </button>
                ) : (
                  <>
                    <button
                      className="px-full-btn"
                      aria-label={shown.favorite ? "Unfavorite" : "Favorite"}
                      aria-pressed={shown.favorite}
                      title={shown.favorite ? "Unfavorite (.)" : "Favorite (.)"}
                      onClick={() => void change(shown, "favorite", !shown.favorite)}
                    >
                      <Heart size={17} fill={shown.favorite ? "currentColor" : "none"} />
                    </button>
                    <button className="px-full-btn" aria-label="Download" title="Download the original" onClick={() => void download(shown)}>
                      <Download size={17} />
                    </button>
                    {inAlbum ? (
                      <button className="px-full-btn" aria-label="Remove from Album" title="Remove from Album (Delete)" onClick={() => void removeFrom([shown.id])}>
                        <CopyMinus size={17} />
                      </button>
                    ) : (
                      <button className="px-full-btn" aria-label="Delete" title="Delete (Delete)" onClick={() => void change(shown, "deleted", true)}>
                        <Trash2 size={17} />
                      </button>
                    )}
                  </>
                )}
                <button
                  className="px-full-btn"
                  aria-label="More"
                  aria-haspopup="menu"
                  title="More"
                  onClick={(e) => {
                    const r = e.currentTarget.getBoundingClientRect();
                    photoMenu(shown, { x: r.right, y: r.bottom + 4, alignRight: true });
                  }}
                >
                  <Ellipsis size={17} />
                </button>
              </div>
            </header>
            ) : (
            <header key="bar" className="px-viewer-bar">
              <div className="toolbar-group">
                <button className="tool px-cap" aria-label="Close" title="Close (Esc)" onClick={close}>
                  <X size={17} strokeWidth={2} />
                </button>
                <button
                  className="tool px-cap"
                  aria-label={fullscreen ? "Exit Full Screen" : "Enter Full Screen"}
                  aria-pressed={fullscreen}
                  title={fullscreen ? "Exit Full Screen" : "Enter Full Screen"}
                  onClick={() => void toggleFullscreen()}
                >
                  {fullscreen ? <Minimize2 size={16} /> : <Maximize2 size={16} />}
                </button>
              </div>
              {when}
              <div className="toolbar-group">
              {viewOnly ? null : place === "recentlyDeleted" && !inAlbum ? (
                <>
                  <button className="tool px-cap" aria-label="Recover" title="Recover" onClick={() => void change(shown, "deleted", false)}>
                    <RotateCcw size={17} />
                  </button>
                  <button className="tool px-cap" aria-label="Delete Permanently" title="Delete permanently (Delete)" onClick={() => void change(shown, "expunged", true)}>
                    <Trash2 size={17} />
                  </button>
                </>
              ) : (
                <>
                  <button
                    className="tool px-cap"
                    aria-label={shown.favorite ? "Unfavorite" : "Favorite"}
                    aria-pressed={shown.favorite}
                    title={shown.favorite ? "Unfavorite (.)" : "Favorite (.)"}
                    onClick={() => void change(shown, "favorite", !shown.favorite)}
                  >
                    <Heart size={17} fill={shown.favorite ? "currentColor" : "none"} />
                  </button>
                  <button
                    className="tool px-cap"
                    aria-label="Add to Album"
                    aria-haspopup="menu"
                    title="Add to Album"
                    onClick={(e) => {
                      const r = e.currentTarget.getBoundingClientRect();
                      setMenu({ at: { x: r.right, y: r.bottom + 4, alignRight: true }, entries: albumEntries([shown.id]) });
                    }}
                  >
                    <CopyPlus size={17} />
                  </button>
                  {inAlbum ? (
                    <button className="tool px-cap" aria-label="Remove from Album" title="Remove from Album (Delete)" onClick={() => void removeFrom([shown.id])}>
                      <CopyMinus size={17} />
                    </button>
                  ) : (
                    <button className="tool px-cap" aria-label="Delete" title="Delete (Delete)" onClick={() => void change(shown, "deleted", true)}>
                      <Trash2 size={17} />
                    </button>
                  )}
                </>
              )}
              </div>
              <div className="toolbar-group">
                <button className="tool px-cap" aria-label="Start Slideshow" title="Start Slideshow" onClick={() => startShow(shown)}>
                  <MonitorPlay size={17} />
                </button>
                <button className="tool px-cap" aria-label="Download" title="Download the original" onClick={() => void download(shown)}>
                  <Download size={17} />
                </button>
                <button className="tool px-cap" aria-label="Info" aria-pressed={info} title="Info (I)" onClick={() => setInfo((v) => !v)}>
                  <Info size={17} />
                </button>
              </div>
            </header>
            )}
            <div
              className="px-stage"
              ref={stage}
              onContextMenu={(e) => {
                // The photo's own menu on the photo itself, as on a Mac.
                e.preventDefault();
                photoMenu(shown, { x: e.clientX, y: e.clientY, alignRight: false });
              }}
            >
              {shown.kind === "video" && videoBase && !unplayable.has(shown.id) ? (
                // Played here, as Photos plays one: in the pieces the player asks for, served
                // as they arrive from Apple, so it starts at once and seeks anywhere.
                <div className="px-video">
                  <VideoPlayer
                    key={film}
                    turn={turn?.key === film ? turn.value : undefined}
                    zoom={turn?.key === film ? turn.zoom : 1}
                    shape={shapeOf(shown)}
                    src={`${videoBase}/${film}`}
                    poster={viewSrc(shown)}
                    label={shown.filename ?? "Video"}
                    style={fitted(shown)}
                    onError={() =>
                      noCopy.has(shown.id)
                        ? setUnplayable((was) => new Set(was).add(shown.id))
                        : setNoCopy((was) => new Set(was).add(shown.id))
                    }
                  />
                </div>
              ) : shown.kind === "video" ? (
                // This computer's engine could not play it (no codec for it): a still, and Play
                // opens it in the system's own player.
                <div className="px-video-hold">
                  <button className="px-video" aria-label="Play in your video player" onClick={() => void play(shown)}>
                    <img key={shown.id} className="px-shown" src={viewSrc(shown)} alt={shown.filename ?? ""} decoding="async" style={fitted(shown)} />
                    <span className="px-play" aria-hidden="true">
                      <Play size={28} strokeWidth={0} fill="currentColor" />
                    </span>
                  </button>
                  {/* Why it is a still: said plainly, with what would fix it. */}
                  {videoBase && (
                    <p className="px-video-note" role="status">
                      {videoSupport && videoSupport.missing.length > 0
                        ? `This video can't play here: this computer is missing ${videoSupport.missing.join(" and ")}. Install it, then reopen Skykeep. Until then, Play opens it in your video player.`
                        : "This video can't play here. Play opens it in your video player."}
                    </p>
                  )}
                </div>
              ) : (
                <>
                {under && (
                  // The last picture painted, held until the next one over it is decoded.
                  <img
                    key={`${under.id}-under`}
                    ref={underEl}
                    className="px-under"
                    src={viewSrc(under)}
                    alt=""
                    aria-hidden="true"
                    decoding="async"
                    draggable={false}
                    style={fitted(under)}
                  />
                )}
                <div
                  ref={pan}
                  className={`px-pan${zoomed ? " is-zoomed" : ""}`}
                  onDoubleClick={(e) => (lensRef.current.k > 1 ? unzoom() : zoomAt(2.5, e.clientX, e.clientY))}
                  onPointerDown={(e) => {
                    if (lensRef.current.k <= 1 || e.button !== 0) return;
                    e.currentTarget.setPointerCapture(e.pointerId);
                    dragging.current = { x: e.clientX, y: e.clientY, lx: lensRef.current.x, ly: lensRef.current.y };
                  }}
                  onPointerMove={(e) => {
                    const d = dragging.current;
                    if (!d) return;
                    paint(bound(lensRef.current.k, d.lx + e.clientX - d.x, d.ly + e.clientY - d.y));
                  }}
                  onPointerUp={() => (dragging.current = null)}
                  onPointerCancel={() => (dragging.current = null)}
                >
                  {/* The picture made the size the stage draws it at — every pixel used, none
                      thrown away by the engine — over the last one painted, or on first opening
                      over the grid's thumbnail, until it is decoded. */}
                  <img
                    key={shown.id}
                    className="px-shown"
                    src={viewSrc(shown)}
                    // What was asked for, kept whatever the fallback below does to `src`.
                    data-picture={viewSrc(shown)}
                    alt={shown.filename ?? ""}
                    decoding="async"
                    draggable={false}
                    // Opening from the grid, the tile's own picture (already decoded) is painted
                    // under it until it is decoded; on a step the last sharp photo is (px-under),
                    // never a thumbnail.
                    style={under ? { ...fitted(shown), backgroundImage: "none" } : fitted(shown)}
                    onLoad={(e) => {
                      const img = e.currentTarget;
                      const url = viewSrc(shown);
                      learnShape(shown, img);
                      const done = () => {
                        decoded.add(url);
                        painted.current = shown;
                        if (underEl.current) underEl.current.style.visibility = "hidden";
                      };
                      img.decode().then(done, done);
                    }}
                    // Never the engine's broken-picture box (a white frame, a question mark, the
                    // file's name): a picture that will not come falls back to the thumbnail,
                    // and past that shows nothing over the thumbnail already painted under it.
                    onError={(e) => {
                      const img = e.currentTarget;
                      const thumb = pictureSrc(shown.id, "thumb");
                      if (!img.src.includes("thumb")) img.src = thumb;
                      else img.dataset.failed = "";
                    }}
                  />
                  {moving === shown.id && videoBase && (
                    // The moving part, laid exactly over the still, played once and gone —
                    // turned as it was recorded (a selfie's is a quarter turn and a mirror),
                    // and not shown until that turn is known, so it never shows sideways.
                    // In the still's own box, clipped to it: the film drawn the still's size and
                    // shape whatever its own pixels, and only once a frame of it is here — until
                    // then, and once it ends, the still itself shows, never a black frame.
                    <div className="px-live-frame" key={`${shown.id}-live`}>
                      <video
                        className="px-live"
                        src={`${videoBase}/${shown.id}/video`}
                        style={{
                          ...(turn?.key === `${shown.id}/video` ? turnStyle(turn.value, shapeOf(shown).w, shapeOf(shown).h, turn.zoom) : {}),
                          opacity: 0,
                        }}
                        autoPlay
                        playsInline
                        onTimeUpdate={(e) => {
                          const v = e.currentTarget;
                          if (v.style.opacity === "0" && v.currentTime > 0.04 && v.readyState >= 2 && turn?.key === `${shown.id}/video`) v.style.opacity = "1";
                        }}
                        onEnded={() => setMoving(null)}
                        onError={() => setMoving(null)}
                      />
                    </div>
                  )}
                  {/* Zoomed in, the whole picture: only then are its own pixels the ones seen. */}
                  {zoomed && (
                    <img
                      key={`${shown.id}-full`}
                      className="px-sharp"
                      src={pictureSrc(shown.id, "full")}
                      alt=""
                      aria-hidden="true"
                      decoding="async"
                      draggable={false}
                      onLoad={(e) => {
                        e.currentTarget.classList.add("is-in");
                        fadeIn(e.currentTarget, { duration: 160 });
                      }}
                    />
                  )}
                </div>
                </>
              )}
              {/* What kind of photo it is, as Photos marks it at the stage's corner: a Live
                  Photo plays its moving part while the pointer rests on LIVE; a burst's BURST
                  goes through all of it. */}
              {shown.kind === "live" && videoBase && !zoomed && (
                <button
                  className={`px-mark${moving === shown.id ? " is-on" : ""}`}
                  aria-label="Play Live Photo"
                  onPointerEnter={() => setMoving(shown.id)}
                  onFocus={() => setMoving(shown.id)}
                  onClick={() => setMoving(shown.id)}
                >
                  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
                    <circle cx="8" cy="8" r="2.2" fill="currentColor" />
                    <circle cx="8" cy="8" r="4.6" fill="none" stroke="currentColor" strokeWidth="1.3" />
                    <circle cx="8" cy="8" r="7" fill="none" stroke="currentColor" strokeWidth="1.1" strokeDasharray="1.2 1.6" />
                  </svg>
                  LIVE
                </button>
              )}
              {shown.burst && (shown.burstCount ?? 0) > 1 && !inBurst && !zoomed && (
                <button className="px-mark" aria-label={`View all ${shown.burstCount} photos in this burst`} onClick={() => void viewBurst(shown)}>
                  <Layers size={13} aria-hidden="true" />
                  BURST
                  <span className="px-mark-count">{shown.burstCount}</span>
                </button>
              )}
              {inBurst && (
                <div className="px-mark is-burst" role="status">
                  <Layers size={13} aria-hidden="true" />
                  Burst · {(open ?? 0) + 1} of {inBurst.count}
                  <button className="px-mark-done" onClick={() => void leaveBurst(true)}>
                    Done
                  </button>
                </div>
              )}
              {open !== null && open > 0 && (
                <button className="px-nav is-prev" aria-label="Previous" onClick={() => step(-1)}>
                  <ChevronLeft size={22} />
                </button>
              )}
              {open !== null && photos && open < photos.length - 1 && (
                <button className="px-nav is-next" aria-label="Next" onClick={() => step(1)}>
                  <ChevronRight size={22} />
                </button>
              )}
            </div>
            {open !== null && photos && photos.length > 1 && (!fullscreen || thumbs) && (
              <Filmstrip photos={photos} at={open} onPick={(i) => step(i - open)} />
            )}
            {info && (
              <aside className="px-info" aria-label="Info">
                {/* As in Photos' Info: the title and caption first, typed in place, and kept on
                    every device; Return or leaving the field saves them, Escape puts them back. */}
                <div className="px-caption-row" onBlur={(e) => wordsLeft(shown, e)}>
                  <input
                    key={`t${shown.id}`}
                    ref={titleField}
                    className="px-title-field"
                    aria-label="Title"
                    placeholder="Add a Title"
                    defaultValue={shown.title ?? ""}
                    maxLength={255}
                    readOnly={place === "recentlyDeleted"}
                    onKeyDown={(e) => wordKeys(e, shown.title)}
                  />
                  <textarea
                    key={`c${shown.id}`}
                    ref={captionField}
                    className="px-caption"
                    aria-label="Caption"
                    placeholder="Add a Caption"
                    defaultValue={shown.caption ?? ""}
                    maxLength={2000}
                    rows={1}
                    readOnly={place === "recentlyDeleted"}
                    onInput={(e) => grow(e.currentTarget)}
                    onFocus={(e) => grow(e.currentTarget)}
                    onKeyDown={(e) => wordKeys(e, shown.caption)}
                  />
                </div>
                {/* When, large, with Adjust beside it, as Photos' Info leads with the date. */}
                <div className="px-info-when">
                  <div>
                    <strong>{new Date(shown.takenAt * 1000).toLocaleDateString(undefined, { dateStyle: "full" })}</strong>
                    <span>{new Date(shown.takenAt * 1000).toLocaleTimeString(undefined, { timeStyle: "short" })}</span>
                  </div>
                  {place !== "recentlyDeleted" && (
                    <button className="px-link px-adjust" onClick={() => setAdjusting(shown)}>
                      Adjust
                    </button>
                  )}
                </div>
                {/* The file, as Photos sets it: its name, then what it is, in one quiet block. */}
                <div className="px-info-file">
                  <strong>{shown.filename ?? "Photo"}</strong>
                  <span>
                    {shown.kind === "video" ? `Video${shown.duration ? `, ${clock(shown.duration)}` : ""}` : shown.kind === "live" ? "Live Photo" : "Photo"}
                    {shown.width && shown.height && shown.kind !== "video" ? `, ${Math.round((shown.width * shown.height) / 1e6)} MP` : ""}
                  </span>
                  <span>
                    {shown.width.toLocaleString()} × {shown.height.toLocaleString()}
                    {shown.size ? `, ${shown.size >= 1024 * 1024 ? `${(shown.size / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(shown.size / 1024))} KB`}` : ""}
                  </span>
                </div>
                <dl className="px-facts">
                  {/* Stars, as Info sets them: a click gives that many; the same again takes them away. */}
                  <div className="px-fact">
                    <dt>Rating</dt>
                    <dd className="px-rating" role="radiogroup" aria-label="Rating">
                      {[1, 2, 3, 4, 5].map((n) => (
                        <button
                          key={n}
                          role="radio"
                          aria-checked={(shown.rating ?? 0) === n}
                          aria-label={`${n} ${n === 1 ? "star" : "stars"}`}
                          className={`px-star${(shown.rating ?? 0) >= n ? " is-on" : ""}`}
                          disabled={place === "recentlyDeleted"}
                          onClick={() => void rate(shown, (shown.rating ?? 0) === n ? 0 : n)}
                        >
                          <Star size={15} fill={(shown.rating ?? 0) >= n ? "currentColor" : "none"} strokeWidth={1.8} />
                        </button>
                      ))}
                    </dd>
                  </div>
                  {shown.latitude != null && shown.longitude != null && (
                    <div className="px-fact">
                      <dt>Place</dt>
                      <dd className="px-place">
                        <span>{degrees(shown.latitude, "N", "S")}, {degrees(shown.longitude, "E", "W")}</span>
                        <button className="px-link" onClick={() => void api.photoOpenMap(shown.id).catch((e) => setError(messageOf(e)))}>
                          Open in Map
                        </button>
                      </dd>
                    </div>
                  )}
                  {/* Keywords, as Info keeps them: each its own chip, taken away with ×; a word
                      typed and Return adds one. */}
                  <div className="px-fact">
                    <dt>Keywords</dt>
                    <dd className="px-words-field">
                      {(shown.keywords ?? []).map((k) => (
                        <span key={k} className="px-chip">
                          {k}
                          {place !== "recentlyDeleted" && (
                            <button
                              aria-label={`Remove ${k}`}
                              onClick={() => void keywords(shown, (shown.keywords ?? []).filter((x) => x !== k))}
                            >
                              <X size={10} strokeWidth={3} aria-hidden="true" />
                            </button>
                          )}
                        </span>
                      ))}
                      {place !== "recentlyDeleted" && (
                        <input
                          key={`k${shown.id}`}
                          className="px-keyword-field"
                          aria-label="Add a Keyword"
                          placeholder="Add a Keyword"
                          maxLength={100}
                          onKeyDown={(e) => {
                            const word = e.currentTarget.value.trim();
                            if ((e.key === "Enter" || e.key === ",") && word) {
                              e.preventDefault();
                              e.currentTarget.value = "";
                              if (!(shown.keywords ?? []).some((x) => x.toLowerCase() === word.toLowerCase())) {
                                void keywords(shown, [...(shown.keywords ?? []), word]);
                              }
                            } else if (e.key === "Escape") {
                              e.currentTarget.value = "";
                              e.currentTarget.blur();
                              e.stopPropagation();
                            }
                          }}
                        />
                      )}
                    </dd>
                  </div>
                </dl>
              </aside>
            )}
          </div>
        )}
      </section>
      {menu && <RowMenu at={menu.at} entries={menu.entries} onClose={() => setMenu(null)} />}
      {asking}
      {/* Drawn at the top of the page: inside the screen, whose content moves by a transform,
          "fixed" was fixed to the screen and a slideshow played inside a box, not the window. */}
      {show && (
        <Portal>
          <Slideshow ids={show.ids} start={show.start} load={(ids) => store.load(ids)} videoBase={videoBase} onClose={() => setShow(null)} />
        </Portal>
      )}
      {adjusting && (
        <Portal>
          <AdjustDate photo={adjusting} onSave={(t) => void redate(adjusting, t)} onClose={() => setAdjusting(null)} />
        </Portal>
      )}
      {options && (
        <Portal>
          <DownloadOptions photos={options} onPick={(w) => void downloadAs(options.map((p) => p.id), w)} onClose={() => setOptions(null)} />
        </Portal>
      )}
      <UploadPanel uploads={[...uploads.values()]} onDismiss={dismissUploads} cancel={api.photosUploadCancel} />
    </div>
  );
}

/**
 * The Apple Account password asked for before Hidden shows, as icloud.com asks for it. Checked
 * by the app (against the password it keeps, else with Apple); a wrong one says so here and
 * the field is cleared for another try.
 */
export function UnlockSheet({ onCancel, onUnlocked }: { onCancel: () => void; onUnlocked: () => void }) {
  const sheetRef = useRef<HTMLFormElement | null>(null);
  useModalFocus(sheetRef);
  const field = useRef<HTMLInputElement | null>(null);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  /** Windows Hello set up on this computer: offered first, as Face ID is on a phone. */
  const [hello, setHello] = useState(false);
  useEffect(() => field.current?.focus(), []);
  useEffect(() => {
    api.photosHelloAvailable().then(setHello, () => setHello(false));
  }, []);
  const withHello = async () => {
    setBusy(true);
    setProblem(null);
    try {
      const outcome = await api.photosUnlockHiddenHello();
      if (outcome === "verified") return onUnlocked();
      if (outcome === "unavailable") setHello(false);
    } catch (e) {
      setProblem(messageOf(e));
    }
    setBusy(false);
    field.current?.focus();
  };
  const go = async () => {
    if (!password || busy) return;
    setBusy(true);
    setProblem(null);
    try {
      await api.photosUnlockHidden(password);
      onUnlocked();
    } catch (e) {
      setProblem(messageOf(e));
      setPassword("");
      setBusy(false);
      field.current?.focus();
    }
  };
  return (
    <Portal>
      <div className="modal-veil" onClick={onCancel}>
        <form
          ref={sheetRef}
          className="modal-sheet confirm px-unlock"
          role="dialog"
          aria-modal="true"
          aria-label="View Hidden Photos"
          onClick={(e) => e.stopPropagation()}
          onSubmit={(e) => {
            e.preventDefault();
            void go();
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              onCancel();
            }
          }}
        >
          <h2>View Hidden Photos</h2>
          <p>Enter the password for your Apple Account.</p>
          <input
            ref={field}
            className="field"
            type="password"
            autoComplete="current-password"
            placeholder="Password"
            aria-label="Password"
            aria-invalid={problem ? true : undefined}
            aria-describedby={problem ? "px-unlock-problem" : undefined}
            value={password}
            disabled={busy}
            onChange={(e) => setPassword(e.target.value)}
          />
          {problem && (
            <p id="px-unlock-problem" className="px-unlock-problem" role="alert">
              {problem}
            </p>
          )}
          {hello && (
            <button type="button" className="quiet px-unlock-hello" disabled={busy} onClick={() => void withHello()}>
              Use Windows Hello
            </button>
          )}
          <div className="confirm-actions">
            <button type="button" className="quiet" onClick={onCancel}>
              Cancel
            </button>
            <button type="submit" className="primary confirm-answer" disabled={!password || busy}>
              {busy ? "Checking…" : "View Album"}
            </button>
          </div>
        </form>
      </div>
    </Portal>
  );
}
