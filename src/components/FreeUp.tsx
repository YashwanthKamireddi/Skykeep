/**
 * Free up iCloud space (docs/free-up-space.md): photos and videos the backup holds leave iCloud,
 * to Recently Deleted, each checked against Apple's own copy first (quince_backup::free_up).
 *
 * The storage bar is the preview: the part that would leave is striped, with the space used now,
 * after, and freed, changing as the age is chosen. One sentence says where things go — Google
 * Photos' own "free up space" is widely mistaken for something else. Every photo shows, by
 * month, and any can be unticked; what stays in iCloud is listed with why. The one button says
 * what it does. Afterwards, the result as it came: removed, and any kept and why.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Portal } from "./Portal";
import { useModalFocus } from "../lib/focus";
import { useI18n, type Key } from "../lib/i18n";
import { dateLocale } from "../lib/when";
import { icloudSize } from "../lib/icloudSize";
import {
  backupApi,
  freeUpApi,
  messageOf,
  pictureSrc,
  FREE_UP_FAILED,
  type BackupStatus,
  type DriveFreePlan,
  type FreeItem,
  type FreeUpPlan,
  type ICloudStorage,
} from "../lib/ipc";

const AGES: { months: number; label: Key }[] = [
  { months: 3, label: "settings.freeUp3m" },
  { months: 12, label: "settings.freeUp1y" },
  { months: 24, label: "settings.freeUp2y" },
];

/** The line under the storage bar: what can leave, and the way in. Nothing while there's nothing. */
export function FreeUpEntry({ storage, onFreed }: { storage: ICloudStorage | null; onFreed: () => void }) {
  const { t } = useI18n();
  const [plan, setPlan] = useState<FreeUpPlan | null>(null);
  const [backup, setBackup] = useState<BackupStatus | null>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    backupApi.status().then(setBackup, () => {});
    freeUpApi.plan(12).then(setPlan, () => {});
  }, [open]);
  if (!storage || !backup?.folder || !plan || plan.items.length === 0) return null;
  const bytes = plan.items.reduce((sum, i) => sum + i.bytes, 0);
  return (
    <>
      <div className="free-up-entry">
        <span>{t("settings.freeUpLine", { size: icloudSize(bytes) })}</span>
        <button className="quiet" onClick={() => setOpen(true)}>
          {t("settings.freeUpButton")}
        </button>
      </div>
      {open && (
        <FreeUpSheet
          storage={storage}
          folder={backup.folder}
          onClose={(freed) => {
            setOpen(false);
            if (freed) onFreed();
          }}
        />
      )}
    </>
  );
}

function FreeUpSheet({
  storage,
  folder,
  onClose,
}: {
  storage: ICloudStorage;
  folder: string;
  onClose: (freed: boolean) => void;
}) {
  const { t } = useI18n();
  const sheetRef = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheetRef);
  const [what, setWhat] = useState<"photos" | "drive">("photos");
  const [months, setMonths] = useState(12);
  const [plan, setPlan] = useState<FreeUpPlan | null>(null);
  const [drivePlan, setDrivePlan] = useState<DriveFreePlan | null>(null);
  const [kept, setKept] = useState<Set<string>>(new Set());
  const [progress, setProgress] = useState<BackupStatus["freeUp"] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [started, setStarted] = useState(false);

  useEffect(() => {
    let live = true;
    setPlan(null);
    freeUpApi.plan(months).then((p) => live && setPlan(p), (e) => live && setProblem(messageOf(e)));
    return () => {
      live = false;
    };
  }, [months]);
  // iCloud Drive's plan asks Apple which backed-up files still exist: read once, when chosen.
  useEffect(() => {
    if (what !== "drive" || drivePlan) return undefined;
    let live = true;
    freeUpApi.drivePlan().then((p) => live && setDrivePlan(p), (e) => live && setProblem(messageOf(e)));
    return () => {
      live = false;
    };
  }, [what, drivePlan]);
  useEffect(() => {
    let live = true;
    const offs = [
      listen<BackupStatus>("quince://backup", (e) => live && setProgress(e.payload.freeUp)),
      listen<string>(FREE_UP_FAILED, (e) => {
        if (!live) return;
        setProblem(e.payload);
        setStarted(false);
      }),
    ];
    return () => {
      live = false;
      offs.forEach((o) => void o.then((off) => off()));
    };
  }, []);

  const leaving = useMemo<{ id: string; bytes: number }[]>(
    () => (what === "photos" ? (plan?.items ?? []) : (drivePlan?.items ?? [])).filter((i) => !kept.has(i.id)),
    [what, plan, drivePlan, kept],
  );
  // Before a run, what is ticked; once one has run, what it actually freed.
  const leavingBytes = started && progress ? progress.freedBytes : leaving.reduce((sum, i) => sum + i.bytes, 0);
  const total = storage.totalBytes;
  // The part of Apple's split the leaving bytes come out of; drawn at the end when Apple's split
  // has no such part, so the bar always says what the figures say.
  const kindKey = what === "photos" ? "photos" : "docs";
  const after = Math.max(0, storage.usedBytes - leavingBytes);
  const months_ = useMemo(() => byMonth(plan?.items ?? []), [plan]);
  const until = new Date(Date.now() + 30 * 86_400_000).toLocaleDateString(dateLocale(), { day: "numeric", month: "long" });
  const drive = folder.split(/[\\/]/).filter(Boolean).pop() ?? folder;
  const running = started && (progress?.running ?? true);
  const done = started && progress && !progress.running && progress.checked > 0;
  const stays = plan?.stays;
  const driveStays = drivePlan?.stays;
  const staysRows: [Key, number, Key | null][] =
    what === "photos"
      ? stays
        ? [
            ["settings.freeUpFavourites", stays.favourites, null],
            ["settings.freeUpHidden", stays.hidden, null],
            ["settings.freeUpSharedLibrary", stays.sharedLibrary, "settings.freeUpSharedWhy"],
            ["settings.freeUpNotBacked", stays.notBackedUp, "settings.freeUpNotBackedWhy"],
            ["settings.freeUpUndated", stays.undated, "settings.freeUpUndatedWhy"],
          ]
        : []
      : driveStays
        ? [
            ["settings.freeUpPackages", driveStays.packages, "settings.freeUpPackagesWhy"],
            ["settings.freeUpAppLibraries", driveStays.appLibraries, "settings.freeUpAppLibrariesWhy"],
            ["settings.freeUpSharedWithYou", driveStays.sharedWithYou, "settings.freeUpSharedWithYouWhy"],
          ]
        : [];
  const staysTotal = staysRows.reduce((sum, [, n]) => sum + n, 0);

  const start = () => {
    setProblem(null);
    setStarted(true);
    setProgress(null);
    (what === "photos" ? freeUpApi.start : freeUpApi.driveStart)(leaving.map((i) => i.id)).catch((e) => {
      setProblem(messageOf(e));
      setStarted(false);
    });
  };
  const close = () => {
    if (running) return;
    onClose(Boolean(done && progress && progress.freed > 0));
  };

  return (
    <Portal>
      <div className="modal-veil" onClick={close}>
        <div
          ref={sheetRef}
          className="modal-sheet free-up"
          role="dialog"
          aria-modal="true"
          aria-label={t("settings.freeUpTitle")}
          onClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              close();
            }
          }}
        >
          <header className="free-up-head">
            <h2>{t("settings.freeUpTitle")}</h2>
            <p>{t(what === "photos" ? "settings.freeUpBody" : "settings.freeUpBodyDrive", { drive, date: until })}</p>
            <div className="storage-bar free-up-bar" role="meter" aria-label={t("settings.freeUpAfterAria")}
              aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round((after / Math.max(1, total)) * 100)}>
              {storage.kinds.length === 0 ? (
                <>
                  <span className="storage-part" style={{ width: `${(after / Math.max(1, total)) * 100}%` }} />
                  <span className="storage-part is-leaving" style={{ width: `${(leavingBytes / Math.max(1, total)) * 100}%` }} />
                </>
              ) : (
                [...storage.kinds, ...(storage.kinds.some((k) => k.key === kindKey) ? [] : [{ key: kindKey, label: "", color: null, bytes: leavingBytes }])].flatMap((k) => {
                  const going = k.key === kindKey ? Math.min(k.bytes, leavingBytes) : 0;
                  const parts = [
                    <span key={k.key} className="storage-part" style={{ width: `${((k.bytes - going) / total) * 100}%`, background: k.color ?? undefined }} />,
                  ];
                  if (going > 0)
                    parts.push(
                      <span key={`${k.key}-going`} className="storage-part is-leaving"
                        style={{ width: `${(going / total) * 100}%`, ["--leaving" as string]: k.color ?? "var(--accent)" }} />,
                    );
                  return parts;
                })
              )}
            </div>
            <div className="free-up-figures">
              <span>
                {t("settings.freeUpNow", { used: icloudSize(storage.usedBytes), after: icloudSize(after) })}
              </span>
              <span>{t("settings.freeUpFreed", { freed: icloudSize(leavingBytes), total: icloudSize(total) })}</span>
            </div>
          </header>

          {!started && (
            <div className="free-up-choose">
              <div className="segmented" role="group" aria-label={t("settings.freeUpWhatAria")}>
                {(["photos", "drive"] as const).map((w) => (
                  <button key={w} className="segment" aria-pressed={what === w} onClick={() => { setWhat(w); setKept(new Set()); setProblem(null); }}>
                    {t(w === "photos" ? "sidebar.photos" : "settings.icloudDrive")}
                  </button>
                ))}
              </div>
              {what === "photos" && (
                <>
                  <span className="free-up-age-label">{t("settings.freeUpOlderThan")}</span>
                  <div className="segmented" role="group" aria-label={t("settings.freeUpOlderThan")}>
                    {AGES.map((a) => (
                      <button key={a.months} className="segment" aria-pressed={months === a.months} onClick={() => { setMonths(a.months); setKept(new Set()); }}>
                        {t(a.label)}
                      </button>
                    ))}
                  </div>
                </>
              )}
            </div>
          )}

          {!started && (
            <div className="free-up-body">
              {what === "photos" ? (
                <div className="free-up-grid" role="group" aria-label={t("settings.freeUpPhotosAria")}>
                  {plan && plan.items.length === 0 && <p className="free-up-empty">{t("settings.freeUpNothing")}</p>}
                  {months_.map((m) => (
                    <Month key={m.key} month={m} kept={kept} onToggle={(id) => setKept((k) => toggled(k, id))} />
                  ))}
                </div>
              ) : (
                <div className="free-up-grid" role="group" aria-label={t("settings.freeUpDriveAria")}>
                  {!drivePlan && !problem && <p className="free-up-empty">{t("settings.freeUpReadingDrive")}</p>}
                  {drivePlan && drivePlan.items.length === 0 && <p className="free-up-empty">{t("settings.freeUpNothingDrive")}</p>}
                  {drivePlan && drivePlan.items.length > 0 && (
                    <ul className="free-up-files">
                      {drivePlan.items.map((f) => {
                        const on = !kept.has(f.id);
                        return (
                          <li key={f.id}>
                            <button className="free-up-file" role="checkbox" aria-checked={on} onClick={() => setKept((k) => toggled(k, f.id))}>
                              <span className="free-up-file-name">{f.name}</span>
                              <span className="free-up-file-folder">{f.folder || t("settings.icloudDrive")}</span>
                              <span className="free-up-file-size">{icloudSize(f.bytes)}</span>
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  )}
                </div>
              )}
              <aside className="free-up-stays" aria-label={t("settings.freeUpStays")}>
                <h3>
                  {t("settings.freeUpStays")}
                  <span className="free-up-stays-sum">{staysTotal.toLocaleString()}</span>
                </h3>
                <ul>
                  {staysRows.filter(([, n]) => n > 0).map(([label, n, why]) => (
                    <li key={label}>
                      <b>{t(label)}</b>
                      <i>{n.toLocaleString()}</i>
                      {why && <small>{t(why)}</small>}
                    </li>
                  ))}
                </ul>
              </aside>
            </div>
          )}

          {started && (
            <div className="free-up-result" role="status" aria-live="polite">
              {running && progress && (
                <p>{t("settings.freeUpChecking", { checked: progress.checked.toLocaleString(), total: progress.total.toLocaleString() })}</p>
              )}
              {running && !progress && <p>{t("settings.freeUpStarting")}</p>}
              {done && progress && (
                <>
                  <p className="free-up-done">
                    {t(
                      what === "photos"
                        ? progress.freed === 1 ? "settings.freeUpDoneOne" : "settings.freeUpDoneMany"
                        : progress.freed === 1 ? "settings.freeUpDoneFilesOne" : "settings.freeUpDoneFilesMany",
                      {
                      count: progress.freed.toLocaleString(),
                      size: icloudSize(progress.freedBytes),
                    })}
                  </p>
                  {progress.edited > 0 && <p>{t("settings.freeUpKeptEdited", { count: progress.edited.toLocaleString() })}</p>}
                  {progress.notSame > 0 && <p>{t("settings.freeUpKeptNotSame", { count: progress.notSame.toLocaleString() })}</p>}
                </>
              )}
            </div>
          )}

          {problem && <p className="settings-hint is-trouble free-up-problem" role="alert">{problem}</p>}

          <footer className="free-up-foot">
            {!started && <p>{t("settings.freeUpUntick")}</p>}
            {running ? (
              <button className="quiet" onClick={() => void freeUpApi.stop()}>{t("settings.freeUpStop")}</button>
            ) : done ? (
              <button className="primary" autoFocus onClick={close}>{t("settings.freeUpDoneButton")}</button>
            ) : (
              <>
                <button className="quiet" autoFocus onClick={close}>{t("common.cancel")}</button>
                <button className="primary danger" disabled={leaving.length === 0} onClick={start}>
                  {t(leaving.length === 1 ? "settings.freeUpRemoveOne" : "settings.freeUpRemoveMany", { count: leaving.length.toLocaleString() })}
                </button>
              </>
            )}
          </footer>
        </div>
      </div>
    </Portal>
  );
}

type MonthGroup = { key: string; label: string; items: FreeItem[]; bytes: number };

function byMonth(items: FreeItem[]): MonthGroup[] {
  const groups: MonthGroup[] = [];
  const fmt = new Intl.DateTimeFormat(dateLocale(), { month: "long", year: "numeric" });
  for (const item of items) {
    const d = new Date(item.takenAt * 1000);
    const key = `${d.getFullYear()}-${d.getMonth()}`;
    let g = groups[groups.length - 1];
    if (!g || g.key !== key) {
      g = { key, label: fmt.format(d), items: [], bytes: 0 };
      groups.push(g);
    }
    g.items.push(item);
    g.bytes += item.bytes;
  }
  return groups;
}

function toggled(set: Set<string>, id: string): Set<string> {
  const next = new Set(set);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return next;
}

/** One month: its name, count and size, and its photos — drawn only once it scrolls near view,
 *  so a library of tens of thousands opens as fast as one of ten. */
function Month({ month, kept, onToggle }: { month: MonthGroup; kept: Set<string>; onToggle: (id: string) => void }) {
  const { t } = useI18n();
  const ref = useRef<HTMLElement | null>(null);
  const [near, setNear] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el || near) return undefined;
    const io = new IntersectionObserver((entries) => entries.some((e) => e.isIntersecting) && setNear(true), { rootMargin: "400px" });
    io.observe(el);
    return () => io.disconnect();
  }, [near]);
  const rows = Math.ceil(month.items.length / 7);
  return (
    <section ref={ref} className="free-up-month" aria-label={month.label}>
      <h4>
        {month.label}
        <span>
          {t(month.items.length === 1 ? "settings.freeUpMonthOne" : "settings.freeUpMonthMany", {
            count: month.items.length.toLocaleString(),
            size: icloudSize(month.bytes),
          })}
        </span>
      </h4>
      <div className="free-up-tiles" style={near ? undefined : { minHeight: `${rows * 64}px` }}>
        {near &&
          month.items.map((i) => {
            const on = !kept.has(i.id);
            return (
              <button key={i.id} className="free-up-tile" role="checkbox" aria-checked={on}
                aria-label={t(on ? "settings.freeUpTileLeaves" : "settings.freeUpTileStays")} onClick={() => onToggle(i.id)}>
                <img src={pictureSrc(i.id, "thumb")} alt="" loading="lazy" decoding="async" draggable={false} />
              </button>
            );
          })}
      </div>
    </section>
  );
}
