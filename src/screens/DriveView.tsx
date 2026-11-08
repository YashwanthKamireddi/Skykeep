import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AppFoot } from "../components/AppFoot";
import { overlayOpen, useActiveRef, useScreenActive } from "../lib/screen";
import { useModalFocus } from "../lib/focus";
import { useRevision } from "../lib/sync";
import { toastSaved, useErrorToast } from "../components/Toast";
import { ConfirmSheet, useConfirm } from "../components/Confirm";
import { EmptyState } from "../components/EmptyState";
import { Spinner } from "../components/Spinner";
import { PaneTitle } from "../components/PaneTitle";
import { Portal } from "../components/Portal";
import { pickOpen } from "../lib/pick";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  takesDriveCopies,
  api,
  driveName,
  isContainer,
  messageOf,
  DRIVE_ROOT,
  UPLOAD,
  type DriveItem,
  type FolderNow,
  type RecentItem,
  type TrashItem,
  type UploadEvent,
} from "../lib/ipc";
import { ActionGlyph, FileGlyph } from "../components/FileGlyph";
import { UploadTray } from "../components/UploadTray";
import { DownloadTray } from "../components/DownloadTray";
import { startDownload } from "../lib/downloads";
import { listen } from "@tauri-apps/api/event";
import { Picker } from "../components/Picker";
import { RowMenu } from "../components/RowMenu";
import { dateText, timeText } from "../lib/when";
import { Clock, Cloud, Trash2 } from "lucide-react";
import { useReveal } from "../lib/reveal";
import { keepSame } from "../lib/same";
import { carryLabel } from "../lib/dragPicture";
import { copyName, copyText, keepBothName } from "../lib/clipboard";
import { useChosen, useIsChosen, type Chosen } from "../lib/chosen";
import { icloudSize } from "../lib/icloudSize";

/** iCloud Drive's own four orders, as its "by Name" menu offers them. */
const SORTS = [
  { key: "name", label: "Name" },
  { key: "date", label: "Date" },
  { key: "kind", label: "Kind" },
  { key: "size", label: "Size" },
] as const;
type SortKey = (typeof SORTS)[number]["key"];
const SORT_KEY = "quince.drive.sort";
const SORT_REVERSED = "quince.drive.sortReversed";

/**
 * Names compare as Finder compares them — "File 2" before "File 10", case ignored; dates and
 * sizes newest and largest first. Folders keep together at the top under every order but
 * Kind, which is itself the grouping.
 */
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
function sortItems(rows: DriveItem[], by: SortKey, reversed = false): DriveItem[] {
  const name = (a: DriveItem, b: DriveItem) => collator.compare(a.name, b.name);
  const folderFirst = (a: DriveItem, b: DriveItem) => Number(isContainer(b)) - Number(isContainer(a));
  const kindOf = (i: DriveItem) => (i.extension ?? "").toLowerCase();
  const order: Record<SortKey, (a: DriveItem, b: DriveItem) => number> = {
    name: (a, b) => folderFirst(a, b) || name(a, b),
    date: (a, b) => folderFirst(a, b) || (b.modifiedAt ?? b.createdAt ?? 0) - (a.modifiedAt ?? a.createdAt ?? 0) || name(a, b),
    // Folders are a kind of their own, first: told apart by a space for an extension, they
    // sorted after a file with none at all ("Makefile", "README").
    kind: (a, b) => folderFirst(a, b) || collator.compare(kindOf(a), kindOf(b)) || name(a, b),
    size: (a, b) => folderFirst(a, b) || (b.size ?? 0) - (a.size ?? 0) || name(a, b),
  };
  // A second click on a heading turns the order round, as in Finder; folders stay on top.
  const turn = reversed ? -1 : 1;
  const sorter = by === "kind"
    ? (a: DriveItem, b: DriveItem) => turn * order.kind(a, b)
    : (a: DriveItem, b: DriveItem) => folderFirst(a, b) || turn * order[by](a, b);
  return [...rows].sort(sorter);
}

const KB = 1024;

function fileSize(bytes: number | null): string {
  if (bytes === null) return "";
  if (bytes < KB) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / KB;
  let unit = 0;
  while (value >= KB && unit < units.length - 1) {
    value /= KB;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/** As Finder writes Date Modified: "Today, 10:12", "Yesterday, 18:40", then the date. */
const modified = (seconds: number | null, now = new Date()) => {
  if (seconds === null) return "";
  const d = new Date(seconds * 1000);
  const days = Math.round(
    (new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime() -
      new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()) / 864e5,
  );
  if (days === 0 || days === 1) {
    return `${days === 0 ? "Today" : "Yesterday"}, ${timeText(d, { hour: "numeric", minute: "2-digit" })}`;
  }
  return dateText(d, { day: "numeric", month: "short", year: "numeric" });
};

/** Apple busy, or the network away for a moment: a failure that passes on its own. */
function passing(message: string): boolean {
  return /busy|try (this one )?again in a minute|offline|can.t reach|network|timed out/i.test(message);
}

/**
 * Recently Deleted, as icloud.com shows it: what was deleted, when, and two ways out —
 * Recover, back to where it was, and Delete Now, for good. Each leaves the list the moment it
 * is asked for; a refusal puts the row back and says why. Delete Now asks first, as Apple's
 * does, because nothing brings it back.
 */
function DriveTrash({ onClose }: { onClose: () => void }) {
  const [rows, setRows] = useState<TrashItem[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useErrorToast(error, () => setError(null));
  const read = useCallback(async () => {
    try {
      setRows(await api.driveTrash());
    } catch (e) {
      setError(messageOf(e));
      setRows((r) => r ?? []);
    }
  }, []);
  useEffect(() => {
    void read();
  }, [read]);

  // Every item asked for goes in one request, as icloud.com's Recover All and Delete All
  // send them; one request each, all at once, Apple turned most away as busy. They leave the
  // list at once; whatever Apple did not do comes back, with the reason.
  const out = async (gone: TrashItem[], recover: boolean) => {
    const ids = new Set(gone.map((i) => i.drivewsid));
    setRows((r) => (r ?? []).filter((i) => !ids.has(i.drivewsid)));
    setError(null);
    try {
      const refused = await api.driveTrashOut(gone, recover);
      if (refused.length > 0) {
        setError(
          refused.length === 1
            ? refused[0].reason
            : `${refused.length} of ${gone.length} items could not be ${recover ? "recovered" : "deleted"}: ${refused[0].reason}`,
        );
        await read();
      }
    } catch (e) {
      setError(messageOf(e));
      await read();
    }
  };
  const recover = (items: TrashItem[]) => out(items, true);
  const [confirm, confirmSheet] = useConfirm();
  const destroy = (items: TrashItem[]) => {
    const what = items.length === 1 ? `\u201c${driveName(items[0])}\u201d` : `these ${items.length} items`;
    void confirm({ title: `Delete ${what} now?`, body: "You can\u2019t undo this.", action: "Delete", danger: true, symbol: "trash" }).then(
      (sure) => sure && void out(items, false),
    );
  };

  return (
    <div className="drive drive-trash">
      <header className="pane-head drive-bar">
        <PaneTitle before={<nav className="crumbs" aria-label="Folder path"><button className="crumb" onClick={onClose}>iCloud Drive</button></nav>}>
          <button className="crumb" aria-current>Recently Deleted</button>
        </PaneTitle>
        {rows && rows.length > 0 && (
          <div className="toolbar-group">
            <button className="tool has-label is-accent" onClick={() => void recover(rows)}>Recover All</button>
            <button className="tool has-label danger" onClick={() => destroy(rows)}>Delete All</button>
          </div>
        )}
      </header>
      {rows === null && <EmptyState busy>Loading…</EmptyState>}
      {rows && rows.length === 0 && !error && <EmptyState>Nothing in Recently Deleted.</EmptyState>}
      {confirmSheet}
      {rows && rows.length > 0 && (
        <ul className="drive-rows">
          {rows.map((item) => (
            <li key={item.drivewsid}>
              <div className="drive-row">
                <span className="drive-row-hit">
                  <span className="drive-badge" aria-hidden="true">
                    <FileGlyph kind={item.kind} extension={item.extension} />
                  </span>
                  <span className="drive-name" title={item.restorePath ? `From ${item.restorePath}` : undefined}>
                    {driveName(item)}
                  </span>
                  <span className="drive-meta drive-kind">
                    {item.restorePath ? `From ${item.restorePath.split("/").slice(0, -1).join("/") || "Drive"}` : kindName(item)}
                  </span>
                  <span className="drive-meta">{isContainer(item) ? "" : fileSize(item.size)}</span>
                  <span className={`drive-meta${daysLeft(item.expiresAt) !== null && (daysLeft(item.expiresAt) ?? 99) <= 3 ? " is-soon" : ""}`}>
                    {timeLeft(item.expiresAt)}
                  </span>
                </span>
                <span className="drive-row-actions is-shown">
                  <button className="link" onClick={() => void recover([item])}>Recover</button>
                  <button className="link danger" onClick={() => destroy([item])}>Delete Now</button>
                </span>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * Recents, as icloud.com and Finder have it: the files opened most recently, under Today,
 * Yesterday, Previous 7 Days, Previous 30 Days and then each month. Read from Apple each
 * time it is opened, since what was opened on the phone a minute ago belongs here too.
 */
function DriveRecents({ onClose }: { onClose: () => void }) {
  const [rows, setRows] = useState<RecentItem[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useErrorToast(error, () => setError(null));
  const [busy, setBusy] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ item: RecentItem; x: number; y: number; alignRight: boolean } | null>(null);
  useEffect(() => {
    api.driveRecents().then(
      (found) => setRows(Array.isArray(found) ? found : []),
      (e) => {
        setError(messageOf(e));
        setRows([]);
      },
    );
  }, []);
  const download = async (item: RecentItem) => {
    setBusy(item.drivewsid);
    setError(null);
    try {
      toastSaved(await api.driveDownload(item.drivewsid, item));
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(null);
    }
  };
  const groups = groupRecents(rows ?? []);
  return (
    <div className="drive drive-recents">
      <header className="pane-head drive-bar">
        <PaneTitle before={<nav className="crumbs" aria-label="Folder path"><button className="crumb" onClick={onClose}>iCloud Drive</button></nav>}>
          <button className="crumb" aria-current>Recents</button>
        </PaneTitle>
      </header>
      {rows === null && <EmptyState busy>Loading…</EmptyState>}
      {rows && rows.length === 0 && !error && <EmptyState>Files you open show here, newest first.</EmptyState>}
      {groups.map((g) => (
        <section key={g.heading} aria-label={g.heading}>
          <h2 className="drive-group">{g.heading}</h2>
          <ul className="drive-rows">
            {g.items.map((item) => (
              <li key={item.drivewsid}>
                <div
                  className={`drive-row${selected === item.drivewsid ? " is-selected" : ""}${busy === item.drivewsid ? " is-busy" : ""}`}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setSelected(item.drivewsid);
                    setMenu({ item, x: e.clientX, y: e.clientY, alignRight: false });
                  }}
                >
                  <button
                    className="drive-row-hit"
                    aria-pressed={selected === item.drivewsid}
                    onClick={() => setSelected(item.drivewsid)}
                    onDoubleClick={() => void download(item)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        void download(item);
                      }
                    }}
                  >
                    <span className="drive-badge" aria-hidden="true">
                      {busy === item.drivewsid ? <Spinner small /> : <FileGlyph kind={item.kind} extension={item.extension} />}
                    </span>
                    <span className="drive-name">{driveName(item)}</span>
                    <span className="drive-meta drive-kind">{kindName(item)}</span>
                    <span className="drive-meta">{fileSize(item.size)}</span>
                    <span className="drive-meta">{openedText(item.openedAt)}</span>
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </section>
      ))}
      {menu && (
        <RowMenu
          at={menu}
          onClose={() => setMenu(null)}
          entries={[{ label: "Download a Copy", run: () => void download(menu.item) }]}
        />
      )}
    </div>
  );
}

/** Recents' headings, newest first: Today, Yesterday, Previous 7 and 30 Days, then months. */
export function groupRecents(rows: RecentItem[], now = new Date()): { heading: string; items: RecentItem[] }[] {
  const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const today = midnight(now);
  const out: { heading: string; items: RecentItem[] }[] = [];
  for (const item of rows) {
    const when = new Date((item.openedAt ?? 0) * 1000);
    const ago = Math.round((today - midnight(when)) / 864e5);
    const heading =
      ago <= 0 ? "Today"
      : ago === 1 ? "Yesterday"
      : ago <= 7 ? "Previous 7 Days"
      : ago <= 30 ? "Previous 30 Days"
      : dateText(when, when.getFullYear() === now.getFullYear() ? { month: "long" } : { month: "long", year: "numeric" });
    const last = out[out.length - 1];
    if (last?.heading === heading) last.items.push(item);
    else out.push({ heading, items: [item] });
  }
  return out;
}

const openedText = (s: number | null) => {
  if (s === null) return "";
  const d = new Date(s * 1000);
  const today = new Date();
  return d.toDateString() === today.toDateString()
    ? timeText(d, { hour: "numeric", minute: "2-digit" })
    : dateText(d, { day: "numeric", month: "short", year: "numeric" });
};

/** What a file's kind is called, as icloud.com's File Info names it. */
const KINDS: Record<string, string> = {
  pdf: "PDF document", png: "PNG image", jpg: "JPEG image", jpeg: "JPEG image", heic: "HEIC image",
  gif: "GIF image", webp: "WebP image", mp4: "MPEG-4 movie", mov: "QuickTime movie", mp3: "MP3 audio",
  m4a: "Apple MPEG-4 audio", wav: "WAVE audio", txt: "Plain text document", md: "Markdown document",
  csv: "Comma-separated values", json: "JSON document", zip: "ZIP archive", html: "HTML document",
  docx: "Office Open XML word processing document", doc: "Microsoft Word document",
  xlsx: "Office Open XML spreadsheet", pptx: "Office Open XML presentation",
  pages: "Pages document", numbers: "Numbers spreadsheet", key: "Keynote presentation",
};
function kindName(item: DriveItem) {
  if (item.kind === "FOLDER") return "Folder";
  if (item.kind === "APP_LIBRARY") return "Application Library";
  const ext = (item.extension ?? "").toLowerCase();
  return KINDS[ext] ?? (ext ? `${ext.toUpperCase()} file` : "Document");
}

/** File Info, as icloud.com shows it: the mark, the name, then what it is and where. */
function InfoSheet({ item, where, thumb, onClose }: { item: DriveItem; where: string; thumb?: string; onClose: () => void }) {
  const done = useRef<HTMLButtonElement | null>(null);
  const sheet = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheet);
  const live = useActiveRef();
  useEffect(() => {
    const id = window.setTimeout(() => done.current?.focus(), 0);
    const onKey = (e: KeyboardEvent) => live.current && e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => { window.clearTimeout(id); window.removeEventListener("keydown", onKey); };
  }, [onClose]);
  const when = (s: number | null) => (s === null ? "—" : new Date(s * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }));
  const rows: [string, string][] = [
    ["Kind", kindName(item)],
    ...(isContainer(item)
      ? item.childCount === null ? [] : [["Contents", `${item.childCount} item${item.childCount === 1 ? "" : "s"}`] as [string, string]]
      : [["Size", fileSize(item.size)] as [string, string]]),
    ["Where", where],
    ["Created", when(item.createdAt)],
    ["Modified", when(item.modifiedAt ?? item.createdAt)],
  ];
  return (
    <Portal>
    <div className="modal-veil" onClick={onClose}>
      <div ref={sheet} className="modal-sheet info-sheet" role="dialog" aria-modal="true" aria-label={isContainer(item) ? "Folder Info" : "File Info"} onClick={(e) => e.stopPropagation()}>
        <header className="info-head">
          <h2>{isContainer(item) ? "Folder Info" : "File Info"}</h2>
          <button ref={done} className="link info-done" onClick={onClose}>Done</button>
        </header>
        <div className={`info-mark${thumb ? " has-thumb" : ""}`} aria-hidden="true">
          {thumb ? <img src={thumb} alt="" draggable={false} /> : <FileGlyph kind={item.kind} extension={item.extension} />}
        </div>
        <p className="info-name">{driveName(item)}</p>
        <dl className="info-rows">
          {rows.map(([k, v]) => (
            <div key={k}><dt>{k}</dt><dd>{v}</dd></div>
          ))}
        </dl>
      </div>
    </div>
    </Portal>
  );
}

/** Days until Apple deletes it for good, rounded up; `null` when Apple did not say. */
function daysLeft(expiresAt: number | null): number | null {
  if (expiresAt === null) return null;
  return Math.max(0, Math.ceil((expiresAt * 1000 - Date.now()) / 86_400_000));
}
/**
 * Why a name can't be used, in icloud.com's own words (iclouddrive bundle, createFolder chunk),
 * or null: not empty, not hidden (a leading dot), no ":" or line break, and not already taken
 * in this folder, whatever the case.
 */
export function nameProblem(full: string, isFile: boolean, siblings: DriveItem[], self?: DriveItem): string | null {
  const kind = isFile ? "file" : "folder";
  if (!full.trim()) return `The name \u201c${full}\u201d is invalid. Please choose a different name.`;
  if (full.startsWith(".")) return `The name of your ${kind} shouldn\u2019t begin with a \u201c.\u201d (dot). Please choose a different name.`;
  if (/[:\n\r]/.test(full)) {
    return `The name \u201c${full}\u201d can\u2019t be used. Try using a name with fewer characters, or with no punctuation marks.`;
  }
  const lower = full.toLocaleLowerCase();
  const taken = siblings.some((i) => i !== self && i.drivewsid !== self?.drivewsid && driveName(i).toLocaleLowerCase() === lower);
  return taken ? `The name \u201c${full}\u201d is already taken. Please choose a different name.` : null;
}

/** "21 days", "1 day", "Today" — icloud.com's Time Left. */
function timeLeft(expiresAt: number | null): string {
  const d = daysLeft(expiresAt);
  if (d === null) return "";
  return d === 0 ? "Today" : `${d} day${d === 1 ? "" : "s"}`;
}

type Crumb = { id: string; name: string };

/** Whether a folder read back is the same as what is on screen: the same items, each at the
 * same version and name. */
function sameItems(a: DriveItem[], b: DriveItem[]): boolean {
  if (a.length !== b.length) return false;
  const key = (i: DriveItem) => `${i.drivewsid}\u0000${i.etag ?? ""}\u0000${i.name}\u0000${i.extension ?? ""}\u0000${i.size ?? ""}`;
  const seen = new Set(a.map(key));
  return b.every((i) => seen.has(key(i)));
}

/**
 * Files and folders of iCloud Drive's own: what is in an app's library (Keynote, Shortcuts) is in
 * that library's zone (see takesDriveCopies), so it is not copied out of it.
 */
const copyable = (item: DriveItem) => (item.kind === "FILE" || item.kind === "FOLDER") && takesDriveCopies(item.drivewsid);

/** What a paste onto a name already there does: both kept ("Report 2.pdf"), or the one there replaced. */
type Clash = "keep" | "replace" | null;

/** A delete the person chose not to go on with: the item stays, and nothing is said. */
const KEPT = "kept";

/** A folder changed since it was shown, said as it now stands: what is in it, and what is new. */
export function folderNowWords(now: FolderNow): string {
  const n = (k: number, one: string) => `${k} ${one}${k === 1 ? "" : "s"}`;
  const since = [
    now.added ? `${n(now.added, "item")} added` : "",
    now.changed ? `${n(now.changed, "item")} changed` : "",
    now.gone ? `${n(now.gone, "item")} removed` : "",
  ].filter(Boolean);
  const said = since.length > 1 ? `${since.slice(0, -1).join(", ")} and ${since[since.length - 1]}` : since[0];
  const holds = `This folder now has ${n(now.items, "item")}`;
  return said
    ? `${holds}, with ${said} since it was shown here, on another device. It goes to Recently Deleted with everything in it.`
    : `${holds}. It changed since it was shown here, on another device. It goes to Recently Deleted with everything in it.`;
}

export function DriveView() {
  const [trail, setTrail] = useState<Crumb[]>([{ id: DRIVE_ROOT, name: "Drive" }]);
  // Asked before a folder that changed since it was shown is deleted.
  const [confirm, confirmSheet] = useConfirm();
  /**
   * Back and Forward, as Finder's: the folders opened before this one, and those gone back
   * from, by whole path. Every way of reaching a folder — a double-click, the path, a search,
   * a folder taken away elsewhere — is a step; a new one after going back drops what was ahead.
   */
  const history = useRef<{ back: Crumb[][]; ahead: Crumb[][]; last: Crumb[]; stepping: boolean }>({ back: [], ahead: [], last: trail, stepping: false });
  useEffect(() => {
    const h = history.current;
    if (h.last === trail) return;
    const same = h.last.length === trail.length && h.last.every((c, i) => c.id === trail[i].id);
    if (!same && !h.stepping) {
      h.back.push(h.last);
      h.ahead = [];
    }
    h.stepping = false;
    h.last = trail;
  }, [trail]);
  const travel = (by: -1 | 1) => {
    const h = history.current;
    const to = (by < 0 ? h.back : h.ahead).pop();
    if (!to) return;
    (by < 0 ? h.ahead : h.back).push(h.last);
    h.stepping = true;
    setTrail(to);
  };
  const [items, setItems] = useState<DriveItem[]>([]);
  const [inTrash, setInTrash] = useState(false);
  const [inRecents, setInRecents] = useState(false);
  /** Apple's own pictures of this folder's files, where it has them; the rest keep icons. */
  const [thumbs, setThumbs] = useState<Record<string, string>>({});
  /** The folder whose pictures have been asked for and answered since it was opened. */
  const thumbsOf = useRef<string | null>(null);
  /** What is free, as icloud.com says it under a folder's name: "5.18 GB available". */
  const [available, setAvailable] = useState<string | null>(null);
  useEffect(() => {
    // Shown only when Apple says: an empty or odd answer leaves the count on its own.
    api.storageInfo().then(
      (s) => setAvailable(
        s && Number.isFinite(s.totalBytes) && Number.isFinite(s.usedBytes)
          ? icloudSize(Math.max(0, s.totalBytes - s.usedBytes))
          : null),
      () => setAvailable(null),
    );
  }, []);
  /** icloud.com's "View as": Grid or List, remembered. */
  const [grid, setGrid] = useState(() => {
    try {
      return window.localStorage.getItem("quince.drive.view") === "grid";
    } catch {
      return false;
    }
  });
  const chooseView = (asGrid: boolean) => {
    setGrid(asGrid);
    try {
      window.localStorage.setItem("quince.drive.view", asGrid ? "grid" : "list");
    } catch {
      // Shown so for now; forgotten on restart.
    }
  };
  const [sortBy, setSortBy] = useState<SortKey>(() => {
    try {
      const saved = window.localStorage.getItem(SORT_KEY);
      return SORTS.some((o) => o.key === saved) ? (saved as SortKey) : "name";
    } catch {
      return "name";
    }
  });
  const [reversed, setReversed] = useState(() => {
    try {
      return window.localStorage.getItem(SORT_REVERSED) === "1";
    } catch {
      return false;
    }
  });
  /** An order, and which way round: a heading clicked again turns it, as in Finder. */
  const chooseSort = (key: SortKey, turn = false) => {
    const nextReversed = turn && key === sortBy ? !reversed : false;
    setSortBy(key);
    setReversed(nextReversed);
    try {
      window.localStorage.setItem(SORT_KEY, key);
      window.localStorage.setItem(SORT_REVERSED, nextReversed ? "1" : "0");
    } catch {
      // Sorted for now; forgotten on restart.
    }
  };
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useErrorToast(error, () => setError(null));
  // Every item with something under way — two downloads at once each keep their spinner: one
  // value, the second took the first's, and the first one ending cleared the second's.
  const [busyIds, setBusyIds] = useState<ReadonlySet<string>>(new Set());
  const markBusy = useCallback((id: string, on: boolean) => {
    setBusyIds((was) => {
      if (was.has(id) === on) return was;
      const next = new Set(was);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);
  /** A file just saved to Downloads, and where — shown for a moment, then cleared. */
  const [renaming, setRenaming] = useState<string | null>(null);
  const [creatingFolder, setCreatingFolder] = useState(false);
  const [newFolderName, setNewFolderName] = useState("");
  /** The item a Move dialog is open for, if any. */
  const [moving, setMoving] = useState<DriveItem[] | null>(null);
  const [info, setInfo] = useState<DriveItem | null>(null);
  /**
   * What is selected, as Finder selects: a click one, Command/Ctrl-click one more or one
   * fewer, Shift-click the run. The last one picked is the one Return, F2 and Space act on.
   */
  const [picked, setPicked] = useState<string[]>([]);
  const selected = picked.length ? picked[picked.length - 1] : null;
  /** What is selected, told to the rows one by one (lib/chosen). */
  const chosen = useChosen(picked);
  const setSelected = (id: string | null) => setPicked(id ? [id] : []);
  /** Where a Shift-click or Shift-arrow run starts. */
  const anchor = useRef<string | null>(null);
  /** Quick Look: the selected file, large, while Space is held to it. */
  const [peeking, setPeeking] = useState(false);
  // A file or folder found by the search across everything: its folder opened, with the whole
  // path above it in the trail, and it selected and brought into view — as Finder's "Show in
  // Enclosing Folder". The path is walked up from the library, which knows every folder seen.
  const revealing = useRef<string | null>(null);
  useReveal("drivews", (r) => {
    void (async () => {
      const crumbs: Crumb[] = [];
      let at = r.within ?? null;
      for (let hops = 0; at && at !== DRIVE_ROOT && hops < 64; hops++) {
        const folder: DriveItem | null = await api.driveItem(at).catch(() => null);
        if (!folder) break;
        crumbs.unshift({ id: folder.drivewsid, name: driveName(folder) });
        at = folder.parentId ?? null;
      }
      setInTrash(false);
      setInRecents(false);
      setTrail([{ id: DRIVE_ROOT, name: "Drive" }, ...crumbs]);
      revealing.current = r.id;
      setSelected(r.id);
    })();
  });
  useEffect(() => {
    const id = revealing.current;
    if (!id || !items.some((i) => i.drivewsid === id)) return;
    revealing.current = null;
    requestAnimationFrame(() =>
      document.querySelector(`.drive [data-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "center" }),
    );
  }, [items]);
  /** The ⋯ menu, or the same menu opened by a right-click, for one item. */
  const [menu, setMenu] = useState<{ item: DriveItem; x: number; y: number; alignRight: boolean } | null>(null);
  /** The folder's own menu, from a right-click on nothing; with what the clipboard held then. */
  const [spaceMenu, setSpaceMenu] = useState<{ x: number; y: number; alignRight: boolean; files: number; other: string | null } | null>(null);
  /** Items of Drive's own being dragged onto a folder — one, or all selected — and the folder it is over. */
  const dragItem = useRef<DriveItem[] | null>(null);
  const [dropOn, setDropOn] = useState<string | null>(null);
  /** True while files are held over the window, so the drop target can say so. */
  const [dropping, setDropping] = useState(false);

  const here = trail[trail.length - 1];
  // The folder on screen now, for work that finishes later. A refresh bound to the folder
  // open when an upload or a new folder began re-read that folder when it ended — after the
  // person had gone elsewhere — and its rows replaced the folder they were in: Drive "went
  // back to the previous location".
  const hereRef = useRef(here.id);
  hereRef.current = here.id;
  const trailRef = useRef(trail);
  trailRef.current = trail;
  /** Items deleted or moved out of a folder that Apple's listing of it may still carry for a
   *  moment after — each kept as "folder:item", so it is hidden only in the folder it left.
   *  By item alone, an item moved into another folder, or recovered from Recently Deleted,
   *  stayed hidden everywhere until the app restarted. */
  const [gone, setGone] = useState<Set<string>>(new Set());
  const goneKey = (folder: string, item: string) => `${folder}:${item}`;

  // A folder load is asynchronous and the id it was for is not always the one still on
  // screen by the time it resolves — a fast double click used to let a second, third,
  // fourth open() stack navigation onto a folder that had not loaded yet, each one seeing
  // the same stale row and appending an identical crumb for it. This is what a real trail
  // fifteen "Downloads" deep, all pointing at the one actual folder, turned out to be.
  const openToken = useRef(0);

  // Why the last folder was left, kept on screen in the one stepped back to.
  const leftBecause = useRef<string | null>(null);

  const open = useCallback(async (folderId: string) => {
    const token = ++openToken.current;
    setLoading(true);
    setError(leftBecause.current);
    leftBecause.current = null;
    try {
      // Show what is stored first, then fetch — a folder visited before opens instantly.
      const cached = await api.driveChildren(folderId);
      if (token !== openToken.current) return;
      // Always this folder's own rows, even none: keeping the last folder's rows on screen
      // while a slow fetch ran meant a click landed on a row from the level just left, and
      // Drive seemed to jump into folders nobody opened.
      setItems(Array.isArray(cached) ? cached : []);
      setThumbs({});
      thumbsOf.current = null;
      try {
        await api.driveSyncFolder(folderId);
      } catch (e) {
        // Apple busy or out of reach while the folder kept here is already on screen: nothing
        // the person asked for failed, so nothing is said. Asked again a little later, quietly;
        // the next sync brings it anyway. Anything else (the folder gone) is said as before.
        if (!(Array.isArray(cached) && cached.length > 0 && passing(messageOf(e)))) throw e;
        const again = (wait: number, left: number) =>
          window.setTimeout(async () => {
            if (token !== openToken.current) return;
            try {
              await api.driveSyncFolder(folderId);
              const fresh = await api.driveChildren(folderId);
              if (token === openToken.current) setItems(Array.isArray(fresh) ? fresh : []);
            } catch (e2) {
              if (left > 0 && passing(messageOf(e2))) again(wait * 3, left - 1);
            }
          }, wait);
        again(5_000, 2);
      }
      if (token !== openToken.current) return;
      const fresh = await api.driveChildren(folderId);
      if (token !== openToken.current) return;
      setItems(Array.isArray(fresh) ? fresh : []);
      // Pictures last, and never in the way: a folder is usable before they arrive.
      api.driveThumbnails(folderId).then(
        (t) => {
          if (token === openToken.current && t) {
            thumbsOf.current = folderId;
            setThumbs(t);
          }
        },
        () => {},
      );
    } catch (e) {
      if (token !== openToken.current) return;
      setError(messageOf(e));
      // Deleted or moved on another device: the backend has forgotten it, so step back out
      // to the nearest folder still here rather than sit in one that is not.
      if (folderId !== DRIVE_ROOT && (await api.driveItem(folderId).catch(() => undefined)) === null) {
        if (token !== openToken.current) return;
        leftBecause.current = messageOf(e);
        setTrail((t) => {
          const at = t.findIndex((c) => c.id === folderId);
          return at > 0 ? t.slice(0, at) : t;
        });
      }
    } finally {
      if (token === openToken.current) {
        setLoading(false);
        setLoaded(true);
      }
    }
  }, []);

  useEffect(() => {
    void open(here.id);
  }, [open, here.id]);

  function enter(item: DriveItem) {
    if (!isContainer(item)) return;
    // Never append a crumb for the folder already open — the second half of the fix, so
    // even a click that does land twice cannot duplicate the trail.
    if (item.drivewsid === here.id) return;
    setCreatingFolder(false);
    setTrail((t) => [...t, { id: item.drivewsid, name: driveName(item) }]);
  }

  /**
   * The open folder again, quietly: after files land, and whenever a sync finishes, so what
   * changed in it on another device shows without the folder being opened again. No
   * "loading", and the pictures stay: re-opening the folder wiped the thumbnails and drew
   * them back, a blink across the grid every time a file arrived.
   */
  const refresh = useCallback(async (ask = true) => {
    const folderId = hereRef.current;
    const token = openToken.current;
    try {
      // Not asked again when the backend has just listed it (`quince://drive-changed`).
      if (ask) await api.driveSyncFolder(folderId);
      if (token !== openToken.current) return;
      const fresh = await api.driveChildren(folderId);
      if (token !== openToken.current) return;
      // Redrawn only when something in it changed: a check that found nothing new leaves the
      // rows, their selection and their pictures exactly as they were.
      const next = Array.isArray(fresh) ? fresh : [];
      const same = sameItems(itemsRef.current, next);
      // And then only the rows that changed: the others keep their objects (lib/same).
      setItems((was) => (sameItems(was, next) ? was : keepSame(was, next, (i) => i.drivewsid)));
      // Pictures only when something in the folder changed (or they never came): a check that
      // found nothing new already has every picture it had, and asking again every ten seconds
      // redrew every row with them.
      if (same && thumbsOf.current === folderId) return;
      api.driveThumbnails(folderId).then(
        (t) => {
          if (token !== openToken.current || !t) return;
          thumbsOf.current = folderId;
          setThumbs((was) => (Object.entries(t).every(([id, src]) => was[id] === src) ? was : { ...was, ...t }));
        },
        () => {},
      );
    } catch {
      // A quiet refresh that fails changes nothing; opening the folder says what is wrong.
    }
  }, []);
  // Apple sends no push for iCloud Drive, but Drive's change feed names each item changed
  // anywhere in it within a second (src-tauri drive_watch.rs). The backend lists again the
  // folders it touched and says which: the one in front is redrawn from the library, with no
  // request of its own. Coming back to the window still looks at once, for the app libraries
  // (Pages, Numbers…) the feed does not cover.
  const shown = useScreenActive();
  const shownNow = useRef(shown);
  shownNow.current = shown;
  useEffect(() => {
    if (!shown) return undefined;
    // Never over a change of this computer's own still on its way: a check read before a
    // rename reached Apple would put the old name back for a moment.
    const look = () => {
      if (document.visibilityState === "visible" && !writingRef.current) void refresh();
    };
    document.addEventListener("visibilitychange", look);
    window.addEventListener("focus", look);
    return () => {
      document.removeEventListener("visibilitychange", look);
      window.removeEventListener("focus", look);
    };
  }, [shown, refresh]);
  // Heard for as long as the screen lives, not only while it shows: a folder that changed
  // behind another app is read when it comes back (the look above).
  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    listen<string[]>("quince://drive-changed", (e) => {
      if (shownNow.current && !writingRef.current && e.payload.includes(hereRef.current)) void refresh(false);
    })
      .then((unlisten) => {
        // Stopping may throw where there is no event system behind the page (see the
        // uploads' listener below), and a screen closing must not throw for that.
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
  }, [refresh]);
  const revision = useRevision("drive");
  // A pass that lands while another screen is in front is read when this one comes back, once
  // (a screen out of sight stays mounted): each landing asked Apple for the folder again.
  const seen = useRef(revision);
  if (shown) seen.current = revision;
  const due = seen.current;
  const firstRevision = useRef(due);
  useEffect(() => {
    if (due === firstRevision.current) return;
    void refresh();
  }, [due, refresh]);

  const download = useCallback(async (item: DriveItem) => {
    markBusy(item.drivewsid, true);
    setError(null);
    try {
      toastSaved(await api.driveDownload(item.drivewsid));
    } catch (e) {
      setError(messageOf(e));
    } finally {
      markBusy(item.drivewsid, false);
    }
  }, [markBusy]);

  /**
   * A copy in Downloads of what is chosen: one file as before, saved with a notice; a folder,
   * or several things, as the same tree — shown in the corner as it arrives, one line for all
   * of it, with Show in Folder once it is there.
   */
  const downloadItems = useCallback((chosen: DriveItem[]) => {
    if (chosen.length === 0) return;
    if (chosen.length === 1 && !isContainer(chosen[0])) {
      void download(chosen[0]);
      return;
    }
    setError(null);
    void startDownload(chosen, chosen.length === 1 ? driveName(chosen[0]) : `${chosen.length} items`);
  }, [download]);


  const rename = useCallback(
    async (item: DriveItem, name: string) => {
      setRenaming(null);
      const trimmed = name.trim();
      if (trimmed === driveName(item)) return;
      const problem = nameProblem(trimmed, item.kind === "FILE", items, item);
      if (problem) {
        setError(problem);
        return;
      }
      markBusy(item.drivewsid, true);
      setError(null);
      // The new name shows the moment Return is pressed; Apple's answer confirms or undoes it.
      const ext = item.extension ? `.${item.extension}` : "";
      const shown = ext && trimmed.endsWith(ext)
        ? { name: trimmed.slice(0, -ext.length), extension: item.extension }
        : { name: trimmed, extension: null };
      setItems((rows) => rows.map((r) => (r.drivewsid === item.drivewsid ? { ...r, ...shown } : r)));
      try {
        await api.driveRename(item.drivewsid, trimmed);
        await refresh();
      } catch (e) {
        setItems((rows) => rows.map((r) => (r.drivewsid === item.drivewsid ? item : r)));
        setError(messageOf(e));
      } finally {
        markBusy(item.drivewsid, false);
      }
    },
    [refresh, items, markBusy]
  );

  /**
   * Delete, as Finder does it: no question, since it goes to Recently Deleted and comes back
   * from there. The row leaves at once and stays gone — Apple's listing can carry it for a
   * moment, and it used to flicker back — unless Apple refuses, when it returns and says why.
   */
  const removeAll = useCallback(
    async (list: DriveItem[]) => {
      if (list.length === 0) return;
      setError(null);
      const from = hereRef.current;
      const ids = list.map((i) => goneKey(from, i.drivewsid));
      setGone((g) => new Set([...g, ...ids]));
      const refused: string[] = [];
      let reason = "";
      // One after another: Apple turns a burst of deletes away as busy.
      for (const item of list) {
        // A folder still being uploaded into stops taking files first: deleting it while files
        // went on landing was what Apple would not do.
        if (isContainer(item)) {
          // Files dropped onto it, or a folder of that name being made in the folder it is in —
          // not one of the same name being made somewhere else.
          const into = [...uploadsRef.current.values()].filter((u) => {
            if (["done", "failed", "cancelled"].includes(u.state)) return false;
            const dest = batchInto.current.get(u.batch);
            if (dest === item.drivewsid) return true;
            return dest === from && ((u.folder && !u.within && u.name === item.name) || u.within.split("/")[0] === item.name);
          });
          for (const batch of new Set(into.map((u) => u.batch))) await api.driveUploadCancel([], batch).catch(() => {});
        }
        try {
          // Only as shown: a folder changed since (a file added on another device) is asked
          // about with what it holds now, and goes only as that.
          let removed = await api.driveDelete(item.drivewsid);
          while (removed.kind === "changed") {
            const sure = await confirm({
              title: `Delete “${driveName(item)}”?`,
              body: folderNowWords(removed),
              action: "Delete Anyway",
              danger: true,
              symbol: "trash",
            });
            if (!sure) throw new Error(KEPT);
            removed = await api.driveDelete(item.drivewsid, removed.etag);
          }
        } catch (e) {
          refused.push(item.drivewsid);
          if (messageOf(e) !== KEPT) reason = messageOf(e);
        }
      }
      if (refused.length) {
        setGone((g) => {
          const next = new Set(g);
          for (const id of refused) next.delete(goneKey(from, id));
          return next;
        });
        if (reason) setError(list.length === 1 ? reason : `${refused.length} of ${list.length} items could not be deleted: ${reason}`);
      }
      if (refused.length < list.length) await refresh();
    },
    [refresh, confirm]
  );
  const remove = useCallback((item: DriveItem) => removeAll([item]), [removeAll]);

  /**
   * Duplicate, as Finder: the copy lands beside the original as "Name copy", named as Copy and
   * Paste names one in its own folder (the same backend copy), and shows as being made at once.
   */
  const duplicate = useCallback(
    async (item: DriveItem) => {
      setError(null);
      // By the item as well as the moment: several duplicated at once share a millisecond.
      const key = `copy-${item.drivewsid}-${Date.now()}`;
      const taken = [
        ...itemsRef.current.map(driveName),
        ...pendingRef.current.filter((p) => p.folder === here.id).map((p) => p.name),
      ];
      setPending((p) => [...p, { key, folder: here.id, name: copyName(item.name, item.extension, taken), doing: "Duplicating…" }]);
      try {
        await api.driveCopy(item.drivewsid, here.id);
        await refresh();
      } catch (e) {
        setError(messageOf(e));
      } finally {
        setPending((p) => p.filter((x) => x.key !== key));
      }
    },
    [refresh, here.id]
  );

  /**
   * Move, from the folder open (a drag, Move to Folder) or from the folder they were copied in
   * (Ctrl-Alt-V, as Finder's Option-Command-V). Gone from where they were at once, and shown
   * where they go when that is the folder open; back, with the reason, if Apple refuses.
   */
  const moveAll = useCallback(
    async (list: DriveItem[], destinationId: string, from: string = hereRef.current) => {
      setMoving(null);
      setError(null);
      const going = list.filter((i) => i.drivewsid !== destinationId);
      if (going.length === 0) return;
      setGone((g) => new Set([...g, ...going.map((i) => goneKey(from, i.drivewsid))]));
      const batch = `move-${Date.now()}`;
      if (destinationId === hereRef.current) {
        setPending((p) => [
          ...p,
          ...going.map((i) => ({ key: `${batch}:${i.drivewsid}`, folder: destinationId, name: driveName(i), doing: "Moving…" })),
        ]);
      }
      const refused: string[] = [];
      let reason = "";
      for (const item of going) {
        try {
          await api.driveMove(item.drivewsid, destinationId);
        } catch (e) {
          refused.push(item.drivewsid);
          reason = messageOf(e);
          settle(`${batch}:${item.drivewsid}`);
        }
      }
      if (refused.length) {
        setGone((g) => {
          const next = new Set(g);
          for (const id of refused) next.delete(goneKey(from, id));
          return next;
        });
        setError(going.length === 1 ? reason : `${refused.length} of ${going.length} items could not be moved: ${reason}`);
      }
      if (refused.length < going.length) await refresh();
      settle(batch);
    },
    [refresh]
  );

  /**
   * Copy and Paste, as Finder: Ctrl-C keeps what is chosen (and puts its names on the
   * desktop's clipboard as text, as Finder does, so the clipboard says what was copied last);
   * Ctrl-V in any folder puts copies there. Kept across folders, and for more than one paste.
   */
  const [clip, setClip] = useState<{ items: DriveItem[]; from: string; text: string | null } | null>(null);
  /** What has the name of each of `list` in the folder open, by the item pasted. */
  const clashesHere = (list: DriveItem[]) => {
    const here = new Map(itemsRef.current.map((i) => [driveName(i).toLocaleLowerCase(), i]));
    const out = new Map<string, DriveItem>();
    for (const i of list) {
      const there = here.get(driveName(i).toLocaleLowerCase());
      if (there && there.drivewsid !== i.drivewsid) out.set(i.drivewsid, there);
    }
    return out;
  };
  /** Finder's question when a pasted name is taken: Keep Both, Replace, or Cancel (null). */
  const [asking, setAsking] = useState<{ items: DriveItem[]; answer: (how: Clash) => void } | null>(null);
  const askClash = (items: DriveItem[]) =>
    new Promise<Clash>((answer) => setAsking({ items, answer: (how) => { setAsking(null); answer(how); } }));
  const clipRef = useRef(clip);
  clipRef.current = clip;
  const copyItems = useCallback(async (list: DriveItem[]) => {
    const copying = list.filter(copyable);
    if (copying.length === 0) return;
    const text = copying.map(driveName).join("\n");
    const from = hereRef.current;
    setClip({ items: copying, from, text: null });
    if (await copyText(text)) setClip((c) => (c && c.items === copying ? { ...c, text } : c));
  }, []);

  /**
   * Copies of `list` in the folder open, each shown at once under the name Finder would give
   * it there and made one after another (Apple turns a burst of writes away as busy). One that
   * fails leaves, with the reason; the rest still land.
   */
  const copyHere = useCallback(
    async (list: DriveItem[], clash: Clash = null) => {
      setError(null);
      const into = hereRef.current;
      const inside = trailRef.current.map((c) => c.id);
      // Never into itself, or into a folder within it, as Finder will not.
      const insideItself = list.find((i) => inside.includes(i.drivewsid));
      const copying = list.filter((i) => !inside.includes(i.drivewsid));
      if (insideItself) setError(`\u201c${driveName(insideItself)}\u201d can\u2019t be copied into a folder inside itself.`);
      if (copying.length === 0) return;
      // Replace: what has the name goes to Recently Deleted first, so the copy takes its name.
      const replacing = clash === "replace" ? clashesHere(copying) : new Map<string, DriveItem>();
      if (replacing.size) setGone((g) => new Set([...g, ...[...replacing.values()].map((o) => goneKey(into, o.drivewsid))]));
      const taken = new Set([
        ...itemsRef.current.filter((i) => ![...replacing.values()].includes(i)).map(driveName),
        ...pendingRef.current.filter((p) => p.folder === into).map((p) => p.name),
      ]);
      const batch = `paste-${Date.now()}`;
      const kept = clash === "keep" ? clashesHere(copying) : new Map<string, DriveItem>();
      const shown = copying.map((item, i) => {
        const ext = isContainer(item) ? null : item.extension;
        const name = kept.has(item.drivewsid) ? keepBothName(item.name, ext, taken) : copyName(item.name, ext, taken);
        taken.add(name);
        return { key: `${batch}:${i}`, folder: into, name, doing: "Copying…" };
      });
      setPending((p) => [...p, ...shown]);
      const refused: string[] = [];
      let reason = "";
      for (const [i, item] of copying.entries()) {
        try {
          const old = replacing.get(item.drivewsid);
          if (old) {
            await api.driveDelete(old.drivewsid).then((removed) => {
              if (removed.kind === "changed") throw new Error(`“${driveName(old)}” changed on another device since it was shown here, so it wasn’t replaced.`);
            }).catch((e) => {
              // Still there: shown again, and this one not copied over it.
              setGone((g) => { const next = new Set(g); next.delete(goneKey(into, old.drivewsid)); return next; });
              throw e;
            });
          }
          // Kept both: the backend names the copy as Finder names a copy ("Report copy.pdf"),
          // and it is then named as Keep Both names it ("Report 2.pdf").
          const listed = async () => { const l = await api.driveChildren(into); return Array.isArray(l) ? l : []; };
          const before = kept.has(item.drivewsid) ? await listed() : null;
          await api.driveCopy(item.drivewsid, into);
          if (before) {
            const named = copyName(item.name, isContainer(item) ? null : item.extension, before.map(driveName)).toLocaleLowerCase();
            const made = (await listed()).find((x) => !before.some((b) => b.drivewsid === x.drivewsid) && driveName(x).toLocaleLowerCase() === named);
            if (made && driveName(made) !== shown[i].name) await api.driveRename(made.drivewsid, shown[i].name);
          }
          // The backend has read the folder again: the library has the copy, so no new look.
          if (into === hereRef.current) await refresh(false);
        } catch (e) {
          refused.push(item.drivewsid);
          reason = messageOf(e);
        } finally {
          settle(shown[i].key);
        }
      }
      if (refused.length) {
        setError(copying.length === 1 ? reason : `${refused.length} of ${copying.length} items could not be copied: ${reason}`);
      }
    },
    [refresh]
  );


  /**
   * Send files from this machine to the folder currently open.
   *
   * Only the paths cross into the backend; the bytes are read and streamed there, which is
   * what lets a large file go up without the interface holding it in memory.
   *
   * One file Apple refuses must not look like all of them failing, so each is reported by
   * name and the rest still land.
   */
  /** Every file sent from this window, until the tray is closed. */
  const [uploads, setUploads] = useState<Map<string, UploadEvent>>(new Map());
  const [uploadMenu, setUploadMenu] = useState(false);
  /**
   * What has been asked for here and is not back from Apple yet, drawn at once as rows of
   * its own: a folder being created, files going up. Nothing showed until Apple answered —
   * seconds later — so a press seemed to do nothing, and then a folder appeared from nowhere.
   */
  const [pending, setPending] = useState<{ key: string; folder: string; name: string; doing: string }[]>([]);
  const pendingRef = useRef(pending);
  pendingRef.current = pending;
  // What is kept on this computer — null while the iCloud Drive folder is off.
  const [kept, setKept] = useState<Set<string> | null>(null);
  useEffect(() => {
    api.driveKept().then((k) => setKept(k ? new Set(k) : null), () => setKept(null));
  }, [here.id]);
  /** Kept or let go of at once on screen; put back with the reason if it cannot be. */
  const keep = async (item: DriveItem, on: boolean) => {
    const was = kept;
    setKept((k) => {
      const next = new Set(k ?? []);
      if (on) next.add(item.drivewsid);
      else next.delete(item.drivewsid);
      return next;
    });
    try {
      setKept(new Set(await api.driveKeep(item.drivewsid, on)));
    } catch (e) {
      setKept(was);
      setError(messageOf(e));
    }
  };
  /**
   * The folder whose row is under a point the window system gave (physical pixels): files
   * dropped there from this machine go into it, in an app's library as anywhere else.
   */
  const folderAt = (x: number, y: number): string | null => {
    const ratio = window.devicePixelRatio || 1;
    const under = document.elementFromPoint(x / ratio, y / ratio) as HTMLElement | null;
    const id = under?.closest?.<HTMLElement>(".drive-rows [data-id]")?.dataset.id;
    const item = id ? itemsRef.current.find((i) => i.drivewsid === id) : undefined;
    return item && item.kind === "FOLDER" ? item.drivewsid : null;
  };
  const itemsRef = useRef(items);
  itemsRef.current = items;
  const settle = (key: string) => setPending((p) => p.filter((x) => !x.key.startsWith(key)));
  const writingRef = useRef(false);
  writingRef.current = busyIds.size > 0 || renaming !== null || pending.length > 0;

  // At most one re-read of the open folder every couple of seconds while files land.
  const landedTimer = useRef<number | null>(null);
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  const landed = () => {
    if (landedTimer.current !== null) return;
    landedTimer.current = window.setTimeout(() => {
      landedTimer.current = null;
      void refreshRef.current();
    }, 2000);
  };
  useEffect(() => () => {
    if (landedTimer.current !== null) window.clearTimeout(landedTimer.current);
  }, []);

  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    listen<UploadEvent>(UPLOAD, (event) => {
      const u = event.payload;
      setUploads((was) => {
        const next = new Map(was);
        const before = next.get(u.id);
        // Events for one file can pass each other; a finished file never goes back.
        if (before && (before.state === "done" || before.state === "failed" || before.state === "cancelled")) return was;
        next.set(u.id, u);
        return next;
      });
      // Each file that lands shows as it lands, not all at once when the batch ends.
      if (u.state === "done" && !u.folder) landed();
    })
      .then((unlisten) => {
        // Stopping may throw where there is no event system behind the page — a plain
        // browser running the interface — and a screen closing must not throw for that.
        const quietly = () => {
          try {
            // It answers with a promise, whose failure would otherwise escape this try.
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

  const uploadsRef = useRef(uploads);
  uploadsRef.current = uploads;
  /** The folder each upload batch was sent into, so deleting a folder stops only its own. */
  const batchInto = useRef(new Map<string, string>());
  const dismissUploads = useCallback(() => setUploads(new Map()), []);

  const upload = useCallback(
    async (paths: string[], target?: string) => {
      if (paths.length === 0) return;
      setError(null);
      // Never blocks the window: browse, open another folder, start another upload.
      const batch = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
      // The open folder, or a folder in it the files were dropped on.
      const into = target ?? here.id;
      batchInto.current.set(batch, into);
      setPending((p) => [
        ...p,
        ...paths.map((path, i) => ({
          key: `${batch}:${i}`,
          folder: into,
          name: path.split(/[\\/]/).filter(Boolean).pop() ?? path,
          doing: "Uploading…",
        })),
      ]);
      try {
        await api.driveUpload(batch, into, paths);
        // What failed is named in the tray, file by file; the folder is re-read either way —
        // a folder in it that took the files shows its new count.
        if (into === hereRef.current || target) await refresh();
      } catch (e) {
        setError(messageOf(e));
      } finally {
        settle(batch);
        // The batch is over: the backend has said all it will. A row of it still open — a
        // report that never came, an upload stopped part-way — is closed now, so nothing can
        // sit at "0 of 80" for ever.
        setUploads((was) => {
          let changed = false;
          const next = new Map(was);
          for (const [id, u] of was) {
            if (u.batch !== batch || ["done", "failed", "cancelled", "planned"].includes(u.state)) continue;
            next.set(id, { ...u, state: u.folder ? "failed" : "cancelled", error: "Stopped before it finished" });
            changed = true;
          }
          return changed ? next : was;
        });
      }
    },
    [here.id, refresh]
  );

  /**
   * Paste into the folder open. Whatever was copied last decides: files copied in the
   * desktop's file manager go up into it, as a drop would send them; else what Drive copied
   * is copied here — or, with Alt (Option on a Mac), moved here, as Finder's Option-Command-V.
   */
  // Each render's own, as the keys are: it uploads into the folder open now.
  const paste = async (move: boolean) => {
    const board = await api.clipboardPaste().catch(() => null);
    if (board?.files.length) {
      await upload(board.files);
      return;
    }
    const c = clipRef.current;
    if (!c) return;
    // Drive copies its items only beside the original: not into an app's library, a zone of
    // its own. Files from this machine go up into one (above).
    if (!takesDriveCopies(hereRef.current)) {
      setError("Items from elsewhere in iCloud Drive can't be pasted into an app's folder. Upload files from this computer instead.");
      return;
    }
    // Something else copied since (text in another app): that is what is on the clipboard.
    if (c.text !== null && board?.text != null && board.text !== c.text) return;
    const into = hereRef.current;
    if (move && c.from === into) return;
    const inside = trailRef.current.map((t) => t.id);
    const insideItself = c.items.find((i) => inside.includes(i.drivewsid));
    if (move && insideItself) {
      setError(`\u201c${driveName(insideItself)}\u201d can\u2019t be moved into a folder inside itself.`);
      return;
    }
    // Pasted where it was copied, a copy is a duplicate ("Report copy.pdf"), as in Finder.
    // Anywhere else a name already there is a question first: Keep Both or Replace.
    const clashing = c.from === into ? [] : [...clashesHere(c.items.filter((i) => !inside.includes(i.drivewsid))).keys()];
    let how: Clash = null;
    if (clashing.length) {
      how = await askClash(c.items.filter((i) => clashing.includes(i.drivewsid)));
      if (!how) return;
    }
    if (!move) {
      await copyHere(c.items, how);
      return;
    }
    const clashes = clashesHere(c.items);
    if (how === "replace" && clashes.size) {
      const old = [...clashes.values()];
      setGone((g) => new Set([...g, ...old.map((o) => goneKey(into, o.drivewsid))]));
      for (const o of old) {
        try {
          const removed = await api.driveDelete(o.drivewsid);
          if (removed.kind === "changed") throw new Error(`“${driveName(o)}” changed on another device since it was shown here, so it wasn’t replaced. ${folderNowWords(removed).split(".")[0]}.`);
        } catch (e) {
          // Still there: shown again, and nothing moved over it.
          setGone((g) => { const next = new Set(g); for (const x of old) next.delete(goneKey(into, x.drivewsid)); return next; });
          setError(messageOf(e));
          return;
        }
      }
    }
    let going = c.items;
    if (how === "keep" && clashes.size) {
      // Named as Keep Both names it before it goes, so the folder never holds two of one name.
      const taken = new Set(itemsRef.current.map(driveName));
      const renamed = new Map<string, DriveItem>();
      for (const item of c.items.filter((i) => clashes.has(i.drivewsid))) {
        const ext = isContainer(item) ? null : item.extension;
        const name = keepBothName(item.name, ext, taken);
        taken.add(name);
        try {
          await api.driveRename(item.drivewsid, name);
        } catch (e) {
          setError(messageOf(e));
          return;
        }
        renamed.set(item.drivewsid, { ...item, name: ext ? name.slice(0, -(ext.length + 1)) : name });
      }
      going = c.items.map((i) => renamed.get(i.drivewsid) ?? i);
    }
    // Moved, it is no longer where it was copied: nothing is left to paste.
    setClip(null);
    await moveAll(going, into, c.from);
  };

  const chooseAndUpload = useCallback(
    async (folders: boolean) => {
      setUploadMenu(false);
      try {
        const chosen = await pickOpen({
          multiple: true,
          directory: folders,
          title: folders ? "Upload folders to iCloud Drive" : "Upload to iCloud Drive",
        });
        if (!chosen) return;
        await upload(chosen);
      } catch (e) {
        setError(messageOf(e));
      }
    },
    [upload]
  );

  // Dropping files on the window uploads them to the folder that is open. Tauri hands over
  // paths rather than contents, so a drop costs nothing until the file is actually read.
  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    // Outside the app shell — a browser running the interface on its own, as the checks do —
    // there is no webview to ask, and asking throws where it stands. Uploading by button
    // still works, so this is a capability that may be missing, not a failure.
    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          // A row of Drive's own being moved is not a file arriving from this machine.
          if (dragItem.current) return;
          // Only while Drive is the screen in front: a file dropped onto a note, with Drive
          // kept in the background, went up to iCloud Drive as well.
          if (!shownNow.current) return;
          const p = event.payload;
          // Over a folder's row, the files go into that folder, as Finder drops them; anywhere
          // else, into the folder that is open.
          const folder = p.type === "over" || p.type === "drop" ? folderAt(p.position.x, p.position.y) : null;
          if (p.type === "over" || p.type === "enter") {
            setDropping(!folder);
            setDropOn((d) => (d === folder ? d : folder));
          } else if (p.type === "leave") {
            setDropping(false);
            setDropOn(null);
          } else if (p.type === "drop") {
            setDropping(false);
            setDropOn(null);
            void upload(p.paths, folder ?? undefined);
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
  }, [upload, here.id]);

  /**
   * One folder per name typed. Enter submits, which closes the field, and closing it blurs
   * the field, which submitted again with the same name still in hand — Apple made the folder
   * the first time and declined the second, and the refusal was what showed.
   */
  const folderPending = useRef(false);
  const createFolder = useCallback(async () => {
    if (folderPending.current) return;
    const name = newFolderName.trim();
    setCreatingFolder(false);
    setNewFolderName("");
    if (!name) return;
    // Finder will not put two things of one name in a folder, nor a hidden or unusable one; neither will iCloud.
    const problem = nameProblem(name, false, items);
    if (problem) {
      setError(problem);
      return;
    }
    folderPending.current = true;
    setError(null);
    const key = `folder-${Date.now()}`;
    setPending((p) => [...p, { key, folder: here.id, name, doing: "Creating…" }]);
    try {
      await api.driveCreateFolder(here.id, name);
      await refresh();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      folderPending.current = false;
      settle(key);
    }
  }, [newFolderName, here.id, refresh, items]);

  /** The rows as shown, sorted once per change to what is here or to the order. */
  const rows = useMemo(
    () => sortItems(items.filter((item) => !gone.has(goneKey(here.id, item.drivewsid))), sortBy, reversed),
    [items, gone, sortBy, reversed, here.id],
  );
  const pickedItems = useMemo(() => rows.filter((r) => picked.includes(r.drivewsid)), [rows, picked]);
  const primary = useMemo(() => (selected ? rows.find((r) => r.drivewsid === selected) ?? null : null), [rows, selected]);
  const listRef = useRef<HTMLUListElement | null>(null);
  const [listFocused, setListFocused] = useState(false);
  /** A row brought into view and given the focus, as the arrows leave it. */
  const showRow = (id: string) =>
    requestAnimationFrame(() => {
      const hit = listRef.current?.querySelector<HTMLButtonElement>(`[data-id="${CSS.escape(id)}"] .drive-row-hit`);
      hit?.scrollIntoView({ block: "nearest" });
      hit?.focus({ preventScroll: true });
    });
  /** Everything from the anchor to `to`, in the order shown, with `to` last — the one acted on. */
  const run = (to: string) => {
    const order = rows.map((r) => r.drivewsid);
    const from = anchor.current && order.includes(anchor.current) ? anchor.current : to;
    const [a, z] = [order.indexOf(from), order.indexOf(to)].sort((x, y) => x - y);
    return [...order.slice(a, z + 1).filter((id) => id !== to), to];
  };
  const openItem = (item: DriveItem) => (isContainer(item) ? enter(item) : void download(item));

  /**
   * What a row does, as one object that never changes, so a row is drawn again only when what
   * it shows changes: in a folder of thousands, a click redrew every row in it. The handlers
   * read the folder's state as it is when they run.
   */
  const latest = useRef({ rows, picked, run, openItem, rename, moveAll });
  latest.current = { rows, picked, run, openItem, rename, moveAll };
  const rowActions = useMemo<DriveRowActions>(
    () => ({
      click: (item, e) => {
        const { picked, run } = latest.current;
        const id = item.drivewsid;
        if (e.metaKey || e.ctrlKey) {
          // Command/Ctrl-click: one more, or one fewer, keeping the rest.
          anchor.current = id;
          setPicked(picked.includes(id) ? picked.filter((p) => p !== id) : [...picked, id]);
        } else if (e.shiftKey && anchor.current) {
          setPicked(run(id));
        } else {
          anchor.current = id;
          setPicked([id]);
        }
      },
      open: (item) => latest.current.openItem(item),
      menu: (item, at) => {
        const { picked } = latest.current;
        // A right-click on something not selected selects it, as in Finder; on the selection,
        // it is for all of it.
        if (!picked.includes(item.drivewsid)) {
          anchor.current = item.drivewsid;
          setPicked([item.drivewsid]);
        }
        setMenu({ item, ...at });
      },
      dragStart: (item, e) => {
        const { rows, picked } = latest.current;
        // Dragging a selected row takes the whole selection with it.
        dragItem.current = picked.includes(item.drivewsid) ? rows.filter((r) => picked.includes(r.drivewsid)) : [item];
        e.dataTransfer.effectAllowed = "move";
        e.dataTransfer.setData("application/x-quince-drive", item.drivewsid);
        carryLabel(e, driveName(item), dragItem.current.length);
      },
      dragEnd: () => {
        dragItem.current = null;
        setDropOn(null);
      },
      dragOver: (item, e) => {
        const moving = dragItem.current;
        if (!moving || item.kind !== "FOLDER" || moving.some((m) => m.drivewsid === item.drivewsid)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        setDropOn((d) => (d === item.drivewsid ? d : item.drivewsid));
      },
      dragLeave: (item) => setDropOn((d) => (d === item.drivewsid ? null : d)),
      drop: (item, e) => {
        e.preventDefault();
        const moving = dragItem.current;
        dragItem.current = null;
        setDropOn(null);
        if (moving && item.kind === "FOLDER" && !moving.some((m) => m.drivewsid === item.drivewsid)) {
          void latest.current.moveAll(moving, item.drivewsid);
        }
      },
      rename: (item, name) => void latest.current.rename(item, name),
      cancelRename: () => setRenaming(null),
    }),
    [],
  );

  // Finder's keys (Ctrl in place of Command): the arrows walk the selection — across and down
  // in Grid — and Shift with them widens it; Return or Ctrl-Down opens, Ctrl-Up goes up a
  // folder, Ctrl-[ and Ctrl-] go back and forward, F2 renames, Ctrl-D duplicates, Ctrl-C and
  // Ctrl-V copy and paste (Ctrl-Alt-V moves), Delete moves to Recently Deleted, Ctrl-A selects
  // all, and Space shows the selected file large, as Quick Look does.
  const live = useActiveRef();
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e: KeyboardEvent) => {
    if (!live.current || e.defaultPrevented || inTrash || inRecents) return;
    if (renaming || creatingFolder || moving || info) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest?.("input, textarea, select, [contenteditable], .drive-bar, .upload-menu")) return;
    if (!peeking && overlayOpen()) return;
    // Return and Space press a focused button of their own; on a row or nowhere, they are Drive's.
    // A row's ⋯ is a button of its own inside the rows: Return there opens its menu, not the file.
    const ownButton = !!target?.closest?.("button, a[href]") && !target?.closest?.(".drive-row-hit");
    const onRows = !target || target === document.body || (!ownButton && !!target.closest?.(".drive-rows, .drive-peek"));
    const mod = (e.ctrlKey || e.metaKey) && !e.altKey;
    const k = e.key;
    if (peeking && (k === "Escape" || k === " ")) {
      e.preventDefault();
      setPeeking(false);
      return;
    }
    const step = (by: number, to?: number) => {
      if (rows.length === 0) return;
      const at = selected ? rows.findIndex((r) => r.drivewsid === selected) : -1;
      // Home and End go to an end whatever is selected; an arrow with nothing selected starts
      // at the end it points from. (Home with nothing selected used to land on the last row.)
      const index = to ?? (at < 0 ? (by > 0 ? 0 : rows.length - 1) : Math.min(rows.length - 1, Math.max(0, at + by)));
      const next = rows[index].drivewsid;
      if (e.shiftKey && selected) setPicked(run(next));
      else {
        anchor.current = next;
        setPicked([next]);
      }
      showRow(next);
    };
    /** How many tiles a line of the grid holds, from where they are laid out. */
    const across = () => {
      const tiles = [...(listRef.current?.querySelectorAll<HTMLElement>(":scope > li[data-id]") ?? [])];
      const top = tiles[0]?.offsetTop;
      return Math.max(1, tiles.filter((t) => t.offsetTop === top).length);
    };
    // Back and Forward: Ctrl-[ and Ctrl-] as Finder's, Alt and the arrows as the desktop's
    // file managers take them. By the key's place as well, for layouts with no bare bracket.
    const back = (mod && (k === "[" || e.code === "BracketLeft")) || (e.altKey && !e.ctrlKey && !e.metaKey && k === "ArrowLeft");
    const ahead = (mod && (k === "]" || e.code === "BracketRight")) || (e.altKey && !e.ctrlKey && !e.metaKey && k === "ArrowRight");
    if (back || ahead) {
      e.preventDefault();
      travel(back ? -1 : 1);
    } else if (mod && (k === "ArrowDown" || k === "o")) {
      if (!primary) return;
      e.preventDefault();
      openItem(primary);
    } else if (mod && k.toLowerCase() === "d" && !e.shiftKey) {
      // Duplicate, as Finder's Command-D: each file chosen, beside itself (folders are not).
      e.preventDefault();
      for (const item of pickedItems) if (!isContainer(item) && !busyIds.has(item.drivewsid)) void duplicate(item);
    } else if (mod && k.toLowerCase() === "c" && !e.shiftKey && pickedItems.length && !window.getSelection()?.toString()) {
      // Copy, as Finder's Command-C: what is chosen, to paste into any folder.
      e.preventDefault();
      void copyItems(pickedItems);
    } else if ((e.ctrlKey || e.metaKey) && !e.shiftKey && (k.toLowerCase() === "v" || (e.altKey && e.code === "KeyV"))) {
      // Paste, as Finder's Command-V; with Alt (Option), moved here instead, as Option-Command-V.
      // By the key's place too with Alt: Option-V on a Mac types "√".
      e.preventDefault();
      setPeeking(false);
      void paste(e.altKey);
    } else if (mod && k === "ArrowUp") {
      e.preventDefault();
      if (trail.length > 1) setTrail((t) => t.slice(0, -1));
    } else if (mod && k.toLowerCase() === "a") {
      e.preventDefault();
      setPicked(rows.map((r) => r.drivewsid));
    } else if (!mod && (k === "ArrowDown" || k === "ArrowUp")) {
      e.preventDefault();
      step((k === "ArrowDown" ? 1 : -1) * (grid ? across() : 1));
    } else if (!mod && grid && (k === "ArrowRight" || k === "ArrowLeft")) {
      e.preventDefault();
      step(k === "ArrowRight" ? 1 : -1);
    } else if (!mod && (k === "Home" || k === "End") && rows.length) {
      e.preventDefault();
      step(0, k === "Home" ? 0 : rows.length - 1);
    } else if (k === "Enter" && !mod && onRows && primary) {
      e.preventDefault();
      setPeeking(false);
      openItem(primary);
    } else if (k === "F2" && primary && pickedItems.length === 1 && !busyIds.has(primary.drivewsid)) {
      e.preventDefault();
      setPeeking(false);
      setRenaming(primary.drivewsid);
    } else if ((k === "Delete" || (mod && k === "Backspace")) && pickedItems.length) {
      e.preventDefault();
      setPeeking(false);
      void removeAll(pickedItems);
    } else if ((k === "ContextMenu" || (k === "F10" && e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey)) && primary) {
      // The menu key, or Shift-F10, as every desktop has them: the selection's own menu, under
      // its row — everything the right-click menu offers (Download among it), by the keys alone.
      e.preventDefault();
      setPeeking(false);
      const hit = listRef.current?.querySelector<HTMLElement>(`[data-id="${CSS.escape(primary.drivewsid)}"] .drive-row-hit`);
      const r = hit?.getBoundingClientRect();
      setMenu({ item: primary, x: r ? r.left + 24 : 24, y: r ? r.bottom : 24, alignRight: false });
    } else if (k === " " && !mod && onRows && primary) {
      e.preventDefault();
      setPeeking(true);
    }
  };
  // The mouse's own Back and Forward buttons, as every file manager takes them.
  const sideButton = useRef<(e: MouseEvent) => void>(() => {});
  sideButton.current = (e: MouseEvent) => {
    if (!live.current || inTrash || inRecents || (e.button !== 3 && e.button !== 4)) return;
    e.preventDefault();
    travel(e.button === 3 ? -1 : 1);
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => keys.current(e);
    const onMouse = (e: MouseEvent) => sideButton.current(e);
    window.addEventListener("keydown", onKey);
    window.addEventListener("mouseup", onMouse);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mouseup", onMouse);
    };
  }, []);
  // Nothing selected, nothing to look at.
  useEffect(() => {
    if (!primary) setPeeking(false);
  }, [primary]);
  // A folder opened starts with nothing selected — except what a search came to show.
  useEffect(() => {
    if (!revealing.current) setPicked([]);
    anchor.current = null;
    setPeeking(false);
  }, [here.id]);

  /**
   * Drive's own sidebar, as Finder's: iCloud Drive, Recents and Recently Deleted, the account
   * and how fresh Drive is at its foot — as every app's window has (docs/apps-as-windows.md).
   */
  const frame = (pane: React.ReactNode) => (
    <div className="dx">
      <aside className="dx-side" aria-label="Drive">
        <button className="source-row" aria-current={!inTrash && !inRecents} onClick={() => { setInTrash(false); setInRecents(false); }}>
          <Cloud className="dx-glyph" size={16} strokeWidth={1.8} aria-hidden="true" />
          <span className="source-name">iCloud Drive</span>
        </button>
        <button className="source-row" aria-current={inRecents} onClick={() => { setInTrash(false); setInRecents(true); }}>
          <Clock className="dx-glyph" size={16} strokeWidth={1.8} aria-hidden="true" />
          <span className="source-name">Recents</span>
        </button>
        <button className="source-row" aria-current={inTrash} onClick={() => { setInRecents(false); setInTrash(true); }}>
          <Trash2 className="dx-glyph" size={16} strokeWidth={1.8} aria-hidden="true" />
          <span className="source-name">Recently Deleted</span>
        </button>
        <AppFoot />
      </aside>
      <div className="dx-main">{pane}</div>
    </div>
  );
  if (inTrash) return frame(<DriveTrash onClose={() => setInTrash(false)} />);
  if (inRecents) return frame(<DriveRecents onClose={() => setInRecents(false)} />);

  /** A step of the path: the folder open is the title; one further up goes back to it, and takes
      a dragged item, as Finder's path bar does. */
  const crumbButton = (crumb: (typeof trail)[number], i: number) => (
    <button
      className={`crumb${dropOn === crumb.id ? " is-drop" : ""}`}
      aria-current={i === trail.length - 1}
      onClick={() => setTrail((t) => t.slice(0, i + 1))}
      // A folder further up the path takes a dragged item too, as Finder's path bar does.
      onDragOver={(e) => {
        if (!dragItem.current || i === trail.length - 1) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        if (dropOn !== crumb.id) setDropOn(crumb.id);
      }}
      onDragLeave={() => setDropOn((d) => (d === crumb.id ? null : d))}
      onDrop={(e) => {
        e.preventDefault();
        const moving = dragItem.current;
        dragItem.current = null;
        setDropOn(null);
        if (moving && i < trail.length - 1) void moveAll(moving, crumb.id);
      }}
    >
      {crumb.name}
    </button>
  );

  return frame(
    <div
      className={`drive${dropping ? " is-dropping" : ""}`}
      // A click on nothing — between or below the rows — selects nothing, as in Finder.
      onClick={(e) => {
        const t = e.target as HTMLElement;
        if (!t.closest("[data-id], button, input, form, a, [role='menu'], .modal-veil, .drive-peek-veil")) setPicked([]);
      }}
      // A right-click on nothing is for the folder itself, as Finder's: a new folder, and Paste
      // when the clipboard holds something to paste — files copied in the file manager, or what
      // was copied here.
      onContextMenu={(e) => {
        const t = e.target as HTMLElement;
        if (t.closest("[data-id], button, input, textarea, form, a, [role='menu'], .drive-bar, .modal-veil, .drive-peek-veil, .upload-tray")) return;
        e.preventDefault();
        setPicked([]);
        const at = { x: e.clientX, y: e.clientY, alignRight: false };
        void api.clipboardRead().then(
          (board) => setSpaceMenu({ ...at, files: board.files.length, other: board.text }),
          () => setSpaceMenu({ ...at, files: 0, other: null }),
        );
      }}
    >
      <header className="pane-head drive-bar">
        {/* The folder's name with what is in it under it, as icloud.com writes them: one block
            whose size never depends on when the count arrives, so the bar is laid out once. */}
        <PaneTitle
          className="drive-title"
          before={
            trail.length > 1 && (
              <nav className="crumbs" aria-label="Folder path">
                {trail.slice(0, -1).map((crumb, i) => (
                  <span key={crumb.id}>
                    {i > 0 && <span className="crumb-sep" aria-hidden="true">›</span>}
                    {crumbButton(crumb, i)}
                  </span>
                ))}
              </nav>
            )
          }
          count={
            // How much is in it; its line is kept while it loads, so nothing moves when it comes.
            <span className="drive-count" aria-live="polite">
              {loaded
                ? (() => {
                    const n = items.filter((x) => !gone.has(goneKey(here.id, x.drivewsid))).length;
                    return `${n} item${n === 1 ? "" : "s"}${available ? `, ${available} available` : ""}`;
                  })()
                : "\u00a0"}
            </span>
          }
        >
          {crumbButton(trail[trail.length - 1], trail.length - 1)}
        </PaneTitle>
        <div className="drive-tools">
          <span className="toolbar-group">
          <Picker
            pullDown
            className="drive-view"
            label="View as"
            value={grid ? "grid" : "list"}
            onChange={(v) => chooseView(v === "grid")}
            options={[{ value: "grid", label: "Grid" }, { value: "list", label: "List" }]}
          />
          <Picker
            pullDown
            className="drive-sort"
            label="Sort by"
            value={sortBy}
            onChange={(v) => chooseSort(v as SortKey)}
            options={SORTS.map((o) => ({ value: o.key, label: `by ${o.label}` }))}
          />
          </span>
          <span className="toolbar-group">
          <button
            className="tool has-label with-glyph"
            title="New Folder"
            onClick={() => setCreatingFolder(true)}
            disabled={creatingFolder}
          >
            <ActionGlyph name="newFolder" />
            New Folder
          </button>
          {/* What is selected, saved to Downloads — a file, a folder as its whole tree, or
              several — as icloud.com's toolbar offers it; nothing selected, nothing to save. */}
          <button
            className="tool has-label with-glyph"
            title={pickedItems.length ? "Download a copy of what is selected" : "Select something to download"}
            onClick={() => downloadItems(pickedItems)}
            disabled={pickedItems.length === 0}
          >
            <ActionGlyph name="download" />
            Download
          </button>
          </span>
          <span className="upload-menu-wrap">
            <button
              className="primary with-glyph"
              aria-haspopup="menu"
              aria-expanded={uploadMenu}
              onClick={() => setUploadMenu((open) => !open)}
            >
              <ActionGlyph name="upload" />
              Upload
            </button>
            {uploadMenu && (
              <span
                className="popover upload-menu"
                role="menu"
                onKeyDown={(e) => e.key === "Escape" && setUploadMenu(false)}
              >
                <button role="menuitem" autoFocus onClick={() => void chooseAndUpload(false)}>
                  Files
                </button>
                <button role="menuitem" onClick={() => void chooseAndUpload(true)}>
                  Folder
                </button>
              </span>
            )}
          </span>
        </div>
      </header>


      {!grid && (
        // Finder's and icloud.com's column headings: what each column is, and a click to
        // sort by it — the menu above and these are the same choice.
        <div className="drive-row drive-head" role="presentation">
          <span className="drive-row-hit">
            <span />
            {(["name", "kind", "size", "date"] as const).map((key) => (
              <button
                key={key}
                className={`drive-col${sortBy === key ? " is-on" : ""}${key === "name" ? " is-name" : ""}`}
                aria-pressed={sortBy === key}
                title={sortBy === key ? "Click to reverse the order" : `Sort by ${SORTS.find((o) => o.key === key)?.label}`}
                onClick={() => chooseSort(key, true)}
              >
                {SORTS.find((o) => o.key === key)?.label}
                {sortBy === key && <span className="drive-col-turn" aria-hidden="true">{reversed ? " ⌃" : " ⌄"}</span>}
              </button>
            ))}
          </span>
          <span className="drive-row-actions" aria-hidden="true" />
        </div>
      )}
      <ul
        ref={listRef}
        className={`drive-rows${grid ? " is-grid" : ""}${listFocused ? " is-focused" : ""}`}
        // Whether the focus is in the list, for the selection's colour (see the CSS on why
        // this is a class and not :focus-within).
        onFocus={() => setListFocused(true)}
        onBlur={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setListFocused(false);
        }}
        aria-label={here.name}
      >
        {pending
          // Shown until a row of its name is there — not one leaving (a Replace's): that one
          // goes at once, and the copy taking its place shows as being made.
          .filter((p) => p.folder === here.id && !items.some((i) => driveName(i) === p.name && !gone.has(goneKey(here.id, i.drivewsid))))
          .map((p) => (
            <li key={p.key} className="drive-row is-busy" aria-busy="true">
              <span className="drive-row-hit">
                <span className="drive-badge" aria-hidden="true">
                  <Spinner small />
                </span>
                <span className="drive-name">{p.name}</span>
                <span className="drive-meta drive-kind" />
                <span className="drive-meta">{p.doing}</span>
                <span className="drive-meta" />
              </span>
            </li>
          ))}
        {creatingFolder && (
          <li>
            <form
              className="drive-row drive-row-new"
              onSubmit={(e) => {
                e.preventDefault();
                void createFolder();
              }}
            >
              <span className="drive-badge" aria-hidden="true">
                <FileGlyph kind="FOLDER" extension={null} />
              </span>
              <input
                autoFocus
                value={newFolderName}
                onChange={(e) => setNewFolderName(e.target.value)}
                onBlur={() => void createFolder()}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    e.currentTarget.value = "";
                    setNewFolderName("");
                    setCreatingFolder(false);
                  }
                }}
                placeholder="Untitled Folder"
                aria-label="New folder name"
              />
            </form>
          </li>
        )}

        <DriveRows
          rows={rows}
          chosen={chosen}
          busyIds={busyIds}
          menuOn={menu?.item.drivewsid ?? null}
          dropOn={dropOn}
          thumbs={thumbs}
          renaming={renaming}
          actions={rowActions}
        />
      </ul>

      {items.length === 0 && !creatingFolder && !error && (loading || !loaded) && (
        <EmptyState busy>Opening {here.name}…</EmptyState>
      )}
      {loaded && !loading && items.length === 0 && !creatingFolder && !error && (
        <EmptyState>This folder is empty.</EmptyState>
      )}

      {moving && (
        <MoveDialog
          items={moving}
          startingIn={here.id}
          onCancel={() => setMoving(null)}
          onMove={(destinationId) => void moveAll(moving, destinationId)}
        />
      )}

      {confirmSheet}
      {asking && (
        <ConfirmSheet<"keep" | "replace">
          title={asking.items.length === 1
            ? `\u201c${driveName(asking.items[0])}\u201d is already in this folder`
            : `${asking.items.length} items with these names are already in this folder`}
          choices={[
            { label: "Keep Both", value: "keep", primary: true, focus: true },
            { label: "Replace", value: "replace", danger: true },
          ]}
          onAnswer={(how) => asking.answer(how)}
          onCancel={() => asking.answer(null)}
        />
      )}

      {menu && (() => {
        // A right-click on the selection is for all of it, as Finder's menu is.
        const many = pickedItems.length > 1 && picked.includes(menu.item.drivewsid) ? pickedItems : null;
        return (
          <RowMenu
            at={menu}
            onClose={() => setMenu(null)}
            entries={many ? [
              // Files and folders alike, as one download into Downloads.
              { label: `Download ${many.length} Copies`, run: () => downloadItems(many) },
              ...(many.some(copyable) ? [{ label: "Copy", run: () => void copyItems(many) }] : []),
              { label: "Move to Folder", run: () => setMoving(many) },
              null,
              { label: `Delete ${many.length} Items`, danger: true, run: () => void removeAll(many) },
            ] : [
              // Keep on This Computer, as Finder's Keep Downloaded: only where the iCloud Drive
              // folder is up, which is where a kept file lives.
              ...(kept
                ? [
                    {
                      label: "Keep on This Computer",
                      checked: kept.has(menu.item.drivewsid),
                      run: () => void keep(menu.item, !kept.has(menu.item.drivewsid)),
                    },
                    null,
                  ]
                : []),
              { label: isContainer(menu.item) ? "Open" : "Download a Copy", run: () => openItem(menu.item) },
              // A folder, whole, into Downloads.
              ...(isContainer(menu.item) ? [{ label: "Download a Copy", run: () => downloadItems([menu.item]) }] : []),
              { label: "Get Info", run: () => setInfo(menu.item) },
              null,
              ...(isContainer(menu.item) ? [] : [{ label: "Duplicate", run: () => void duplicate(menu.item) }]),
              ...(copyable(menu.item) ? [{ label: "Copy", run: () => void copyItems([menu.item]) }] : []),
              { label: "Rename", run: () => setRenaming(menu.item.drivewsid) },
              { label: "Move to Folder", run: () => setMoving([menu.item]) },
              null,
              { label: "Delete", danger: true, run: () => void remove(menu.item) },
            ]}
          />
        );
      })()}

      {spaceMenu && (() => {
        // What Paste would put here: the file manager's files, or what was copied in Drive while
        // nothing else has been copied since (see paste).
        const theirs = spaceMenu.files > 0;
        const ours = !!clip && (clip.text === null || spaceMenu.other === null || spaceMenu.other === clip.text);
        return (
          <RowMenu
            at={spaceMenu}
            onClose={() => setSpaceMenu(null)}
            entries={[
              { label: "New Folder", run: () => setCreatingFolder(true) },
              ...(theirs || ours ? [null, { label: "Paste", run: () => void paste(false) }] : []),
            ]}
          />
        );
      })()}

      {peeking && primary && (
        <QuickLook
          item={primary}
          thumb={thumbs[primary.drivewsid]}
          busy={busyIds.has(primary.drivewsid)}
          onOpen={() => { setPeeking(false); openItem(primary); }}
          onClose={() => setPeeking(false)}
        />
      )}

      {info && (
        <InfoSheet item={info} where={trail.map((c) => c.name).join(" › ")} thumb={thumbs[info.drivewsid]} onClose={() => setInfo(null)} />
      )}

      <div className="trays">
        <DownloadTray />
        <UploadTray uploads={[...uploads.values()]} onDismiss={dismissUploads} />
      </div>
    </div>
  );
}

type MenuAt = { x: number; y: number; alignRight: boolean };
type DriveRowActions = {
  click: (item: DriveItem, e: React.MouseEvent) => void;
  open: (item: DriveItem) => void;
  menu: (item: DriveItem, at: MenuAt) => void;
  dragStart: (item: DriveItem, e: React.DragEvent) => void;
  dragEnd: () => void;
  dragOver: (item: DriveItem, e: React.DragEvent) => void;
  dragLeave: (item: DriveItem) => void;
  drop: (item: DriveItem, e: React.DragEvent) => void;
  rename: (item: DriveItem, name: string) => void;
  cancelRename: () => void;
};

/** What a folder holds, or a file weighs, in the Size column. */
const sizeText = (item: DriveItem) =>
  isContainer(item)
    ? item.childCount === null ? "" : `${item.childCount} item${item.childCount === 1 ? "" : "s"}`
    : fileSize(item.size);

/**
 * The folder's rows. Drawn again only when what they show changes — not when the selection
 * moves: that reaches each row through `chosen` (lib/chosen), so an arrow press in a folder of
 * thousands draws the two rows it moved between, without making every row anew to find them.
 */
const DriveRows = memo(function DriveRows({
  rows, chosen, busyIds, menuOn, dropOn, thumbs, renaming, actions,
}: {
  rows: DriveItem[];
  chosen: Chosen;
  busyIds: ReadonlySet<string>;
  /** The row whose menu is open. */
  menuOn: string | null;
  dropOn: string | null;
  thumbs: Record<string, string>;
  renaming: string | null;
  actions: DriveRowActions;
}) {
  return (
    <>
      {rows.map((item) => (
        <DriveRow
          key={item.drivewsid}
          item={item}
          busy={busyIds.has(item.drivewsid)}
          chosen={chosen}
          menuOpen={menuOn === item.drivewsid}
          dropTarget={dropOn === item.drivewsid}
          thumb={thumbs[item.drivewsid]}
          renaming={renaming === item.drivewsid}
          actions={actions}
        />
      ))}
    </>
  );
});

/**
 * One file or folder. Memoised on what it shows, so in a folder of thousands a click, a key or
 * a menu opening redraws the rows it changes and no others.
 */
const DriveRow = memo(function DriveRow({
  item, busy, chosen, menuOpen, dropTarget, thumb, renaming, actions,
}: {
  item: DriveItem;
  busy: boolean;
  chosen: Chosen;
  menuOpen: boolean;
  dropTarget: boolean;
  thumb?: string;
  renaming: boolean;
  actions: DriveRowActions;
}) {
  const selected = useIsChosen(chosen, item.drivewsid);
  const container = isContainer(item);
  if (renaming) {
    return (
      <li data-id={item.drivewsid}>
        <div className="drive-row">
          <span className="drive-badge" aria-hidden="true">
            <FileGlyph kind={item.kind} extension={item.extension} />
          </span>
          <input
            className="drive-rename-input"
            autoFocus
            defaultValue={driveName(item)}
            aria-label={`Rename ${driveName(item)}`}
            // The name without its extension is what is selected, as Finder selects it, so
            // typing keeps ".pdf".
            onFocus={(e) => {
              const field = e.currentTarget;
              const end = item.extension && field.value.endsWith(`.${item.extension}`)
                ? field.value.length - item.extension.length - 1
                : field.value.length;
              field.setSelectionRange(0, end);
            }}
            onBlur={(e) => actions.rename(item, e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              if (e.key === "Escape") {
                e.currentTarget.value = driveName(item);
                actions.cancelRename();
              }
            }}
          />
        </div>
      </li>
    );
  }
  return (
    <li data-id={item.drivewsid}>
      <div
        className={`drive-row${container ? " is-folder" : ""}${busy ? " is-busy" : ""}${menuOpen ? " is-menu" : ""}${selected ? " is-selected" : ""}${dropTarget ? " is-drop" : ""}`}
        // Dragged onto a folder, an item moves into it — as in Finder and icloud.com.
        draggable={!busy && item.kind !== "APP_LIBRARY"}
        onDragStart={(e) => actions.dragStart(item, e)}
        onDragEnd={actions.dragEnd}
        onDragOver={(e) => actions.dragOver(item, e)}
        onDragLeave={() => actions.dragLeave(item)}
        onDrop={(e) => actions.drop(item, e)}
        onContextMenu={(e) => {
          e.preventDefault();
          if (!busy) actions.menu(item, { x: e.clientX, y: e.clientY, alignRight: false });
        }}
      >
        <button
          className="drive-row-hit"
          // As Finder and icloud.com: a click selects, a double-click opens — a folder goes
          // in, a file is saved to Downloads and shown there. Return opens the selected item
          // too; a single click that opened made every misclick a download.
          aria-pressed={selected}
          onClick={(e) => actions.click(item, e)}
          onDoubleClick={() => actions.open(item)}
          disabled={busy}
        >
          <span className="drive-badge" aria-hidden="true">
            {busy ? (
              <Spinner small />
            ) : thumb ? (
              <img className="drive-thumb" src={thumb} alt="" draggable={false} />
            ) : (
              <FileGlyph kind={item.kind} extension={item.extension} />
            )}
          </span>
          <span className="drive-name">{driveName(item)}</span>
          <span className="drive-meta drive-kind">{kindName(item)}</span>
          <span className="drive-meta">{sizeText(item)}</span>
          <span className="drive-meta">{modified(item.modifiedAt ?? item.createdAt)}</span>
        </button>
        <span className="drive-row-actions">
          <button
            className="icon-button small drive-action"
            aria-label={`Actions for ${driveName(item)}`}
            aria-haspopup="menu"
            title="More"
            disabled={busy}
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              actions.menu(item, { x: r.right, y: r.bottom + 4, alignRight: true });
            }}
          >
            <span aria-hidden="true" className="drive-more">•••</span>
          </button>
        </span>
      </div>
    </li>
  );
});

/**
 * Quick Look, as near as iCloud lets it be without fetching the file: Apple's own picture of
 * it, large, with its name, kind, size and date. Space or Escape closes it; the arrows go on
 * walking the folder behind it and the look follows. Opening it fully is one press away.
 */
function QuickLook({ item, thumb, busy, onOpen, onClose }: {
  item: DriveItem;
  thumb?: string;
  busy: boolean;
  onOpen: () => void;
  onClose: () => void;
}) {
  const container = isContainer(item);
  const sheet = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheet);
  const facts = [kindName(item), sizeText(item)].filter(Boolean).join(", ");
  const when = modified(item.modifiedAt ?? item.createdAt);
  return (
    <Portal>
      <div className="modal-veil drive-peek-veil" onClick={onClose}>
        <div ref={sheet} className="modal-sheet drive-peek" role="dialog" aria-modal="true" aria-label={`Quick Look: ${driveName(item)}`} onClick={(e) => e.stopPropagation()}>
          <div className={`drive-peek-face${thumb ? " has-thumb" : ""}`} aria-hidden="true">
            {thumb ? <img src={thumb} alt="" draggable={false} /> : <FileGlyph kind={item.kind} extension={item.extension} />}
          </div>
          <p className="drive-peek-name">{driveName(item)}</p>
          <p className="drive-peek-facts">{facts}</p>
          {when && <p className="drive-peek-facts">Modified {when}</p>}
          <div className="drive-peek-actions">
            <button className="primary" disabled={busy} onClick={onOpen}>
              {container ? "Open" : busy ? "Downloading…" : "Download a Copy"}
            </button>
            <button className="quiet" onClick={onClose}>Close</button>
          </div>
        </div>
      </div>
    </Portal>
  );
}

/**
 * Where to move something — a small folder browser of its own, in the same shape Files'
 * own Move sheet takes: breadcrumbs, folders only, a Move button that names where it will
 * land. You cannot navigate into the item being moved, which is what stops the one obvious
 * way to move something inside itself.
 */
function MoveDialog({
  items,
  startingIn,
  onCancel,
  onMove,
}: {
  items: DriveItem[];
  startingIn: string;
  onCancel: () => void;
  onMove: (destinationId: string) => void;
}) {
  const [trail, setTrail] = useState<Crumb[]>([{ id: DRIVE_ROOT, name: "Drive" }]);
  const [folders, setFolders] = useState<DriveItem[]>([]);
  const [loading, setLoading] = useState(false);

  const here = trail[trail.length - 1];
  /** What is being moved: none of it can be gone into, which is what stops a folder going inside itself. */
  const moving = useMemo(() => new Set(items.map((i) => i.drivewsid)), [items]);
  const what = items.length === 1 ? `\u201c${driveName(items[0])}\u201d` : `${items.length} Items`;

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    (async () => {
      try {
        await api.driveSyncFolder(here.id);
        const fresh = await api.driveChildren(here.id);
        if (!cancelled) {
          setFolders(
            (Array.isArray(fresh) ? fresh : []).filter(
              (i) => isContainer(i) && !moving.has(i.drivewsid)
            )
          );
        }
      } catch {
        if (!cancelled) setFolders([]);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [here.id, moving]);

  const alreadyHere = here.id === startingIn;

  // A sheet is modal: Escape leaves it exactly as Cancel would, and it must not be possible
  // to tab out into the page still open underneath.
  const live = useActiveRef();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!live.current) return;
      if (e.key === "Escape") onCancel();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onCancel]);

  // JSX autoFocus loses a real race: the click that opened this dialog also gives the
  // trigger button native focus, and that lands after React's own autoFocus handling —
  // so a keyboard user who opened this with Enter stayed on the row behind it, in a dialog
  // that was visually in front but not where their next Tab or Escape would act. Moving
  // focus explicitly, one tick later, wins the race properly.
  const cancelRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    const id = window.setTimeout(() => cancelRef.current?.focus(), 0);
    return () => window.clearTimeout(id);
  }, []);
  const sheet = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheet);

  return (
    <Portal>
    <div className="modal-veil" onClick={onCancel}>
      <div
        ref={sheet}
        className="modal-sheet move-sheet"
        role="dialog"
        aria-modal="true"
        aria-label={items.length === 1 ? `Move ${driveName(items[0])}` : `Move ${items.length} Items`}
        onClick={(e) => e.stopPropagation()}
      >
        <h2>Move {what}</h2>

        <nav className="crumbs" aria-label="Destination folder">
          {trail.map((crumb, i) => (
            <span key={crumb.id}>
              {i > 0 && (
                <span className="crumb-sep" aria-hidden="true">
                  ›
                </span>
              )}
              <button
                className="crumb"
                aria-current={i === trail.length - 1}
                onClick={() => setTrail((t) => t.slice(0, i + 1))}
              >
                {crumb.name}
              </button>
            </span>
          ))}
        </nav>

        <ul className="move-sheet-list">
          {loading && <li className="move-sheet-none">Loading…</li>}
          {!loading && folders.length === 0 && (
            <li className="move-sheet-none">No folders here.</li>
          )}
          {!loading &&
            folders.map((folder) => (
              <li key={folder.drivewsid}>
                <button
                  className="drive-row-hit move-sheet-row"
                  onClick={() =>
                    setTrail((t) => [...t, { id: folder.drivewsid, name: driveName(folder) }])
                  }
                >
                  <span className="drive-badge" aria-hidden="true">
                    <FileGlyph kind="FOLDER" extension={null} />
                  </span>
                  <span className="drive-name">{driveName(folder)}</span>
                </button>
              </li>
            ))}
        </ul>

        <div className="cal-form-actions">
          <button
            className="primary"
            disabled={alreadyHere}
            onClick={() => onMove(here.id)}
          >
            Move here
          </button>
          <button className="quiet" ref={cancelRef} onClick={onCancel}>
            Cancel
          </button>
        </div>
      </div>
    </div>
    </Portal>
  );
}
