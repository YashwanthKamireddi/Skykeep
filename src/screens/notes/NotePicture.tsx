/**
 * A note's picture shown large, as clicking one in Notes on the Mac does: over the dimmed
 * window, at once — the picture already drawn in the note, nothing fetched again — with its
 * name and a way to open it in another app. Escape, the close button or a click outside put
 * it away.
 */
import { useEffect, useRef, useState } from "react";
import { useModalFocus } from "../../lib/focus";
import { X } from "lucide-react";
import { useI18n } from "../../lib/i18n";
import { Portal } from "../../components/Portal";
import { api, messageOf } from "../../lib/ipc";
import { PICTURE_EVENT, type PictureShown } from "./NoteEditor";

export function NotePicture() {
  const { t } = useI18n();
  const [shown, setShown] = useState<PictureShown | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  // Said on the button at once, until the other app has it.
  const [opening, setOpening] = useState(false);
  useEffect(() => {
    const show = (e: Event) => {
      setProblem(null);
      setOpening(false);
      setShown((e as CustomEvent<PictureShown>).detail);
    };
    window.addEventListener(PICTURE_EVENT, show);
    return () => window.removeEventListener(PICTURE_EVENT, show);
  }, []);
  useEffect(() => {
    if (!shown) return undefined;
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setShown(null);
      }
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, [shown]);
  // The picture as the note drew it, at once; then the whole picture in its place once it is
  // read and decoded — never a blank frame waiting for it.
  const [whole, setWhole] = useState<string | null>(null);
  useEffect(() => {
    setWhole(null);
    const src = shown?.whole;
    if (!src) return undefined;
    let live = true;
    const img = new Image();
    img.src = src;
    img.decode().then(
      () => live && setWhole(src),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [shown]);
  const sheetRef = useRef<HTMLElement | null>(null);
  useModalFocus(sheetRef, shown !== null);
  if (!shown) return null;
  return (
    <Portal>
      <div className="modal-veil nf-picture-veil" onClick={() => setShown(null)}>
        <figure
          ref={sheetRef}
          className="nf-picture"
          role="dialog"
          aria-modal="true"
          aria-label={shown.name}
          onClick={(e) => e.stopPropagation()}
        >
          {shown.drawing ? (
            <>
              <img src={shown.src} alt={shown.name} draggable={false} className="nf-drawing nf-ink-light" />
              <img src={shown.srcDark} alt={shown.name} draggable={false} className="nf-drawing nf-ink-dark" />
            </>
          ) : (
            <img src={whole ?? shown.src} alt={shown.name} draggable={false} />
          )}
          <figcaption>
            <span className="nf-picture-name">{problem ?? shown.name}</span>
            {!shown.drawing && (
              <button
                className="quiet"
                disabled={opening}
                aria-busy={opening}
                onClick={() => {
                  setOpening(true);
                  void api
                    .noteAttachmentOpen(shown.id)
                    .catch((e) => setProblem(messageOf(e)))
                    .finally(() => setOpening(false));
                }}
              >
                {opening ? t("notes.opening") : t("notes.openInDefaultApp")}
              </button>
            )}
            <button className="icon-button round nf-picture-close" aria-label={t("common.close")} autoFocus onClick={() => setShown(null)}>
              <X size={16} strokeWidth={2} />
            </button>
          </figcaption>
        </figure>
      </div>
    </Portal>
  );
}
