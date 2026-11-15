import { useEffect, useRef, useState } from "react";
import { t } from "../../lib/i18n";
import { ImagePlus } from "lucide-react";
import { Portal } from "../../components/Portal";
import { useModalFocus } from "../../lib/focus";

/** The circle a face is chosen in, and the frame around it, in CSS pixels. */
const FRAME = 240;
const RADIUS = 108;
/** The longest `data:` URL icloud.com sends for a picture: the image is made to fit under it. */
const LIMIT = 229_376;

export type ChosenPhoto = {
  /** The whole picture, as a JPEG `data:` URL — what icloud.com sends. */
  url: string;
  /** The square, `x,y,width,height` in the sent picture's pixels, measured from its bottom left. */
  crop: string;
  /** The square alone, small, to show on the card until iCloud has the picture. */
  preview: string;
};

type Loaded = { src: string; img: HTMLImageElement; w: number; h: number };
/** Where the picture sits: its point under the circle's centre, and its scale. */
type View = { cx: number; cy: number; scale: number };

const minScale = (l: Loaded) => (2 * RADIUS) / Math.min(l.w, l.h);

/** The view moved or zoomed, kept so the circle never shows past the picture's edge. */
function within(l: Loaded, v: View): View {
  const scale = Math.max(minScale(l), Math.min(v.scale, minScale(l) * 8));
  const half = RADIUS / scale;
  return {
    scale,
    cx: Math.min(Math.max(v.cx, half), l.w - half),
    cy: Math.min(Math.max(v.cy, half), l.h - half),
  };
}

/** The picture drawn to a canvas and made a JPEG under icloud.com's size: quality first, then size. */
function encode(l: Loaded): { url: string; factor: number } | null {
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  let factor = Math.min(1, 2048 / Math.max(l.w, l.h));
  let quality = 0.9;
  for (let i = 0; i < 24; i++) {
    canvas.width = Math.max(1, Math.round(l.w * factor));
    canvas.height = Math.max(1, Math.round(l.h * factor));
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(l.img, 0, 0, canvas.width, canvas.height);
    const url = canvas.toDataURL("image/jpeg", quality);
    if (url.length <= LIMIT) return { url, factor: canvas.width / l.w };
    if (quality > 0.4) quality -= 0.15;
    else factor *= 0.8;
  }
  return null;
}

/**
 * Choosing a contact's picture, as Contacts does: a photo picked or dropped, then moved and
 * zoomed until the face sits in the circle. Done sends the whole picture with the square
 * chosen, as icloud.com does; Delete Photo takes the picture away.
 */
export function PhotoSheet({ hasPhoto, onDone, onRemove, onCancel }: {
  hasPhoto: boolean;
  onDone: (photo: ChosenPhoto) => void;
  onRemove: () => void;
  onCancel: () => void;
}) {
  const sheet = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheet);
  const input = useRef<HTMLInputElement | null>(null);
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [view, setView] = useState<View | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const drag = useRef<{ x: number; y: number; from: View } | null>(null);

  useEffect(() => () => {
    if (loaded?.src.startsWith("blob:")) URL.revokeObjectURL(loaded.src);
  }, [loaded]);

  const open = (file: File | null | undefined) => {
    if (!file) return;
    if (!/^image\/(png|jpeg|gif|bmp|webp)$/.test(file.type)) {
      setProblem("Choose a JPEG, PNG, GIF, BMP or WebP picture.");
      return;
    }
    const src = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      const l = { src, img, w: img.naturalWidth, h: img.naturalHeight };
      setProblem(null);
      setLoaded(l);
      setView(within(l, { cx: l.w / 2, cy: l.h / 2, scale: minScale(l) }));
    };
    img.onerror = () => setProblem("This picture can't be read. Choose another.");
    img.src = src;
  };

  const zoomTo = (scale: number) => loaded && view && setView(within(loaded, { ...view, scale }));
  const done = () => {
    if (!loaded || !view) return;
    const encoded = encode(loaded);
    if (!encoded) {
      setProblem("This picture is too large to use. Choose a smaller one.");
      return;
    }
    const side = (2 * RADIUS) / view.scale;
    const top = view.cy - side / 2;
    const f = encoded.factor;
    const x = Math.round((view.cx - side / 2) * f);
    // Apple measures the square from the picture's bottom edge, as the Mac does.
    const y = Math.round((loaded.h - top - side) * f);
    const s = Math.max(1, Math.round(side * f));
    const small = document.createElement("canvas");
    small.width = small.height = 160;
    small.getContext("2d")?.drawImage(loaded.img, view.cx - side / 2, top, side, side, 0, 0, 160, 160);
    onDone({
      url: encoded.url,
      crop: `${Math.max(0, x)},${Math.max(0, y)},${s},${s}`,
      preview: small.toDataURL("image/jpeg", 0.85),
    });
  };

  const min = loaded ? minScale(loaded) : 1;
  return (
    <Portal>
      <div className="modal-veil" onClick={onCancel}>
        <div
          ref={sheet}
          className="modal-sheet photo-sheet"
          role="dialog"
          aria-modal="true"
          aria-label={t("contacts.photo.aria")}
          onClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              onCancel();
            }
          }}
          onDragOver={(e) => e.preventDefault()}
          onDrop={(e) => {
            e.preventDefault();
            open(e.dataTransfer.files?.[0]);
          }}
        >
          <h2>{t("contacts.photo.title")}</h2>
          <div
            className={`photo-frame${loaded ? " has-photo" : ""}`}
            style={{ width: FRAME, height: FRAME }}
            onPointerDown={(e) => {
              if (!view) return;
              e.currentTarget.setPointerCapture(e.pointerId);
              drag.current = { x: e.clientX, y: e.clientY, from: view };
            }}
            onPointerMove={(e) => {
              const d = drag.current;
              if (!d || !loaded) return;
              setView(within(loaded, {
                ...d.from,
                cx: d.from.cx - (e.clientX - d.x) / d.from.scale,
                cy: d.from.cy - (e.clientY - d.y) / d.from.scale,
              }));
            }}
            onPointerUp={() => (drag.current = null)}
            onPointerCancel={() => (drag.current = null)}
            onWheel={(e) => view && zoomTo(view.scale * (e.deltaY < 0 ? 1.08 : 1 / 1.08))}
          >
            {loaded && view ? (
              <>
                <img
                  className="photo-frame-image"
                  src={loaded.src}
                  alt=""
                  draggable={false}
                  style={{
                    width: loaded.w * view.scale,
                    height: loaded.h * view.scale,
                    transform: `translate(${FRAME / 2 - view.cx * view.scale}px, ${FRAME / 2 - view.cy * view.scale}px)`,
                  }}
                />
                <span className="photo-frame-ring" style={{ width: RADIUS * 2, height: RADIUS * 2 }} aria-hidden="true" />
              </>
            ) : (
              <button type="button" className="photo-frame-pick" onClick={() => input.current?.click()}>
                <ImagePlus size={30} strokeWidth={1.5} aria-hidden="true" />
                <span>{t("contacts.photo.choose")}</span>
              </button>
            )}
          </div>
          {loaded && view && (
            <label className="photo-zoom">
              <span>{t("contacts.photo.zoom")}</span>
              <input
                type="range"
                min={0}
                max={100}
                step={1}
                aria-label={t("contacts.photo.zoom")}
                value={Math.round((Math.log(view.scale / min) / Math.log(8)) * 100)}
                onChange={(e) => zoomTo(min * 8 ** (Number(e.target.value) / 100))}
              />
            </label>
          )}
          {problem && <p className="photo-problem" role="alert">{problem}</p>}
          <input
            ref={input}
            type="file"
            accept="image/png,image/jpeg,image/gif,image/bmp,image/webp"
            hidden
            aria-label={t("contacts.photo.file")}
            onChange={(e) => {
              open(e.target.files?.[0]);
              e.target.value = "";
            }}
          />
          <div className="photo-sheet-actions">
            {hasPhoto && !loaded && (
              <button type="button" className="quiet danger" onClick={onRemove}>{t("ui.deletePhoto")}</button>
            )}
            {loaded && (
              <button type="button" className="quiet" onClick={() => input.current?.click()}>{t("ui.chooseAnother")}</button>
            )}
            <span className="photo-sheet-gap" />
            <button type="button" className="quiet" onClick={onCancel}>{t("ui.cancel")}</button>
            <button type="button" className="primary" disabled={!loaded} onClick={done}>{t("ui.done")}</button>
          </div>
        </div>
      </div>
    </Portal>
  );
}
