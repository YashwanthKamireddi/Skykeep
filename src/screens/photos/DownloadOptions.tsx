/**
 * Download Options, as icloud.com's: the photos (one, or how many), then Unmodified Originals —
 * as captured or imported — and, including edits, Highest Resolution or Most Compatible. Each
 * row downloads at once; Escape or × closes.
 */
import { t } from "../../lib/i18n";
import { useActiveRef } from "../../lib/screen";
import { useEffect, useRef } from "react";
import { useModalFocus } from "../../lib/focus";
import { Download, X } from "lucide-react";
import { pictureSrc, type Photo } from "../../lib/ipc";

export type Want = "originals" | "highestQuality" | "mostCompatible";

export function DownloadOptions({ photos, onPick, onClose }: { photos: Photo[]; onPick: (want: Want) => void; onClose: () => void }) {
  const sheetRef = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheetRef);
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
  const n = photos.length;
  const row = (want: Want, title: string, words: string) => (
    <div className="px-dl-row">
      <span>
        <strong>{title}</strong>
        <small>{words}</small>
      </span>
      <button className="icon-button px-dl-go" aria-label={t("photos.downloadItem", { title })} title={t("ui.download")} onClick={() => onPick(want)}>
        <Download size={16} strokeWidth={2} aria-hidden="true" />
      </button>
    </div>
  );
  return (
    <div className="modal-veil" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={sheetRef} className="modal-sheet px-sheet" role="dialog" aria-modal="true" aria-label={t("ui.downloadOptions")}>
        <button className="icon-button small round sheet-close" aria-label={t("ui.close")} onClick={onClose}>
          <X size={16} strokeWidth={2.2} aria-hidden="true" />
        </button>
        <h2>{t("photos.downloadOptions")}</h2>
        <div className="px-dl-pics" aria-hidden="true">
          {photos.slice(0, 3).map((p) => (
            <img key={p.id} src={pictureSrc(p.id, "thumb")} alt="" />
          ))}
        </div>
        <p className="px-dl-count">{n === 1 ? "1 Item" : `${n} Items`}</p>
        {row("originals", "Unmodified Originals", "Originals as captured or imported")}
        <p className="px-dl-head">{t("ui.includingEdits")}</p>
        {row("highestQuality", t("photos.highestQuality"), t("photos.highestQualityHint"))}
        {row("mostCompatible", t("photos.mostCompatible"), t("photos.mostCompatibleHint"))}
      </div>
    </div>
  );
}
