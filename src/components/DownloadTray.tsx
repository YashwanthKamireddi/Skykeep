import { t } from "../lib/i18n";
import { useEffect, useRef, useState } from "react";
import { Progress } from "./Spinner";
import { FileGlyph } from "./FileGlyph";
import { toast } from "./Toast";
import { api, messageOf } from "../lib/ipc";
import { heading, view } from "../lib/downloadModel";
import {
  cancelDownload,
  dismissFinished,
  downloadAgain,
  forgetDownload,
  offerUnfinished,
  useDownloads,
} from "../lib/downloads";

/**
 * Downloads into Downloads, in the corner where uploads are shown and in the same shape: one
 * line for each folder or selection, however many files are in it, with its progress, and —
 * once it is there — Show in Folder. One that stopped short says why and offers to Resume
 * where it was; one left unfinished when Skykeep was last closed is offered again. A line
 * opens to say what in it did not download, and why. It stays until it is closed: Show in
 * Folder is what it is for.
 */
export function DownloadTray() {
  const lines = useDownloads();
  const [open, setOpen] = useState<string | null>(null);
  useEffect(() => {
    void offerUnfinished();
  }, []);
  // Progress only ever moves forward: four files arrive at once and report out of step, and a
  // bar that slid back on every report read as broken.
  const highest = useRef<Map<string, number>>(new Map());
  const forward = (key: string, value: number | null) => {
    if (value === null) return null;
    const now = Math.max(highest.current.get(key) ?? 0, value);
    highest.current.set(key, now);
    return now;
  };
  if (lines.length === 0) {
    highest.current.clear();
    return null;
  }
  const views = lines.map(view);
  const active = views.some((v) => v.running);
  const one = views.length === 1 ? views[0] : null;
  const headPercent = one?.running ? forward(`head:${one.batch}`, one.percent) : null;
  const reveal = (path: string) =>
    void api.downloadReveal(path).catch((e) => toast({ message: messageOf(e), tone: "error" }));

  return (
    <section className="upload-tray download-tray" aria-label={t("downloads.title")} aria-live="polite">
      <header className="upload-tray-head">
        <span className="upload-tray-title">{heading(views)}</span>
        {active ? (
          <span className="upload-tray-actions">
            {headPercent !== null && <span className="upload-tray-percent">{headPercent}%</span>}
            <button
              className="quiet upload-tray-close"
              onClick={() => views.filter((v) => v.running).forEach((v) => void cancelDownload(v.batch))}
            >
              {t("downloads.cancel")}
            </button>
          </span>
        ) : (
          <button className="quiet upload-tray-close" onClick={dismissFinished} aria-label={t("ui.closeDownloads")}>
            {t("downloads.done")}
          </button>
        )}
      </header>
      {active && one && <Progress value={headPercent === null ? null : headPercent / 100} className="upload-bar" />}
      <ul className="upload-rows">
        {views.map((v) => {
          const p = forward(v.batch, v.percent);
          const unfolds = v.failures.length > 0 && !v.running;
          return (
            <li key={v.batch} className={`upload-row is-${v.tone}`}>
              <div className="upload-line">
                <span className="upload-glyph" aria-hidden="true">
                  <FileGlyph kind="FOLDER" extension={null} />
                </span>
                <span className="upload-text">
                  <span className="upload-name">{v.title}</span>
                  {v.running && views.length > 1 && (
                    <Progress value={p === null ? null : p / 100} className="upload-bar small" />
                  )}
                  {v.note && <span className="upload-where" title={v.note}>{v.note}</span>}
                </span>
                <span className="upload-state">
                  {v.tone === "done" ? <span aria-label={t("downloads.downloaded")}>✓</span> : v.status}
                </span>
                <span className="download-actions">
                  {v.again && (
                    <button className="quiet download-show" onClick={() => void downloadAgain(v.batch)}>
                      {t(v.again === "Resume" ? "downloads.resume" : "downloads.tryAgain")}
                    </button>
                  )}
                  {v.reveal && (
                    <button className="quiet download-show" onClick={() => reveal(v.reveal!)}>
                      {t("downloads.showInFolder")}
                    </button>
                  )}
                </span>
                {unfolds ? (
                  <button
                    className="icon-button small round upload-stop"
                    aria-expanded={open === v.batch}
                    aria-label={open === v.batch ? t("downloads.hideFailed") : t("downloads.showFailed")}
                    onClick={() => setOpen(open === v.batch ? null : v.batch)}
                  >
                    {open === v.batch ? "▾" : "▸"}
                  </button>
                ) : v.running && views.length > 1 ? (
                  // One download has one Cancel — the head's.
                  <button
                    className="icon-button small round upload-stop"
                    aria-label={t("uploads.cancelName", { name: v.title })}
                    title={t("uploads.cancel")}
                    onClick={() => void cancelDownload(v.batch)}
                  >
                    ✕
                  </button>
                ) : v.forgettable ? (
                  <button
                    className="icon-button small round upload-stop"
                    aria-label={t("downloads.forgetName", { name: v.title })}
                    title={t("downloads.dontOfferTitle")}
                    onClick={() => void forgetDownload(v.batch, v.forgettable!)}
                  >
                    ✕
                  </button>
                ) : (
                  <span />
                )}
              </div>
              {open === v.batch && unfolds && (
                <ul className="upload-failures">
                  {v.failures.map((f, i) => (
                    <li key={`${f.path}:${i}`}>
                      <b>{f.path}</b> {f.error}
                    </li>
                  ))}
                  {v.unnamed > 0 && <li>and {v.unnamed} more</li>}
                  {v.forgettable && (
                    <li>
                      <button className="quiet download-show" onClick={() => void forgetDownload(v.batch, v.forgettable!)}>
                        {t("downloads.dontOffer")}
                      </button>
                    </li>
                  )}
                </ul>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
