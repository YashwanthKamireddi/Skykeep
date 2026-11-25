import { dateLocale } from "../lib/when";
import { MOVE_CHECKED_KEY } from "./notes/model";
import { onWindows } from "../lib/platform";
import { ConfirmSheet } from "../components/Confirm";
import { toast, toastSaved, useErrorToast } from "../components/Toast";
import { Progress } from "../components/Spinner";
import { lazy, Suspense, useEffect, useRef, useState, useSyncExternalStore, type Dispatch, type ReactNode, type SetStateAction } from "react";
import { Archive, ArrowDownCircle, Bell, House, Laptop, Settings as Gear, ShieldCheck, Smartphone, type LucideIcon } from "lucide-react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { pickOpen } from "../lib/pick";
import { UnlockSheet } from "./PhotosView";
import {
  api,
  EXPORT_CANCELLED,
  EXPORT_PROGRESS,
  exportApi,
  backupApi,
  type BackupStatus,
  type AccountInfo,
  type DriveFolder,
  type ExportProgress,
  type LocalStorage,
  type LoginItem,
  messageOf,
  IPHONE_CHANGED,
  iphoneApi,
  type IphoneStatus,
  PHOTO_CACHE,
  photoCacheApi,
  downloadRecordApi,
  RECORD_WHYS,
  type RecordDay,
  type PhotoCache,
  settingsApi,
} from "../lib/ipc";
import { checkedFreshness, useSync } from "../lib/sync";
import { useUpdateStatus } from "../lib/updates";
import { setTheme, storedTheme, type Theme } from "../lib/theme";
import remindersIcon from "../assets/apps/reminders.svg";
import driveIcon from "../assets/apps/drive.svg";
import photosIcon from "../assets/apps/photos.svg";
import calendarIcon from "../assets/apps/calendar.svg";
import { Launcher } from "../components/Launcher";
import { AccountAvatar, useNotKept } from "../components/Account";
import { Picker } from "../components/Picker";
import { localeFirstDay, setWeekStart, weekStartChoice, type WeekStartChoice } from "./calendar/model";
import { icloudSize } from "../lib/icloudSize";
import { appName, t, useI18n, LANGUAGES, type LanguageChoice, type Translate, systemLanguage, languageName } from "../lib/i18n";

// Its licence texts are some 400 kB: loaded when the sheet is first opened, not with Settings.
const LicencesSheet = lazy(() => import("../components/LicencesSheet").then((m) => ({ default: m.LicencesSheet })));

/** The Drive folder's line when it is up: up to date, uploading, or why something is waiting. */
function folderState(f: DriveFolder, t: Translate): string {
  const w = f.waiting;
  if (!w || w.files === 0) return t("settings.driveUpToDate", { path: f.path });
  if (w.failure) return t(w.files === 1 ? "settings.driveFilesWaitingOne" : "settings.driveFilesWaitingMany", { count: w.files, failure: w.failure });
  if (w.sending > 0) return t(w.files === 1 ? "settings.driveFilesUploadingOne" : "settings.driveFilesUploadingMany", { count: w.files });
  return t(w.files === 1 ? "settings.driveFilesWaitingToUploadOne" : "settings.driveFilesWaitingToUploadMany", { count: w.files });
}

/** A size on this computer, with the system's decimal mark: "6.8 GB", "6,8 GB". */
function size(bytes: number): string {
  const gb = bytes / 1024 ** 3;
  const places = gb >= 10 ? 0 : 1;
  if (gb >= 1) return `${gb.toLocaleString(undefined, { minimumFractionDigits: places, maximumFractionDigits: places })} GB`;
  if (bytes < 1024 ** 2) return bytes < 1024 ? t("settings.zeroKB") : `${Math.round(bytes / 1024)} KB`;
  return `${Math.round(bytes / 1024 ** 2)} MB`;
}

/**
 * Settings, as macOS System Settings lays it out: the account at the head, then grouped
 * sections of rows, a label on the left and its control on the right. Only what really
 * changes something is here — every switch is one the backend obeys, every figure is read
 * from Apple or from this computer's library.
 */
type Unsent = { changes: number; files: number; photos: number };

/** What has not reached iCloud yet, as a sentence's subject: "2 changes and 1 file". */
function unsentPhrase(u: Unsent, t: Translate): string | null {
  const parts = [
    u.changes > 0 && t(u.changes === 1 ? "settings.unsentChangeOne" : "settings.unsentChangeMany", { count: u.changes }),
    u.files > 0 && t(u.files === 1 ? "settings.unsentFileOne" : "settings.unsentFileMany", { count: u.files }),
    u.photos > 0 && t(u.photos === 1 ? "settings.unsentPhotoOne" : "settings.unsentPhotoMany", { count: u.photos }),
  ].filter((p): p is string => Boolean(p));
  if (parts.length === 0) return null;
  return parts.length === 1
    ? parts[0]
    : t("settings.unsentJoin", {
        first: parts.slice(0, -1).join(t("settings.unsentComma")),
        last: parts[parts.length - 1],
      });
}

/**
 * Signing out asks what macOS asks: keep a copy here, or remove it. Kept, everything is here
 * again on signing back in — and another Apple ID signing in meanwhile finds it set aside, never
 * mixed with its own. What has not reached iCloud yet is said plainly, since removing loses it.
 */
function SignOutSheet({
  unsent,
  onCancel,
  onChoose,
}: {
  unsent: Unsent;
  onCancel: () => void;
  onChoose: (keep: boolean) => void;
}) {
  const { t } = useI18n();
  const waiting = unsentPhrase(unsent, t);
  const total = unsent.changes + unsent.files + unsent.photos;
  return (
    <ConfirmSheet<boolean>
      title={t("settings.signOutTitle")}
      body={t("settings.signOutBody")}
      choices={[
        { label: t("settings.keepCopy"), value: true, primary: true, focus: true },
        { label: t("settings.removeFromComputer"), value: false, danger: true },
      ]}
      onAnswer={onChoose}
      onCancel={onCancel}
    >
      {waiting && (
        <p className="signout-unsent">
          {total === 1
            ? t("settings.unsentWarning", { waiting })
            : t("settings.unsentWarningPlural", { waiting })}
        </p>
      )}
    </ConfirmSheet>
  );
}


/** A folder of iCloud on this computer, as a row: where it is or why it is not, Open, and its switch. */
function FolderRow({
  app,
  label,
  off,
  folder,
  busy,
  line,
  onOpen,
  onSet,
}: {
  app: string;
  label: string;
  off: string;
  folder: DriveFolder | null;
  busy: boolean;
  line: (f: DriveFolder) => string;
  onOpen: () => Promise<unknown>;
  onSet: (on: boolean) => void;
}) {
  const { t } = useI18n();
  // Why the folder could not be opened, said on its own row until it is tried again.
  const [openFailed, setOpenFailed] = useState<string | null>(null);
  const problem = openFailed ?? folder?.problem;
  const trouble = Boolean(problem || folder?.waiting?.failure);
  return (
    <div className="settings-row">
      <AppMark app={app} />
      <span className="settings-label settings-stack">
        {label}
        <span className={`settings-hint${trouble ? " is-trouble" : ""}`} title={folder?.on ? folder.path : undefined}>
          {problem ?? (folder?.on ? line(folder) : off)}
        </span>
      </span>
      {folder?.on && (
        <button
          className="quiet"
          onClick={() => {
            setOpenFailed(null);
            onOpen().catch((e) => setOpenFailed(messageOf(e)));
          }}
        >
          {t("common.open")}
        </button>
      )}
      <button
        className="switch"
        role="switch"
        aria-checked={folder?.on ?? false}
        aria-label={label}
        disabled={!folder || busy}
        onClick={() => {
          setOpenFailed(null);
          if (folder) onSet(!folder.on);
        }}
      >
        <span className="switch-knob" />
      </button>
    </div>
  );
}

/**
 * Export a Copy, kept outside the screen: an export runs on while Settings is closed, and
 * coming back finds it where it is, not idle. Photos are Back Up Photos' (every original, kept up
 * to date); a copy of only the photos on this computer read as a backup and wasn't one.
 */
type ExportState =
  | { phase: "idle" }
  | { phase: "running"; progress: ExportProgress | null; cancelling: boolean }
  | { phase: "done"; path: string; photosMissing: number };
let exportState: ExportState = { phase: "idle" };
const exportListeners = new Set<() => void>();
const setExportState = (s: ExportState) => {
  exportState = s;
  exportListeners.forEach((l) => l());
};
const subscribeExport = (l: () => void) => {
  exportListeners.add(l);
  return () => exportListeners.delete(l);
};


/** How far the whole export is, 0 to 1: services done, and the share of the one under way. */
function exportShare(p: ExportProgress | null): number | null {
  if (!p || p.steps === 0) return null;
  const within = p.total > 0 ? p.done / p.total : p.done === p.total ? 1 : 0;
  return (p.step - 1 + within) / p.steps;
}

async function runExport(onError: (m: string) => void, t: Translate) {
  const dest = (await pickOpen({ directory: true, multiple: false, title: t("settings.chooseExportLocation") }))?.[0];
  if (!dest) return;
  setExportState({ phase: "running", progress: null, cancelling: false });
  const stop = listen<ExportProgress>(EXPORT_PROGRESS, (e) => {
    if (exportState.phase === "running") setExportState({ ...exportState, progress: e.payload });
  });
  try {
    const summary = await exportApi.start(dest, false);
    setExportState({ phase: "done", path: summary.path, photosMissing: summary.photosMissing });
    toast({
      message: t("settings.exportedTo", { folder: folderName(summary.path) }),
      tone: "done",
      action: { label: t("common.showInFolder"), run: () => void exportApi.reveal().catch((e) => onError(messageOf(e))) },
    });
  } catch (e) {
    const message = messageOf(e);
    setExportState({ phase: "idle" });
    if (message === EXPORT_CANCELLED) toast(t("settings.exportCancelled"));
    else onError(message);
  } finally {
    void stop.then((unlisten) => unlisten()).catch(() => {});
  }
}

const folderName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

/** The row: what goes out and where, Export, then the bar and Cancel, then Show in Folder. */
/**
 * Back Up iCloud: every photo and video, and iCloud Drive's files, copied as originals into a
 * folder the person chooses and kept up to date (src-tauri/src/backup.rs). Off until a folder is
 * chosen. iCloud Drive is in unless switched off; the Hidden album is offered always, and
 * included only once unlocked, with the sheet Photos unlocks it with.
 */
function BackupRow({ onError }: { onError: (m: string) => void }) {
  const { t } = useI18n();
  const [state, setState] = useState<BackupStatus | null>(null);
  const [unlocking, setUnlocking] = useState(false);
  useEffect(() => {
    let live = true;
    void backupApi.status().then((s) => live && setState(s), () => {});
    const off = listen<BackupStatus>("quince://backup", (e) => live && setState(e.payload));
    return () => {
      live = false;
      void off.then((f) => f());
    };
  }, []);
  const set = (folder: string | null, hidden: boolean, drive = state?.drive ?? true) =>
    backupApi.set(folder, hidden, drive).then(setState, (e) => onError(messageOf(e)));
  const choose = async () => {
    const picked = await pickOpen({ directory: true, title: t("settings.chooseBackupLocation") }).catch((e) => {
      onError(messageOf(e));
      return null;
    });
    if (picked) await set(picked[0], state?.hidden ?? false);
  };
  const hidden = () => {
    if (!state) return;
    if (!state.hidden && state.hiddenLocked) setUnlocking(true);
    else void set(state.folder, !state.hidden);
  };
  const on = Boolean(state?.folder);
  const photosCopying = Boolean(state && state.running && state.copied < state.total);
  const photosLine = !state
    ? ""
    : photosCopying
      ? t("settings.photosCountOfTotal", {
          copied: state.copied.toLocaleString(),
          total: state.total.toLocaleString(),
          copiedSize: size(state.photosBytes),
          totalSize: size(state.photosBytesTotal),
        })
      : t(state.total === 1 ? "settings.photosCountAndSizeOne" : "settings.photosCountAndSizeMany", {
          count: state.total.toLocaleString(),
          size: size(state.photosBytesTotal),
        });
  const driveLine = !state
    ? ""
    : !state.drive
      ? t("settings.notInBackup")
      : state.driveRunning
        ? state.driveFilesTotal > 0
          ? t("settings.driveFilesOfTotal", {
              copied: state.driveFiles.toLocaleString(),
              total: state.driveFilesTotal.toLocaleString(),
              copiedSize: size(state.driveBytes),
              totalSize: size(state.driveBytesTotal),
            })
          : t("settings.readingDrive")
        : state.driveFiles > 0
          ? t(state.driveFiles === 1 ? "settings.driveFilesCountAndSizeOne" : "settings.driveFilesCountAndSizeMany", {
              count: state.driveFiles.toLocaleString(),
              size: size(state.driveBytes),
            })
          : t("settings.driveHint");
  return (
    <div className="settings-row is-stacked backup" aria-label={t("settings.backupTitle")} role="group">
      <div className="settings-row-line" style={{ alignItems: "center" }}>
        <span className="settings-label settings-stack">
          {t("settings.backup")}
          <span className={`settings-hint${state?.problem ? " is-trouble" : ""}`} aria-live="polite" title={state?.folder ?? undefined}>
            {!state ? "" : state.problem ? state.problem : on ? (
              <>
                {t("settings.inFolder", { folder: folderName(state.folder!) })}
                <button className="link" onClick={() => void backupApi.reveal().catch((e) => onError(messageOf(e)))}>
                  {t("common.showInFolder")}
                </button>
              </>
            ) : (
              t("settings.backupHint")
            )}
          </span>
        </span>
        <button className="quiet" onClick={() => void choose()} disabled={!state}>
          {on ? t("settings.changeFolder") : t("settings.chooseFolder")}
        </button>
        {on && (
          <button className="switch" role="switch" aria-checked aria-label={t("settings.backup")} onClick={() => void set(null, state?.hidden ?? false)}>
            <span className="switch-knob" />
          </button>
        )}
      </div>
      <div className="backup-source" aria-label={t("settings.photosInBackup")} role="group">
        <AppMark app="photos" />
        <span className="settings-label settings-stack">
          {t("sidebar.photos")}
          <span className="settings-hint" aria-live="polite">{photosLine}</span>
        </span>
      </div>
      {on && photosCopying && state && state.total > 0 && <Progress value={state.copied / state.total} label={t("settings.photosBackupProgress")} />}
      <div className="settings-row-line backup-option" style={{ alignItems: "center" }}>
        <span className="settings-hint">{t("settings.includeHidden")}</span>
        <button className="switch" role="switch" aria-checked={state?.hidden ?? false} aria-label={t("settings.includeHidden")} disabled={!state} onClick={hidden}>
          <span className="switch-knob" />
        </button>
      </div>
      <div className="backup-source" aria-label={t("settings.driveInBackup")} role="group">
        <AppMark app="drivews" />
        <span className="settings-label settings-stack">
          {t("settings.icloudDrive")}
          <span className={`settings-hint${state?.driveNote ? " is-trouble" : ""}`} aria-live="polite">
            {state?.driveNote ?? driveLine}
          </span>
        </span>
        <button
          className="switch"
          role="switch"
          aria-checked={state?.drive ?? false}
          aria-label={t("settings.includeDrive")}
          disabled={!state}
          onClick={() => state && void set(state.folder, state.hidden, !state.drive)}
        >
          <span className="switch-knob" />
        </button>
      </div>
      {on && state?.driveRunning && state.driveFilesTotal > 0 && (
        <Progress value={state.driveFiles / state.driveFilesTotal} label={t("settings.driveBackupProgress")} />
      )}
      {unlocking && (
        <UnlockSheet
          onCancel={() => setUnlocking(false)}
          onUnlocked={() => {
            setUnlocking(false);
            void set(state?.folder ?? null, true);
          }}
        />
      )}
    </div>
  );
}

function ExportRow({ onError }: { onError: (m: string) => void }) {
  const { t } = useI18n();
  const state = useSyncExternalStore(subscribeExport, () => exportState);
  const running = state.phase === "running";
  const p = running ? state.progress : null;
  const hint = running
    ? state.cancelling
      ? t("common.cancelling")
      : p
        ? `${appName(p.service) ?? p.service} · ${p.total > 0 ? t("settings.exportProgressCount", { done: Math.min(p.done, p.total).toLocaleString(), total: p.total.toLocaleString() }) : t("settings.nothingToExport")}`
        : t("common.gettingReady")
    : state.phase === "done"
      ? t("settings.exportedTo", { folder: folderName(state.path) })
      : t("settings.exportCopyHint");
  return (
    <div className="settings-row is-stacked" aria-label={t("settings.exportCopy")} role="group">
      <div className="settings-row-line" style={{ alignItems: "center" }}>
        <span className="settings-label settings-stack">
          {t("settings.exportCopy")}
          <span className="settings-hint" aria-live="polite" title={state.phase === "done" ? state.path : undefined}>
            {hint}
          </span>
        </span>
        {state.phase === "done" && !running && (
          <button className="quiet" onClick={() => void exportApi.reveal().catch((e) => onError(messageOf(e)))}>
            {t("common.showInFolder")}
          </button>
        )}
        {running ? (
          <button
            className="quiet"
            disabled={state.cancelling}
            onClick={() => {
              setExportState({ ...state, cancelling: true });
              void exportApi.cancel().catch(() => {});
            }}
          >
            {t("common.cancel")}
          </button>
        ) : (
          <button className="quiet" onClick={() => void runExport(onError, t)}>
            {t("settings.export")}
          </button>
        )}
      </div>
      {running && <Progress value={exportShare(p)} label={t("settings.exportProgress")} />}
    </div>
  );
}

/** A cap as Settings says it: "512 MB", "2 GB". */
const capWords = (bytes: number) => (bytes < 1024 ** 3 ? `${Math.round(bytes / 1024 ** 2)} MB` : `${Math.round(bytes / 1024 ** 3)} GB`);

/** How much room photos opened here may keep on this computer: one choice, the backend's
 *  steps, sent as it is picked. */
/** The local date as the record keeps it, `back` days ago: "2026-11-25". */
function localDate(back: number): string {
  const d = new Date();
  d.setDate(d.getDate() - back);
  const two = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}`;
}

/**
 * What Skykeep downloaded from iCloud, and why, as iPhone's own Cellular Data list says it:
 * today and the last seven days, then each reason with its share, largest first, and the
 * programs that read the Drive and Photos folders by name. Read from the record the backend
 * keeps on this computer (quince-record); nothing here leaves it.
 */
function Downloaded() {
  const { t } = useI18n();
  const [days, setDays] = useState<RecordDay[] | null>(null);
  useEffect(() => {
    downloadRecordApi.days().then(setDays, () => setDays([]));
  }, []);
  if (!days) return null;
  const week = new Set(Array.from({ length: 7 }, (_, i) => localDate(i)));
  const recent = days.filter((d) => week.has(d.date));
  const total = (d: RecordDay) => d.bytes.reduce((a, b) => a + b, 0);
  const today = recent.find((d) => d.date === localDate(0));
  const byWhy = RECORD_WHYS.map((why, i) => ({ why, bytes: recent.reduce((a, d) => a + (d.bytes[i] ?? 0), 0) }))
    .filter((r) => r.bytes >= 1024 ** 2)
    .sort((a, b) => b.bytes - a.bytes);
  const programs = new Map<string, number>();
  for (const d of recent) for (const [name, n] of Object.entries(d.programs)) programs.set(name, (programs.get(name) ?? 0) + n);
  const readers = [...programs].filter(([, n]) => n >= 1024 ** 2).sort((a, b) => b[1] - a[1]).slice(0, 3);
  const weekTotal = recent.reduce((a, d) => a + total(d), 0);
  return (
    <div className="settings-group" aria-label={t("settings.downloaded")}>
      <div className="settings-row">
        <span className="settings-label settings-stack">
          {t("settings.downloaded")}
          <span className="settings-hint">
            {weekTotal < 1024 ** 2
              ? t("settings.downloadedNone")
              : t("settings.downloadedHint", { today: size(today ? total(today) : 0), week: size(weekTotal) })}
          </span>
        </span>
      </div>
      {byWhy.map(({ why, bytes }) => (
        <div className="settings-row" key={why}>
          <span className="settings-label settings-stack">
            {t(`settings.why.${why}`)}
            {why === "folders" && readers.length > 0 && (
              <span className="settings-hint">
                {readers.map(([name, n]) => t("settings.programRead", { program: name, size: size(n) })).join(", ")}
              </span>
            )}
          </span>
          <span className="settings-value">{size(bytes)}</span>
        </div>
      ))}
    </div>
  );
}

function PhotoCacheCap({ onError }: { onError: (m: string) => void }) {
  const { t } = useI18n();
  const [cache, setCache] = useState<PhotoCache | null>(null);
  useEffect(() => {
    photoCacheApi.status().then(setCache, () => {});
    const stop = listen<PhotoCache>(PHOTO_CACHE, (e) => setCache(e.payload));
    return () => void stop.then((un) => un()).catch(() => {});
  }, []);
  if (!cache) return null;
  return (
    <div className="settings-row">
      <span className="settings-label settings-stack">
        {t("settings.keepPhotosUpTo")}
        <span className="settings-hint">
          {cache.limit < cache.cap
            ? t("settings.diskRoomHint", { size: size(cache.limit) })
            : t("settings.photosOfflineHint", { size: size(cache.used) })}
        </span>
      </span>
      <Picker
        label={t("settings.keepPhotosUpTo")}
        value={String(cache.cap)}
        options={cache.steps.map((b) => ({ value: String(b), label: capWords(b) }))}
        onChange={(v) =>
          photoCacheApi.set(Number(v)).then(setCache, (e) => onError(messageOf(e)))
        }
      />
    </div>
  );
}

/**
 * The iPhone as this computer's camera and microphone, in a card of its own: one switch, off
 * until turned on, so nothing runs for those who don't use it. On, it says the one step
 * missing (plug it in, open Skykeep on it) or that it's in use, as the phone changes.
 */
function useIphone() {
  const [status, setStatus] = useState<IphoneStatus | null>(null);
  useEffect(() => {
    let live = true;
    const read = () => iphoneApi.status().then((s) => live && setStatus(s), () => {});
    void read();
    const off = listen(IPHONE_CHANGED, read);
    return () => {
      live = false;
      off.then((f) => f());
    };
  }, []);
  return [status, setStatus] as const;
}

function IphoneCard({
  status,
  setStatus,
  shown,
}: {
  status: IphoneStatus | null;
  setStatus: Dispatch<SetStateAction<IphoneStatus | null>>;
  shown: boolean;
}) {
  const { t } = useI18n();
  const [busy, setBusy] = useState(false);
  const inUse = status?.state === "inUse" || status?.state === "cameraPaused" || status?.state === "micOnly";
  // While it's in use and this section is open, how quick the picture and sound are, read
  // each second from the link's own counters (a local read; nothing leaves the computer).
  useEffect(() => {
    if (!inUse || !shown) return undefined;
    const tick = window.setInterval(() => {
      iphoneApi.status().then((s) => setStatus((was) => (JSON.stringify(was) === JSON.stringify(s) ? was : s)), () => {});
    }, 1000);
    return () => window.clearInterval(tick);
  }, [inUse, shown, setStatus]);
  if (!status || status.state === "unavailable") return null;
  const on = status.state !== "off";
  const hint = !on
    ? t("settings.iphoneOffHint")
    : status.state === "inUse"
      ? t("settings.iphoneInUse")
      : status.state === "cameraPaused"
        ? t("settings.iphoneCameraPaused")
        : status.state === "micOnly"
          ? t("settings.iphoneMicOnly")
          : status.state === "plugged"
            ? t("settings.iphonePlugged")
            : t("settings.iphoneNone");
  const toggle = async () => {
    setBusy(true);
    try {
      setStatus(await iphoneApi.set(!on));
    } catch {
      /* the switch shows what is still true */
    } finally {
      setBusy(false);
    }
  };
  const setUp = async () => {
    setBusy(true);
    try {
      const everyApp = await iphoneApi.everyApp();
      setStatus((s) => (s ? { ...s, everyApp } : s));
    } catch {
      /* the row says what is still true */
    } finally {
      setBusy(false);
    }
  };
  // Each part as it is now: a light (green live, amber waiting, grey off) and a word.
  type Light = "live" | "wait" | "off";
  const connection: [Light, string] =
    status.state === "none" ? ["off", t("settings.iphoneStateNotConnected")]
    : status.state === "plugged" ? ["wait", t("settings.iphoneStateOpenApp")]
    : ["live", t("settings.iphoneStateConnected")];
  const ms = (v: number | null) => (v == null ? "" : ` · ${Math.round(v)} ms`);
  const camera: [Light, string] =
    status.state === "inUse" ? ["live", t("settings.iphoneStateLive") + ms(status.cameraMs)]
    : status.state === "cameraPaused" ? ["wait", t("settings.iphoneStatePaused")]
    : ["off", t("settings.iphoneStateOff")];
  const microphone: [Light, string] = inUse
    ? ["live", t("settings.iphoneStateLive") + ms(status.microphoneMs)]
    : ["off", t("settings.iphoneStateOff")];
  const Row = ({ label, light: [light, word] }: { label: string; light: [Light, string] }) => (
    <div className="settings-row">
      <span className="settings-label">{label}</span>
      <span className={`prefs-state is-${light}`}>
        <span className="prefs-light" aria-hidden="true" />
        {word}
      </span>
    </div>
  );
  return (
    <Pane id="iphone" shown={shown} title={t("settings.iphone")} badge={t("settings.beta")}>
      <div className="settings-group">
        <div className="settings-row">
          <span className="settings-label settings-stack">
            {t("settings.iphoneCamera")}
            <span className="settings-hint" aria-live="polite">
              {hint}
            </span>
          </span>
          <button
            className="switch"
            role="switch"
            aria-checked={on}
            aria-label={t("settings.iphoneCamera")}
            disabled={busy}
            onClick={toggle}
          >
            <span className="switch-knob" />
          </button>
        </div>
        {on && status.everyApp !== "ready" && (
          <div className="settings-row">
            <span className="settings-label settings-stack">
              {t("settings.iphoneEveryApp")}
              <span className="settings-hint">
                {status.everyApp === "install" ? t("settings.iphoneEveryAppInstall") : t("settings.iphoneEveryAppHint")}
              </span>
            </span>
            {status.everyApp === "setUp" && (
              <button className="quiet" onClick={setUp} disabled={busy}>
                {busy ? t("settings.iphoneSettingUp") : t("settings.iphoneSetUp")}
              </button>
            )}
          </div>
        )}
      </div>
      {on && (
        <div className="settings-group" role="group" aria-label={t("settings.iphoneNow")} aria-live="polite">
          <Row label={t("settings.iphoneConnection")} light={connection} />
          <Row label={t("settings.iphoneCameraPart")} light={camera} />
          <Row label={t("settings.iphoneMicrophonePart")} light={microphone} />
        </div>
      )}
    </Pane>
  );
}

export function SettingsView({ onSignOut }: { onSignOut: (keep: boolean) => Promise<void> }) {
  const { t } = useI18n();
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const [storage, setStorage] = useState<{ usedBytes: number; totalBytes: number } | null>(null);
  const [local, setLocal] = useState<LocalStorage | null>(null);
  const [clearing, setClearing] = useState(false);
  const [freed, setFreed] = useState<number | null>(null);
  const [alerts, setAlerts] = useState<{ reminderAlerts: boolean; eventAlerts: boolean } | null>(null);
  const [alertsShown, setAlertsShown] = useState<boolean | null>(null);
  const [theme, chooseTheme] = useState<Theme>(storedTheme);
  const [version, setVersion] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useErrorToast(error, () => setError(null));
  const [signingOut, setSigningOut] = useState<Unsent | null>(null);
  const [folder, setFolder] = useState<DriveFolder | null>(null);
  const [folderBusy, setFolderBusy] = useState(false);
  const [photosFolder, setPhotosFolder] = useState<DriveFolder | null>(null);
  const [photosBusy, setPhotosBusy] = useState(false);
  const [login, setLogin] = useState<LoginItem | null>(null);
  const [loginBusy, setLoginBusy] = useState(false);
  const [licences, setLicences] = useState(false);
  const { report, syncing, refresh } = useSync();
  const notKept = useNotKept();
  const [iphone, setIphone] = useIphone();
  const [pane, setPane] = useState<PaneId>("home");
  const navRef = useRef<HTMLElement>(null);
  const mainRef = useRef<HTMLDivElement>(null);
  // A section is opened at its top, never somewhere down the last one's scroll.
  const choose = (next: PaneId) => {
    setPane(next);
    mainRef.current?.scrollTo({ top: 0 });
  };

  useEffect(() => {
    api.accountInfo().then(setAccount, () => setAccount({ name: null, email: null }));
    api.storageInfo().then(setStorage, () => {});
    api.settingsGet().then(setAlerts, (e) => setError(messageOf(e)));
    settingsApi.alertsShown().then(setAlertsShown, () => {});
    settingsApi.localStorage().then(setLocal, () => {});
    getVersion().then(setVersion, () => {});
    api.driveFolderStatus().then(setFolder, () => {});
    api.photosFolderStatus().then(setPhotosFolder, () => {});
    api.loginItem().then(setLogin, () => {});
  }, []);
  // While the Drive folder is up, what it has still to send is read every two seconds (a
  // local question; nothing asked of Apple), so nothing ever waits unseen.
  const folderOn = folder?.on ?? false;
  useEffect(() => {
    if (!folderOn) return undefined;
    // Kept only when it changed: a new object every two seconds re-rendered all of Settings,
    // and a re-render landing mid-scroll was a dropped frame — the stutter while scrolling.
    const t = window.setInterval(() => {
      api
        .driveFolderStatus()
        .then((next) => setFolder((was) => (JSON.stringify(was) === JSON.stringify(next) ? was : next)), () => {});
    }, 2000);
    return () => window.clearInterval(t);
  }, [folderOn]);

  // A switch moves the moment it is pressed, and goes back with the reason if it could not
  // be kept.
  const toggle = (key: "reminderAlerts" | "eventAlerts") => {
    if (!alerts) return;
    const on = !alerts[key];
    setAlerts({ ...alerts, [key]: on });
    api.settingsSet(key, on).catch((e) => {
      setAlerts((now) => (now ? { ...now, [key]: !on } : now));
      setError(messageOf(e));
    });
  };

  // A folder's switch moves at once; Apple's answer only matters when it could not come up,
  // and then the reason is said on the folder's own row, where the eye already is.
  const setFolderOn = (
    current: DriveFolder,
    on: boolean,
    put: (f: DriveFolder) => void,
    busy: (b: boolean) => void,
    ask: (on: boolean) => Promise<DriveFolder>,
  ) => {
    put({ ...current, on, problem: null });
    busy(true);
    ask(on)
      .then(put, (e) => put({ ...current, problem: messageOf(e) }))
      .finally(() => busy(false));
  };

  const clearPictures = () => {
    if (!local) return;
    const before = local.total;
    setClearing(true);
    setFreed(null);
    settingsApi
      .localStorageClear()
      .then(
        (after) => {
          setLocal(after);
          setFreed(Math.max(0, before - after.total));
        },
        (e) => setError(messageOf(e)),
      )
      .finally(() => setClearing(false));
  };

  const used = storage && storage.totalBytes > 0 ? Math.min(1, storage.usedBytes / storage.totalBytes) : 0;
  const showDrive = folder?.available !== false;
  const showPhotos = photosFolder?.available !== false;
  const MB = 1024 ** 2;

  const nav: { id: Exclude<PaneId, "home">; label: string }[][] = [
    [
      { id: "backup", label: t("settings.backupTitle") },
      { id: "computer", label: t("settings.thisComputer") },
      ...(iphone && iphone.state !== "unavailable" ? [{ id: "iphone" as const, label: t("settings.iphone") }] : []),
    ],
    [
      { id: "notifications", label: t("settings.notifications") },
      { id: "general", label: t("settings.general") },
    ],
    [
      { id: "privacy", label: t("settings.privacy") },
      { id: "updates", label: t("settings.softwareUpdate") },
    ],
  ];
  const order: PaneId[] = ["home", ...nav.flat().map((n) => n.id)];
  // Up and down move through the sections, as in a list; Home and End to either end.
  const onNavKey = (e: React.KeyboardEvent) => {
    const at = order.indexOf(pane);
    const next =
      e.key === "ArrowDown" ? order[Math.min(order.length - 1, at + 1)]
      : e.key === "ArrowUp" ? order[Math.max(0, at - 1)]
      : e.key === "Home" ? order[0]
      : e.key === "End" ? order[order.length - 1]
      : null;
    if (!next) return;
    e.preventDefault();
    choose(next);
    navRef.current?.querySelector<HTMLElement>(`[data-pane="${next}"]`)?.focus();
  };

  return (
    <div className="settings prefs">
      <h1 className="visually-hidden">Skykeep</h1>
      <nav
        className="prefs-side"
        role="tablist"
        aria-orientation="vertical"
        aria-label={t("settings.sections")}
        ref={navRef}
        onKeyDown={onNavKey}
      >
        <div className="prefs-group">
          <button
            className="source-row"
            role="tab"
            data-pane="home"
            id="tab-home"
            aria-controls="pane-home"
            aria-selected={pane === "home"}
            tabIndex={pane === "home" ? 0 : -1}
            onClick={() => choose("home")}
          >
            <House className="prefs-icon" size={16} strokeWidth={1.75} aria-hidden="true" />
            <span className="source-name">{t("settings.home")}</span>
          </button>
        </div>
        {nav.map((group, i) => (
          <div className="prefs-group" key={i}>
            {group.map((n) => (
              <button
                key={n.id}
                className="source-row"
                role="tab"
                data-pane={n.id}
                id={`tab-${n.id}`}
                aria-controls={`pane-${n.id}`}
                aria-selected={pane === n.id}
                tabIndex={pane === n.id ? 0 : -1}
                onClick={() => choose(n.id)}
              >
                <NavTile pane={n.id} />
                <span className="source-name">{n.label}</span>
              </button>
            ))}
          </div>
        ))}
      </nav>
      <div className="prefs-main" ref={mainRef}>
        <div className="prefs-column">
        <Pane id="home" shown={pane === "home"} title={t("settings.home")} hideTitle>
            <div className="prefs-hero">
              <AccountAvatar account={account} large />
              <div className="prefs-hero-text">
                <strong>{account?.name ?? t("settings.yourAppleAccount")}</strong>
                <span className="prefs-hero-mail">{account?.email ?? ""}</span>
                {notKept && <span className="settings-account-note">{t("account.notKeptNotice")}</span>}
                <div className="prefs-storage">
                  <div className="settings-row-line">
                    <span>
                      {storage
                        ? t("settings.storageUsed", { used: icloudSize(storage.usedBytes), total: icloudSize(storage.totalBytes) })
                        : t("settings.readingStorage")}
                    </span>
                    {storage && (
                      <span className="settings-value">
                        {t("settings.storageAvailable", { available: icloudSize(Math.max(0, storage.totalBytes - storage.usedBytes)) })}
                      </span>
                    )}
                  </div>
                  <div
                    className="storage-bar"
                    role="meter"
                    aria-label={t("settings.storageAria")}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={Math.round(used * 100)}
                  >
                    <span style={{ width: `${used * 100}%` }} />
                  </div>
                </div>
              </div>
              <div className="prefs-hero-actions">
                <button className="quiet" onClick={() => void refresh(true)} disabled={syncing}>
                  {syncing ? t("sync.syncing") : t("launcher.syncNow")}
                </button>
                <button
                  className="quiet"
                  onClick={() => api.unsent().then(setSigningOut, () => setSigningOut({ changes: 0, files: 0, photos: 0 }))}
                >
                  {t("settings.signOut")}
                </button>
              </div>
            </div>
            <Launcher
              services={report?.services ?? []}
              syncing={syncing}
              driveFree={storage ? icloudSize(Math.max(0, storage.totalBytes - storage.usedBytes)) : null}
            />
          </Pane>
        <Pane id="backup" shown={pane === "backup"} title={t("settings.backupTitle")}>
            <div className="settings-group">
              <BackupRow onError={setError} />
            </div>
            <div className="settings-group">
              <ExportRow onError={setError} />
            </div>
          </Pane>
        <Pane id="computer" shown={pane === "computer"} title={t("settings.thisComputer")}>
            {(showDrive || showPhotos) && (
              <div className="settings-group">
                {showDrive && (
                  <FolderRow
                    app="drivews"
                    label={t("settings.driveFolder")}
                    off={onWindows ? t("settings.driveFolderExplorerHint") : t("settings.driveFolderFilesHint")}
                    folder={folder}
                    busy={folderBusy}
                    line={(f) => folderState(f, t)}
                    onOpen={api.driveFolderOpen}
                    onSet={(on) => folder && setFolderOn(folder, on, setFolder, setFolderBusy, api.driveFolderSet)}
                  />
                )}
                {showPhotos && (
                  <FolderRow
                    app="photos"
                    label={t("settings.photosFolder")}
                    off={t("settings.photosFolderHint")}
                    folder={photosFolder}
                    busy={photosBusy}
                    line={(f) => f.path}
                    onOpen={api.photosFolderOpen}
                    onSet={(on) =>
                      photosFolder && setFolderOn(photosFolder, on, setPhotosFolder, setPhotosBusy, api.photosFolderSet)
                    }
                  />
                )}
              </div>
            )}
            <div className="settings-group">
              <div className="settings-row">
                <span className="settings-label settings-stack">
                  {t("settings.cachedPictures")}
                  <span className="settings-hint" aria-live="polite">
                    {freed != null ? t("settings.clearedHint", { size: size(freed) }) : t("settings.cachedPicturesHint")}
                  </span>
                </span>
                {local && local.clearable >= MB && <span className="settings-value">{size(local.clearable)}</span>}
                <button className="quiet" onClick={clearPictures} disabled={!local || clearing || local.clearable < MB}>
                  {clearing ? t("common.clearing") : t("common.clear")}
                </button>
              </div>
              <PhotoCacheCap onError={setError} />
            </div>
            <Downloaded />
          </Pane>
        <IphoneCard status={iphone} setStatus={setIphone} shown={pane === "iphone"} />
        <Pane id="notifications" shown={pane === "notifications"} title={t("settings.notifications")}>
            <div className="settings-group">
              {(
                [
                  ["reminderAlerts", t("settings.reminderAlerts"), "reminders"],
                  ["eventAlerts", t("settings.calendarAlerts"), "calendar"],
                ] as const
              ).map(([key, label, icon]) => (
                <div className="settings-row" key={key}>
                  <img className="prefs-app-icon" src={icon === "reminders" ? remindersIcon : calendarIcon} alt="" draggable={false} />
                  <span className="settings-label">{label}</span>
                  <button
                    className="switch"
                    role="switch"
                    aria-checked={alerts?.[key] ?? false}
                    aria-label={label}
                    disabled={!alerts}
                    onClick={() => toggle(key)}
                  >
                    <span className="switch-knob" />
                  </button>
                </div>
              ))}
              {/* Asked of the desktop itself: without a notification service every alert is dropped. */}
              {alertsShown === false && (
                <div className="settings-row">
                  <span className="settings-label settings-hint is-trouble">{t("settings.noNotificationService")}</span>
                </div>
              )}
            </div>
          </Pane>
        <Pane id="general" shown={pane === "general"} title={t("settings.general")}>
            <div className="settings-group">
              <div className="settings-row">
                <span className="settings-label">{t("settings.appearance")}</span>
                <div className="segmented" role="group" aria-label={t("settings.appearance")}>
                  {(["auto", "light", "dark"] as const).map((tOpt) => (
                    <button
                      key={tOpt}
                      className="segment"
                      aria-pressed={theme === tOpt}
                      onClick={() => {
                        chooseTheme(tOpt);
                        setTheme(tOpt);
                      }}
                    >
                      {tOpt === "auto" ? t("settings.auto") : tOpt === "light" ? t("settings.light") : t("settings.dark")}
                    </button>
                  ))}
                </div>
              </div>
              <LanguageRow />
              <WeekStartRow />
            </div>
            <div className="settings-group">
              {login?.available === false ? (
                <div className="settings-row">
                  <span className="settings-label settings-stack">
                    {t("settings.openAtLogin")}
                    <span className="settings-hint">{t("settings.openAtLoginUnavailable")}</span>
                  </span>
                </div>
              ) : (
                <div className="settings-row">
                  <span className="settings-label settings-stack">
                    {t("settings.openAtLogin")}
                    <span className="settings-hint">
                      {login?.elsewhere
                        ? login.elsewhere
                        : login?.on && !login.honoured
                          ? t("settings.openAtLoginUnsupported")
                          : t("settings.openAtLoginDesc")}
                    </span>
                    {login?.on && !login.honoured && <code className="settings-command">{login.command}</code>}
                  </span>
                  <button
                    className="switch"
                    role="switch"
                    aria-checked={login?.on ?? false}
                    aria-label={t("settings.openAtLogin")}
                    disabled={!login || loginBusy || Boolean(login.elsewhere)}
                    onClick={() => {
                      if (!login) return;
                      const was = login;
                      setLogin({ ...login, on: !login.on });
                      setLoginBusy(true);
                      api
                        .loginItemSet(!was.on)
                        .then(setLogin, (e) => {
                          setLogin(was);
                          setError(messageOf(e));
                        })
                        .finally(() => setLoginBusy(false));
                    }}
                  >
                    <span className="switch-knob" />
                  </button>
                </div>
              )}
              {/* Only where Skykeep starts itself at login, with the flag this setting is read for. */}
              {login?.available !== false && login?.on && (
                <div className="settings-row">
                  <span className="settings-label settings-stack">
                    {t("settings.startInTray")}
                    <span className="settings-hint">
                      {login.startInTray !== false ? t("settings.startInTrayDesc") : t("settings.startInTrayOpen")}
                    </span>
                  </span>
                  <button
                    className="switch"
                    role="switch"
                    aria-checked={login.startInTray !== false}
                    aria-label={t("settings.startInTray")}
                    disabled={loginBusy}
                    onClick={() => {
                      const was = login;
                      const on = was.startInTray === false;
                      setLogin({ ...was, startInTray: on });
                      setLoginBusy(true);
                      api
                        .loginStartInTraySet(on)
                        .then(setLogin, (e) => {
                          setLogin(was);
                          setError(messageOf(e));
                        })
                        .finally(() => setLoginBusy(false));
                    }}
                  >
                    <span className="switch-knob" />
                  </button>
                </div>
              )}
            </div>
          {/* Notes' own setting, as Apple keeps it in Notes' settings on the iPhone and the Mac. */}
            <h2 className="settings-heading">{t("sidebar.notes")}</h2>
            <div className="settings-group">
              <MoveCheckedRow />
            </div>
          </Pane>
        {/* What Skykeep talks to, in plain sight: a statement of trust is not hidden behind a link. */}
        <Pane id="privacy" shown={pane === "privacy"} title={t("settings.privacy")}>
            <ReachGroup />
          </Pane>
        <Pane id="updates" shown={pane === "updates"} title={t("settings.softwareUpdate")}>
            <UpdatesGroup />
            <div className="settings-group">
              <div className="settings-row">
                <span className="settings-label settings-stack">
                  Skykeep {version ?? ""}
                  <span className="settings-hint">Copyright (C) 2026 Yashwanth Kamireddi. {t("settings.licence")}</span>
                </span>
              </div>
              <div className="settings-row">
                <span className="settings-label">{t("settings.saveDiagnosticReport")}</span>
                <button
                  className="quiet"
                  aria-label={t("settings.saveDiagnosticReport")}
                  onClick={() =>
                    api.diagnosticsSave().then(
                      (path) => void toastSaved(path),
                      (e) => setError(messageOf(e)),
                    )
                  }
                >
                  {t("common.save")}
                </button>
              </div>
              {/* Where Skykeep is made, in the open: most who install it from a store never see it. */}
              <div className="settings-row">
                <span className="settings-label settings-stack">
                  {t("settings.githubRepo")}
                  <span className="settings-hint">{t("settings.githubRepoHint")}</span>
                </span>
                <a className="quiet" href="https://github.com/YashwanthKamireddi/Skykeep" target="_blank" rel="noreferrer">
                  {t("common.open")}
                </a>
              </div>
              <div className="settings-row">
                <span className="settings-label">{t("settings.openSourceLicences")}</span>
                <button className="quiet" onClick={() => setLicences(true)}>
                  {t("common.show")}
                </button>
              </div>
            </div>
          </Pane>
        </div>
      </div>
      {licences && (
        <Suspense fallback={null}>
          <LicencesSheet onClose={() => setLicences(false)} />
        </Suspense>
      )}
      {signingOut && (
        <SignOutSheet
          unsent={signingOut}
          onCancel={() => setSigningOut(null)}
          onChoose={(keep) => {
            setSigningOut(null);
            onSignOut(keep).catch((e) => setError(messageOf(e)));
          }}
        />
      )}
    </div>
  );
}

/** An app's own icon, the one on its tile in Home and in the launcher. */
const MARKS: Record<string, string> = { drivews: driveIcon, photos: photosIcon, reminders: remindersIcon, calendar: calendarIcon };
function AppMark({ app }: { app: string }) {
  const src = MARKS[app];
  return src ? <img className="prefs-app-icon" src={src} alt="" draggable={false} /> : null;
}

type PaneId = "home" | "backup" | "computer" | "iphone" | "notifications" | "general" | "privacy" | "updates";

/** Each section's mark in the sidebar: one plain glyph, as a Mac's own sidebars have them. */
const TILES: Record<Exclude<PaneId, "home">, LucideIcon> = {
  backup: Archive,
  computer: Laptop,
  iphone: Smartphone,
  notifications: Bell,
  general: Gear,
  privacy: ShieldCheck,
  updates: ArrowDownCircle,
};

function NavTile({ pane }: { pane: Exclude<PaneId, "home"> }) {
  const Icon = TILES[pane];
  return <Icon className="prefs-icon" size={16} strokeWidth={1.75} aria-hidden="true" />;
}

/** One section's pane: its title, then its groups. */
function Pane({
  id,
  title,
  hideTitle,
  badge,
  shown,
  children,
}: {
  id: PaneId;
  title: string;
  hideTitle?: boolean;
  /** A word beside the title, such as Beta, said as part of it. */
  badge?: string;
  shown: boolean;
  children: ReactNode;
}) {
  // Every section is made once, when Settings opens, and kept: hidden while another is shown, it
  // has its answers ready, so showing it again draws what is true at once, never a first frame
  // of placeholders that then flips (a switch on, then off).
  return (
    <section className="prefs-pane" id={`pane-${id}`} role="tabpanel" aria-labelledby={`card-${id}`} hidden={!shown}>
      <h2 id={`card-${id}`} className={hideTitle ? "visually-hidden" : "prefs-title"}>
        {title}
        {badge && <span className="prefs-badge">{badge}</span>}
      </h2>
      {children}
    </section>
  );
}

/** Interface language: the system's preferred language, or one chosen here. */
function LanguageRow() {
  const { choice, setLanguage, t } = useI18n();
  const sysLang = systemLanguage();
  const sysName = languageName(sysLang);
  return (
    <div className="settings-row">
      <span className="settings-label">{t("settings.language")}</span>
      <Picker
        label={t("settings.language")}
        value={choice}
        onChange={(v) => {
          setLanguage(v as LanguageChoice);
        }}
        options={[
          { value: "auto", label: `${t("settings.auto")} (${sysName})` },
          ...LANGUAGES.map((l) => ({ value: l, label: languageName(l) })),
        ]}
      />
    </div>
  );
}

/** Calendar's first day of the week: the locale's, or one chosen here, as Calendar on a Mac. */
function WeekStartRow() {
  const { t } = useI18n();
  const [choice, setChoice] = useState<WeekStartChoice>(weekStartChoice);
  const day = (n: number) => new Intl.DateTimeFormat(dateLocale(), { weekday: "long" }).format(new Date(2026, 0, 4 + n));
  // The locale's own day, named in Automatic, so it is clear what Automatic gives.
  const locale = localeFirstDay();
  return (
    <div className="settings-row">
      <span className="settings-label">{t("settings.firstDayOfWeek")}</span>
      <Picker
        label={t("settings.firstDayOfWeek")}
        value={choice}
        onChange={(v) => {
          const next = v as WeekStartChoice;
          setChoice(next);
          setWeekStart(next);
        }}
        options={[
          { value: "auto", label: `${t("settings.auto")} (${day(locale)})` },
          { value: "0", label: day(0) },
          { value: "1", label: day(1) },
          { value: "6", label: day(6) },
        ]}
      />
    </div>
  );
}

type Reach = { accounts: string[]; apple: string[]; subscribed: string[]; updates: string[] };

/** What Skykeep can reach, as its network locks hold it (`network_reach`). */
function useReach(): Reach | null {
  const [reach, setReach] = useState<Reach | null>(null);
  useEffect(() => {
    api.networkReach().then(setReach, () => {});
  }, []);
  return reach;
}

/** A list of hosts, each as code. */
function Hosts({ hosts }: { hosts: string[] }) {
  return (
    <>
      {hosts.map((h, i) => (
        <span key={h}>
          {i > 0 && ", "}
          <code>{h}</code>
        </span>
      ))}
    </>
  );
}

/**
 * What Skykeep talks to, read from the network locks themselves: Apple's domains
 * (`quince_auth::APPLE_DOMAINS`), each subscribed calendar's host, and GitHub's release servers
 * (`quince_auth::releases::RELEASE_HOSTS`) for a copy that updates itself. Each is reached by a
 * client of its own that can reach nothing else.
 */
function ReachGroup() {
  const { t } = useI18n();
  const reach = useReach();
  if (!reach) return null;
  const apple = reach.apple.filter((d) => !d.endsWith(".cn"));
  return (
    <>
      <h2 className="settings-heading">{t("settings.whatSkykeepTalksTo")}</h2>
      <section className="settings-group settings-reach" aria-label={t("settings.whatSkykeepTalksTo")}>
        <div className="settings-row is-stacked">
          <span className="settings-label">{t("settings.apple")}</span>
          <span className="settings-hint settings-hint-wrap">
            <Hosts hosts={apple} /> {t("settings.appleReachDesc")}
          </span>
        </div>
        <div className="settings-row is-stacked">
          <span className="settings-label">{t("settings.calendarAccounts")}</span>
          <span className="settings-hint settings-hint-wrap">
            {reach.accounts.length === 0 ? t("settings.noCalendarAccounts") : <Hosts hosts={reach.accounts} />}
          </span>
        </div>
        <div className="settings-row is-stacked">
          <span className="settings-label">{t("settings.calendarSubscriptions")}</span>
          <span className="settings-hint settings-hint-wrap">
            {reach.subscribed.length === 0 ? t("settings.noCalendarSubscriptions") : <Hosts hosts={reach.subscribed} />}
          </span>
        </div>
        <div className="settings-row is-stacked">
          <span className="settings-label">{t("settings.updates")}</span>
          <span className="settings-hint settings-hint-wrap">
            {reach.updates.length === 0 ? (
              t("settings.noUpdatesReach")
            ) : (
              <>
                <Hosts hosts={reach.updates} />, {t("settings.githubUpdatesReachDesc")}
              </>
            )}
          </span>
        </div>
      </section>
    </>
  );
}


/** When this copy last checked, said as a person would. */
function checkedLine(at: number | null): string {
  return checkedFreshness(at);
}

/**
 * Software Update: Check for Updates Automatically, and Check for Updates — or, for a copy a
 * package manager installed, who updates it instead. What is found is offered by the update
 * notice; here it can be installed too.
 */
function UpdatesGroup() {
  const { t } = useI18n();
  const [status, setStatus] = useUpdateStatus();
  const [checking, setChecking] = useState(false);
  if (!status) return null;
  const here = status.install === "here";
  const elsewhere: Record<string, string> = {
    packageManager: t("settings.packageManagerUpdate"),
    flatpak: t("settings.flatpakUpdate"),
    store: t("settings.storeUpdate"),
    development: t("settings.devBuildNoUpdate"),
    manual: t("settings.manualUpdate"),
  };
  const busy = checking || status.phase === "checking";
  const offered = (status.phase === "available" || status.phase === "ready") && status.version;
  let line: string;
  let trouble = false;
  switch (status.phase) {
    case "checking":
      line = t("settings.checkingForUpdates");
      break;
    case "available":
    case "ready":
      line = t("settings.updateAvailable", { version: status.version ?? "" });
      break;
    case "downloading":
      line = t("settings.downloadingUpdate", { version: status.version ?? "" });
      break;
    case "waiting":
      line =
        status.waitingFor === "typing"
          ? t("settings.installWhenFinishTyping", { version: status.version ?? "" })
          : t("settings.installWhenSynced", { version: status.version ?? "" });
      break;
    case "installing":
      line = t("settings.installingUpdate", { version: status.version ?? "" });
      break;
    case "failed":
      line = status.problem ?? t("settings.updateFailed");
      trouble = true;
      break;
    case "current":
      line = t("settings.upToDate", { version: status.current, checked: checkedLine(status.checkedAt) });
      break;
    default:
      line = t("settings.versionChecked", { version: status.current, checked: checkedLine(status.checkedAt) });
  }
  return (
    <>
      <section className="settings-group" aria-label={t("settings.softwareUpdate")}>
        {!here ? (
          <div className="settings-row">
            <span className="settings-label settings-hint">{elsewhere[status.install]}</span>
          </div>
        ) : (
          <>
            <div className="settings-row">
              <span className="settings-label settings-stack">
                {t("settings.checkUpdatesAuto")}
                <span className="settings-hint">{t("settings.checkUpdatesAutoHint")}</span>
              </span>
              <button
                className="switch"
                role="switch"
                aria-checked={status.auto}
                aria-label={t("settings.checkUpdatesAuto")}
                onClick={() => {
                  const was = status;
                  setStatus({ ...status, auto: !status.auto });
                  api.updateAutoSet(!was.auto).catch(() => setStatus(was));
                }}
              >
                <span className="switch-knob" />
              </button>
            </div>
            <div className="settings-row">
              <span className={`settings-label settings-hint settings-hint-wrap${trouble ? " is-trouble" : ""}`} aria-live="polite">
                {line}
              </span>
              {offered ? (
                <button className="quiet" onClick={() => void api.updateApply().catch(() => {})}>
                  {t("settings.updateAndRestart")}
                </button>
              ) : (
                <button
                  className="quiet"
                  disabled={busy || ["downloading", "waiting", "installing"].includes(status.phase)}
                  onClick={() => {
                    setChecking(true);
                    setStatus({ ...status, phase: "checking" });
                    api
                      .updateCheck()
                      .then(setStatus, (e) => setStatus({ ...status, phase: "failed", problem: messageOf(e) }))
                      .finally(() => setChecking(false));
                  }}
                >
                  {t("settings.checkForUpdates")}
                </button>
              )}
            </div>
          </>
        )}
      </section>
    </>
  );
}

/** Notes' Sort Checked Items, Automatically: on this computer, as each iPhone keeps its own. */
function MoveCheckedRow() {
  const { t } = useI18n();
  const [on, setOn] = useState(() => {
    try {
      return localStorage.getItem(MOVE_CHECKED_KEY) === "1";
    } catch {
      return false;
    }
  });
  const choose = (next: boolean) => {
    setOn(next);
    try {
      localStorage.setItem(MOVE_CHECKED_KEY, next ? "1" : "0");
    } catch {
      /* kept for this session only */
    }
  };
  // Apple's own choice, worded as Apple words it: Sort Checked Items, Manually or Automatically.
  return (
    <div className="settings-row">
      <span className="settings-label">{t("settings.sortChecked")}</span>
      <div className="segmented" role="group" aria-label={t("settings.sortChecked")}>
        <button className="segment" aria-pressed={!on} onClick={() => choose(false)}>
          {t("settings.manually")}
        </button>
        <button className="segment" aria-pressed={on} onClick={() => choose(true)}>
          {t("settings.automatically")}
        </button>
      </div>
    </div>
  );
}
