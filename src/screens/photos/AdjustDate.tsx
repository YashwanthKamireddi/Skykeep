/**
 * Adjust Date and Time, as icloud.com's: the photo, the timezone the times are in, the original
 * date, and the one to give it — to the second. Save only once it is a different, real date.
 */
import { dateLocale } from "../../lib/when";
import { t } from "../../lib/i18n";
import { useActiveRef } from "../../lib/screen";
import { useEffect, useRef, useState } from "react";
import { useModalFocus } from "../../lib/focus";
import { X } from "lucide-react";
import { pictureSrc, type Photo } from "../../lib/ipc";

const pad = (n: number) => String(n).padStart(2, "0");
/** A moment as the field takes it, in this computer's own time: 2026-06-30T21:26:54. */
const local = (seconds: number) => {
  const d = new Date(seconds * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
};
/** "GMT+05:30", as icloud.com names the zone. */
const zone = () => {
  const m = -new Date().getTimezoneOffset();
  return `GMT${m >= 0 ? "+" : "−"}${pad(Math.floor(Math.abs(m) / 60))}:${pad(Math.abs(m) % 60)}`;
};

export function AdjustDate({ photo, onSave, onClose }: { photo: Photo; onSave: (takenAt: number) => void; onClose: () => void }) {
  const sheetRef = useRef<HTMLFormElement | null>(null);
  useModalFocus(sheetRef);
  const [value, setValue] = useState(() => local(photo.takenAt));
  const live = useActiveRef();
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (!live.current) return;
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, [onClose]);
  const ms = Date.parse(value);
  const next = Number.isNaN(ms) ? null : Math.floor(ms / 1000);
  const changed = next !== null && next !== photo.takenAt && next > -2_208_988_800 && next < 253_402_300_800;
  const shown = new Date(photo.takenAt * 1000).toLocaleString(dateLocale(), { dateStyle: "medium", timeStyle: "medium" });
  return (
    <div className="modal-veil" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <form
        ref={sheetRef}
        className="modal-sheet px-sheet"
        role="dialog"
        aria-modal="true"
        aria-label={t("photos.adjustDateTime")}
        onSubmit={(e) => {
          e.preventDefault();
          if (changed && next !== null) onSave(next);
        }}
      >
        <button type="button" className="icon-button small round sheet-close" aria-label={t("ui.close")} onClick={onClose}>
          <X size={16} strokeWidth={2.2} aria-hidden="true" />
        </button>
        <h2>{t("photos.adjustDateTime")}</h2>
        <div className="px-dl-pics" aria-hidden="true">
          <img src={pictureSrc(photo.id, "thumb")} alt="" />
        </div>
        <p className="px-date-zone">All times are {zone()}</p>
        <div className="px-date-grid">
          <span>{t("photos.original")}</span>
          <output className="px-date-was">{shown}</output>
          <label htmlFor="px-adjusted">{t("photos.adjusted")}</label>
          <input
            id="px-adjusted"
            className="field px-date-field"
            type="datetime-local"
            step={1}
            value={value}
            autoFocus
            onChange={(e) => setValue(e.target.value)}
          />
        </div>
        <div className="sheet-actions">
          <button type="button" className="quiet" onClick={onClose}>
            {t("photos.cancel")}
          </button>
          <button type="submit" className="primary" disabled={!changed}>
            {t("photos.save")}
          </button>
        </div>
      </form>
    </div>
  );
}
