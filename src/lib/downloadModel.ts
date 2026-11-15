/**
 * A download into Downloads — a folder, or a selection of files and folders — as the tray
 * shows it: one line for the whole of it, however many files are in it.
 *
 * Kept as counts, not as a row per file: a folder of a hundred thousand files sends about as
 * many events, and each is a few additions here. Only the large files arriving right now are
 * held one by one (for their bytes), and only the first NAMED failures by name.
 */
import type { DownloadEvent, DownloadReport, DriveItem } from "./ipc";
import { t } from "./i18n.ts";

/** How many failures a line names; past this they are counted. */
export const NAMED = 200;

export type Failure = { path: string; error: string };

export type DownloadLine = {
  batch: string;
  /** What was chosen: a folder's name, or "3 items". */
  title: string;
  /** What was chosen, to download again: that resumes it. */
  items: DriveItem[];
  /** Counted while the tree is read: files and their bytes. */
  found: number;
  done: number;
  failedCount: number;
  failures: Failure[];
  bytesDone: number;
  /** The large files arriving now, and how many of their bytes are here. */
  inflight: Map<string, number>;
  /** Large files that have finished: a late report for one is not undone. */
  ended: Set<string>;
  /** Every file in it is known: how many, and their bytes. */
  planned: { files: number; bytes: number } | null;
  finished: boolean;
  report: DownloadReport | null;
  error: string | null;
  /** An unfinished download from before Skykeep was last closed: what of it is in Downloads. */
  paused: { key: string; saved: string[] } | null;
};

export function newLine(batch: string, title: string, items: DriveItem[]): DownloadLine {
  return {
    batch,
    title,
    items,
    found: 0,
    done: 0,
    failedCount: 0,
    failures: [],
    bytesDone: 0,
    inflight: new Map(),
    ended: new Set(),
    planned: null,
    finished: false,
    report: null,
    error: null,
    paused: null,
  };
}

/** An unfinished download from before, shown so it can be gone on with. */
export function pausedLine(key: string, title: string, items: DriveItem[], saved: string[]): DownloadLine {
  return { ...newLine(`unfinished:${key}`, title, items), finished: true, paused: { key, saved } };
}

function fail(line: DownloadLine, e: DownloadEvent) {
  line.failedCount += 1;
  if (line.failures.length < NAMED) {
    line.failures.push({ path: [e.within, e.name].filter(Boolean).join("/"), error: e.error ?? "" });
  }
}

/** One event from the backend, counted into its line. */
export function apply(line: DownloadLine, e: DownloadEvent) {
  if (line.finished) return;
  if (e.state === "found") {
    line.found = e.sent;
    return;
  }
  if (e.state === "planned") {
    line.planned = { files: e.sent, bytes: e.total };
    return;
  }
  // A folder's only event is that it could not be read or made.
  if (e.folder) {
    if (e.state === "failed") fail(line, e);
    return;
  }
  // Events for one file can pass each other; a finished file never goes back.
  if (line.ended.has(e.id)) return;
  const end = () => {
    if (line.inflight.delete(e.id)) line.ended.add(e.id);
  };
  switch (e.state) {
    case "downloading":
      line.inflight.set(e.id, Math.max(line.inflight.get(e.id) ?? 0, e.sent));
      break;
    case "done":
      end();
      line.done += 1;
      line.bytesDone += Math.max(e.sent, 0);
      break;
    case "failed":
      end();
      fail(line, e);
      break;
    case "cancelled":
      end();
      break;
    default:
      break;
  }
}

/** The backend has said all it will: whatever was still arriving is closed now. */
export function finish(line: DownloadLine, report: DownloadReport | null, error: string | null) {
  line.finished = true;
  line.report = report;
  line.error = error;
  line.inflight.clear();
  if (report && report.moreFailed > 0) line.failedCount = Math.max(line.failedCount, report.failed.length + report.moreFailed);
}

export type Tone = "sending" | "done" | "failed" | "cancelled";

export type LineView = {
  batch: string;
  title: string;
  running: boolean;
  /** How far, 0–100; null while it is not known how much there is. */
  percent: number | null;
  status: string;
  /** Stopped to be resumed later: told apart by this, never by the words said for it. */
  paused: boolean;
  tone: Tone;
  /** A sentence under the name: why it stopped, when it did. */
  note: string | null;
  failures: Failure[];
  /** Failures not named. */
  unnamed: number;
  /** What Show in Folder shows. */
  reveal: string | null;
  /** What going on with it is called: Resume where it was, or Try Again afresh. */
  again: "Resume" | "Try Again" | null;
  /** It is remembered as unfinished, and can be let go of. */
  forgettable: string | null;
};

const arrived = (line: DownloadLine) => line.bytesDone + [...line.inflight.values()].reduce((n, b) => n + b, 0);

/** How far a line has got, and what it is doing, in the fewest words that are true. */
export function view(line: DownloadLine): LineView {
  const running = !line.finished;
  let percent: number | null = null;
  if (line.planned) {
    const { bytes, files } = line.planned;
    if (bytes > 0) percent = Math.min(100, Math.floor((arrived(line) / bytes) * 100));
    else if (files > 0) percent = Math.min(100, Math.floor((line.done / files) * 100));
    else percent = running ? 0 : 100;
  }
  const failures = line.failures;
  const unnamed = Math.max(0, line.failedCount - failures.length);
  const saved = line.report?.saved ?? line.paused?.saved ?? [];
  const reveal = !running && saved.length > 0 ? saved[0] : null;
  const resumable = Boolean(line.report?.resumable || line.paused);
  const base = {
    batch: line.batch,
    title: line.title,
    running,
    failures,
    unnamed,
    reveal,
    forgettable: resumable ? (line.report?.key ?? line.paused?.key ?? null) : null,
  };
  if (running) {
    // Still counting what is in it: a count that would only grow is not a total yet.
    const status = line.planned
      ? t("downloads.doneOf", { done: line.done, total: line.planned.files })
      : line.found
        ? t("downloads.preparingFound", { count: line.found })
        : t("downloads.preparing");
    return { ...base, percent, status, paused: false, tone: "sending", note: null, again: null };
  }
  const again = resumable ? "Resume" : "Try Again";
  if (line.paused) return { ...base, percent: null, status: t("downloads.paused"), paused: true, tone: "cancelled", note: null, again };
  if (line.error) return { ...base, percent: null, status: t("downloads.didntDownload"), paused: false, tone: "failed", note: line.error, again };
  const problem = line.report?.problem ?? null;
  if (problem) return { ...base, percent: null, status: line.done ? t("downloads.someDownloaded", { count: line.done }) : t("downloads.stopped"), paused: false, tone: "failed", note: problem, again };
  if (line.report?.cancelled) {
    return { ...base, percent: null, status: line.done ? t("downloads.cancelledAfter", { count: line.done }) : t("downloads.cancelled"), paused: false, tone: "cancelled", note: null, again };
  }
  if (line.failedCount > 0) {
    return { ...base, percent: null, status: t("downloads.someDidntDownload", { count: line.failedCount }), paused: false, tone: "failed", note: null, again };
  }
  return { ...base, percent: 100, status: t("downloads.downloaded"), paused: false, tone: "done", note: null, again: null };
}

/** The tray's head: what is happening, across every line. */
export function heading(views: LineView[]): string {
  const running = views.filter((v) => v.running);
  if (running.length === 1) return t("downloads.downloadingName", { name: running[0].title });
  if (running.length > 1) return t("downloads.downloading");
  if (views.length > 0 && views.every((v) => v.paused)) {
    return views.length === 1 ? t("downloads.oneUnfinished") : t("downloads.someUnfinished");
  }
  if (views.some((v) => v.tone === "failed")) return t("downloads.someUnfinished");
  if (views.length > 0 && views.every((v) => v.tone === "cancelled")) return views.length === 1 ? t("downloads.oneCancelled") : t("downloads.manyCancelled");
  return views.length === 1 ? t("downloads.nameDownloaded", { name: views[0].title }) : t("downloads.finished");
}
