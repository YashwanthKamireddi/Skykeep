import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Link, Type, Unlink } from "lucide-react";
import { Portal } from "../../components/Portal";
import { useI18n } from "../../lib/i18n";
import { useActiveRef } from "../../lib/screen";
import { api } from "../../lib/ipc";
import { linkAddress } from "./linking";

/** What the box was opened on: where, the link already there, and whether text is asked for. */
export type LinkAsk = {
  /** Where the selection starts across, and its top and foot on screen: the box opens below
   * it, left-aligned with it, or above when there is no room below. */
  at: { x: number; top: number; bottom: number };
  /** The note's own area: the box stays inside it, never over the list or the sidebar. */
  within: { left: number; right: number; top: number; bottom: number };
  /** The address of the link the cursor stands in, to change or take off. */
  href: string | null;
  /** Nothing selected and no link there: the words to show are asked for too. */
  needsText: boolean;
};

/**
 * Add Link (Command-K on a Mac), drawn as Notes' own link popover and as this app's other
 * glass surfaces are: one field for the address, a second for the words to show only when
 * nothing is selected, no buttons. Return adds it, Escape or a click away leaves the note as it
 * was; on a link already there, the unlink symbol in the field takes it off.
 */
export function LinkBox({
  ask,
  onDone,
  onRemove,
  onCancel,
}: {
  ask: LinkAsk;
  onDone: (href: string, text: string) => void;
  onRemove: () => void;
  /** Closed with nothing changed: `back` into the note (Escape, Cancel), or clicked away. */
  onCancel: (back: boolean) => void;
}) {
  const { t } = useI18n();
  const box = useRef<HTMLFormElement | null>(null);
  const [address, setAddress] = useState(ask.href ?? "");
  const [text, setText] = useState("");
  const [wrong, setWrong] = useState(false);
  const field = useRef<HTMLInputElement | null>(null);
  const [pos, setPos] = useState({ left: ask.at.x, top: ask.at.bottom + 6, width: 280 });
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    // Inside the note: as wide as there is room for, up to 280, never over the list beside it.
    const area = ask.within;
    const room = Math.max(200, area.right - area.left - 16);
    const width = Math.min(280, room);
    const height = el.offsetHeight;
    const lo = Math.max(8, area.left + 8);
    const hi = Math.min(window.innerWidth, area.right) - width - 8;
    const left = Math.max(lo, Math.min(ask.at.x - 12, hi));
    const floor = Math.min(window.innerHeight, area.bottom) - 8;
    const below = ask.at.bottom + 6;
    const top = below + height <= floor ? below : Math.max(Math.max(8, area.top + 4), ask.at.top - height - 6);
    setPos({ left, top, width });
  }, [ask, wrong]);
  // A link just copied is what is wanted nine times in ten: offered in the field, selected, so
  // Return takes it and typing replaces it — as Mac apps offer the clipboard's address.
  useEffect(() => {
    if (ask.href) return;
    let live = true;
    api.clipboardRead().then(
      ({ text }) => {
        const offered = text?.trim() ?? "";
        if (!live || !offered || offered.length > 2000 || !linkAddress(offered)) return;
        setAddress((now) => {
          if (now) return now;
          requestAnimationFrame(() => field.current?.select());
          return offered;
        });
      },
      () => {},
    );
    return () => {
      live = false;
    };
  }, [ask.href]);
  const live = useActiveRef();
  useEffect(() => {
    const down = (e: MouseEvent) => {
      if (!box.current?.contains(e.target as Node)) onCancel(false);
    };
    const key = (e: KeyboardEvent) => {
      if (live.current && e.key === "Escape") {
        e.preventDefault();
        onCancel(true);
      }
    };
    const resize = () => onCancel(true);
    document.addEventListener("mousedown", down);
    window.addEventListener("keydown", key);
    window.addEventListener("resize", resize);
    return () => {
      document.removeEventListener("mousedown", down);
      window.removeEventListener("keydown", key);
      window.removeEventListener("resize", resize);
    };
  }, [onCancel, live]);

  const title = ask.href ? t("notes.editLink") : t("notes.addLink");
  return (
    <Portal>
      <form
        ref={box}
        className="link-box popover"
        role="dialog"
        aria-label={title}
        style={{ left: pos.left, top: pos.top, width: pos.width }}
        onSubmit={(e) => {
          e.preventDefault();
          const href = linkAddress(address);
          if (!href) {
            setWrong(true);
            return;
          }
          onDone(href, text);
        }}
      >
        <div className="search-field link-box-field">
          <Link size={14} strokeWidth={2} aria-hidden="true" />
          <input
            ref={field}
            autoFocus
            aria-label={t("notes.linkTo")}
            value={address}
            inputMode="url"
            spellCheck={false}
            autoCapitalize="off"
            autoComplete="off"
            placeholder={t("notes.linkPlaceholder")}
            aria-invalid={wrong}
            aria-describedby={wrong ? "link-box-wrong" : undefined}
            onChange={(e) => {
              setAddress(e.target.value);
              setWrong(false);
            }}
          />
          {ask.href && (
            <button type="button" className="icon-button small" aria-label={t("notes.removeLink")} title={t("notes.removeLink")} onClick={onRemove}>
              <Unlink size={14} strokeWidth={2} aria-hidden="true" />
            </button>
          )}
        </div>
        {ask.needsText && (
          <div className="search-field link-box-field">
            <Type size={14} strokeWidth={2} aria-hidden="true" />
            <input aria-label={t("notes.linkText")} value={text} placeholder={t("notes.linkTextPlaceholder")} onChange={(e) => setText(e.target.value)} />
          </div>
        )}
        {wrong && (
          <p id="link-box-wrong" className="link-box-wrong" role="alert">
            {t("notes.linkWrong")}
          </p>
        )}
        {/* Return in either field adds it, as in Notes; Escape leaves. */}
        <button type="submit" hidden aria-hidden="true" tabIndex={-1} />
      </form>
    </Portal>
  );
}
