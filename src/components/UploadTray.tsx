import { Progress } from "./Spinner";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { api, type UploadEvent } from "../lib/ipc";
import { FileGlyph } from "./FileGlyph";

const extensionOf = (name: string) => {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1) : null;
};

const ended = (u: UploadEvent) => u.state === "done" || u.state === "failed" || u.state === "cancelled";

/**
 * Every file sent from a screen, until its tray is closed, held outside the screen's state.
 *
 * Kept as the screen's state, each upload event copied the whole map and drew Drive again:
 * a thousand files going up into a folder of thousands sent dozens of events a second, each
 * redrawing the file list for a tray in the corner. Here an event changes one entry in place,
 * and the tray alone is told, at most every 80 ms, as the download tray is (lib/downloads).
 */
export class Uploads {
  private byId = new Map<string, UploadEvent>();
  private shown: UploadEvent[] = [];
  private listeners = new Set<() => void>();
  private flushing: number | null = null;

  /** An event from the backend. Events for one file can pass each other; a finished file never goes back. */
  hear(u: UploadEvent) {
    const before = this.byId.get(u.id);
    if (before && ended(before)) return;
    this.byId.set(u.id, u);
    this.changed();
  }

  /** Change the entries `edit` returns a new value for; told only if one changed. */
  edit(edit: (u: UploadEvent) => UploadEvent | null) {
    let any = false;
    for (const [id, u] of this.byId) {
      const next = edit(u);
      if (next) {
        this.byId.set(id, next);
        any = true;
      }
    }
    if (any) this.changed();
  }

  clear() {
    if (this.byId.size === 0) return;
    this.byId.clear();
    // At once: Done closes the tray in the same frame it is pressed.
    this.flush();
  }

  values(): IterableIterator<UploadEvent> {
    return this.byId.values();
  }

  /** The uploads as last told, the same array until the next change is told. */
  get = (): UploadEvent[] => this.shown;

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private changed() {
    if (this.flushing !== null) return;
    this.flushing = window.setTimeout(() => this.flush(), 80);
  }

  private flush() {
    if (this.flushing !== null) window.clearTimeout(this.flushing);
    this.flushing = null;
    this.shown = [...this.byId.values()];
    this.listeners.forEach((l) => l());
  }
}

/**
 * One line of the tray: a file dropped on its own, or a whole folder with everything in it
 * counted as one — as Google Drive and Finder show a folder going up. A folder of 214 files
 * listed file by file was a scroll of names nobody could follow.
 */
type Item = {
  key: string;
  name: string;
  folder: boolean;
  files: UploadEvent[];
  /** The folder's own row: being made on Apple, made, or refused. */
  self?: UploadEvent;
};

function group(uploads: UploadEvent[]): Item[] {
  const items: Item[] = [];
  const byFolder = new Map<string, Item>();
  for (const u of uploads) {
    if (u.state === "planned") continue;
    const top = u.within ? u.within.split("/")[0] : null;
    if (u.folder && !u.within) {
      const item: Item = { key: `${u.batch}/${u.name}`, name: u.name, folder: true, files: [], self: u };
      byFolder.set(item.key, item);
      items.push(item);
    } else if (top) {
      const key = `${u.batch}/${top}`;
      let item = byFolder.get(key);
      if (!item) {
        item = { key, name: top, folder: true, files: [] };
        byFolder.set(key, item);
        items.push(item);
      }
      if (!u.folder) item.files.push(u);
    } else if (!u.folder) {
      items.push({ key: u.id, name: u.name, folder: false, files: [u] });
    }
  }
  return items;
}

/** How far a line has got, and what it is doing, in the fewest words that are true. */
function measure(item: Item, planned: boolean) {
  const total = item.files.reduce((n, u) => n + u.total, 0);
  const sent = item.files.reduce((n, u) => n + (u.state === "done" ? u.total : u.sent), 0);
  const done = item.files.filter((u) => u.state === "done").length;
  const failed = item.files.filter((u) => u.state === "failed");
  const cancelled = item.files.filter((u) => u.state === "cancelled").length;
  const running = item.files.some((u) => !ended(u)) || item.self?.state === "preparing" || (item.folder && !planned);
  const percent = total > 0 ? Math.floor((sent / total) * 100) : running ? 0 : 100;
  let status: string;
  if (item.self?.state === "failed") status = "Couldn’t be made";
  // Still finding what is in it: a count that would only grow is not a total yet.
  else if (item.folder && !planned) status = item.files.length ? `Preparing… ${item.files.length} found` : "Preparing…";
  else if (running && item.files.length === 0) status = "Preparing…";
  else if (running && !item.folder) {
    const u = item.files[0];
    status = u.state === "waiting" ? "Waiting" : u.state === "verifying" ? "Checking" : `${percent}%`;
  } else if (running) status = `${done} of ${item.files.length}`;
  else if (failed.length > 0) status = item.folder ? `${failed.length} didn’t upload` : "Didn’t upload";
  else if (cancelled > 0 && done === 0) status = "Cancelled";
  else if (cancelled > 0) status = `${done} uploaded, ${cancelled} cancelled`;
  else status = "Uploaded";
  return { total, sent, percent, running, failed, status };
}

/**
 * Every upload going up, in the corner as a download list sits: one line per file dropped and
 * one per folder, each with its own progress, the whole batch summed in the head. A folder
 * opens to show what in it did not upload, and why. When everything has gone up cleanly the
 * tray steps aside on its own; anything that failed keeps it until it is read.
 */
export function UploadTray({
  store,
  onDismiss,
  cancel = api.driveUploadCancel,
}: {
  store: Uploads;
  onDismiss: () => void;
  /** Stops uploads by id, or a whole batch; Drive's unless the screen says otherwise. */
  cancel?: (ids: string[], batch?: string) => Promise<void>;
}) {
  const uploads = useSyncExternalStore(store.subscribe, store.get);
  const [open, setOpen] = useState<string | null>(null);
  const items = group(uploads);
  const planned = new Set(uploads.filter((u) => u.state === "planned").map((u) => u.batch));
  const measured = items.map((item) => ({ item, ...measure(item, planned.has(item.files[0]?.batch ?? item.self?.batch ?? "")) }));
  const active = measured.some((m) => m.running);
  const anyFailed = measured.some((m) => m.failed.length > 0 || m.item.self?.state === "failed");
  const allPlanned = [...new Set(uploads.map((u) => u.batch))].every((b) => planned.has(b));
  const total = measured.reduce((n, m) => n + m.total, 0);
  const sent = measured.reduce((n, m) => n + m.sent, 0);
  // Progress only ever moves forward: three files go up at once and report out of step, and
  // a bar that slid back on every report read as broken.
  const highest = useRef<Map<string, number>>(new Map());
  if (uploads.length === 0) highest.current.clear();
  const forward = (key: string, value: number) => {
    const was = highest.current.get(key) ?? 0;
    const now = Math.max(was, value);
    highest.current.set(key, now);
    return now;
  };
  const percent = allPlanned && total > 0 ? forward("all", Math.floor((sent / total) * 100)) : 0;
  const files = uploads.filter((u) => !u.folder);
  const finishedFiles = files.filter(ended).length;

  // Gone by itself a moment after a clean finish; held while anything needs reading.
  useEffect(() => {
    if (uploads.length === 0 || active || anyFailed) return undefined;
    const t = window.setTimeout(onDismiss, 4000);
    return () => window.clearTimeout(t);
  }, [uploads.length, active, anyFailed, onDismiss]);

  // Away from the window, WebKit stops drawing while the upload goes on; coming back, the
  // bars jump to where the upload has got to instead of replaying every step at once.
  const [instant, setInstant] = useState(false);
  useEffect(() => {
    const back = () => {
      if (document.visibilityState !== "visible") return;
      setInstant(true);
      requestAnimationFrame(() => requestAnimationFrame(() => setInstant(false)));
    };
    document.addEventListener("visibilitychange", back);
    window.addEventListener("focus", back);
    return () => {
      document.removeEventListener("visibilitychange", back);
      window.removeEventListener("focus", back);
    };
  }, []);

  if (uploads.length === 0) return null;
  const batches = [...new Set(uploads.filter((u) => !ended(u)).map((u) => u.batch))];
  const heading = active
    ? items.length === 1 && items[0].folder
      ? `Uploading ${items[0].name}`
      : `Uploading ${Math.min(finishedFiles + 1, Math.max(files.length, 1))} of ${Math.max(files.length, 1)}`
    : anyFailed
      ? "Some uploads didn’t finish"
      : items.length === 1
        ? `${items[0].name} uploaded`
        : `${files.length} uploaded`;


  return (
    <section className={`upload-tray${instant ? " is-instant" : ""}`} aria-label="Uploads" aria-live="polite">
      <header className="upload-tray-head">
        <span className="upload-tray-title">{heading}</span>
        {active ? (
          <span className="upload-tray-actions">
            {allPlanned && <span className="upload-tray-percent">{percent}%</span>}
            <button className="quiet upload-tray-close" onClick={() => batches.forEach((b) => void cancel([], b))}>
              Cancel
            </button>
          </span>
        ) : (
          <button className="quiet upload-tray-close" onClick={onDismiss} aria-label="Close uploads">
            Done
          </button>
        )}
      </header>
      {active && (
        <Progress value={allPlanned ? percent / 100 : null} instant={instant} className="upload-bar" />
      )}
      <ul className="upload-rows">
        {measured.map(({ item, percent: raw, running, failed, status }) => {
          const p = forward(item.key, raw);
          const stoppable = item.files.filter((u) => u.state === "waiting" || u.state === "sending");
          const state = running ? "sending" : failed.length || item.self?.state === "failed" ? "failed" : status === "Cancelled" ? "cancelled" : "done";
          return (
            <li key={item.key} className={`upload-row is-${state}`}>
              <div className="upload-line">
                <span className="upload-glyph" aria-hidden="true">
                  <FileGlyph kind={item.folder ? "FOLDER" : "FILE"} extension={item.folder ? null : extensionOf(item.name)} />
                </span>
                <span className="upload-text">
                  <span className="upload-name">{item.name}</span>
                  {running && (item.folder ? planned.has(item.files[0]?.batch ?? "") : true) && (
                    <Progress value={p / 100} instant={instant} className="upload-bar small" />
                  )}
                  {!running && !item.folder && failed[0]?.error && <span className="upload-where">{failed[0].error}</span>}
                </span>
                <span className="upload-state">
                  {state === "done" && status === "Uploaded" ? <span aria-label="Uploaded">✓</span> : status}
                </span>
                {item.folder && failed.length > 0 && (
                  <button
                    className="icon-button small round upload-stop"
                    aria-expanded={open === item.key}
                    aria-label={open === item.key ? "Hide what didn’t upload" : "Show what didn’t upload"}
                    onClick={() => setOpen(open === item.key ? null : item.key)}
                  >
                    {open === item.key ? "▾" : "▸"}
                  </button>
                )}
                {/* One upload has one Cancel — the head's. */}
                {stoppable.length > 0 && measured.length > 1 && (
                  <button
                    className="icon-button small round upload-stop"
                    aria-label={`Cancel ${item.name}`}
                    title="Cancel"
                    onClick={() => void cancel(stoppable.map((u) => u.id))}
                  >
                    ✕
                  </button>
                )}
              </div>
              {open === item.key && (
                <ul className="upload-failures">
                  {failed.map((u) => (
                    <li key={u.id}>
                      <b>{u.within.split("/").slice(1).concat(u.name).join("/")}</b> {u.error}
                    </li>
                  ))}
                </ul>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
