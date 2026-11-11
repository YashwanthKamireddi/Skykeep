import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useActiveRef } from "../../lib/screen";
import { Check, Link } from "lucide-react";
import { inMenu, Portal } from "../../components/Portal";
import type { EditorStatus } from "./NoteEditor";
import { STYLE } from "./rich";
import { useI18n, type Key } from "../../lib/i18n";

/** The styles, in icloud.com's order, each drawn the way it sets a line. */
const STYLES: { style: number | null; label: Key; className: string; mark?: string }[] = [
  { style: STYLE.title, label: "notes.styleTitle", className: "fp-title" },
  { style: STYLE.heading, label: "notes.styleHeading", className: "fp-heading" },
  { style: STYLE.subheading, label: "notes.styleSubheading", className: "fp-subheading" },
  { style: null, label: "notes.styleBody", className: "fp-body" },
  { style: STYLE.mono, label: "notes.styleMono", className: "fp-mono" },
  { style: STYLE.bulleted, label: "notes.styleBulleted", className: "fp-list", mark: "•" },
  { style: STYLE.dashed, label: "notes.styleDashed", className: "fp-list", mark: "-" },
  { style: STYLE.numbered, label: "notes.styleNumbered", className: "fp-list", mark: "1." },
];

/** The phone's line tools, drawn as Notes draws them: lines of text with a triangle for
 * indenting, an arrow to a bar for moving, a bar beside a block for quoting, dotted lines for
 * a list. 22 units square, stroked in the text colour. */
const LineGlyph = ({ d, fill }: { d: string; fill?: string }) => (
  <svg width="20" height="20" viewBox="0 0 22 22" aria-hidden="true" fill="none" stroke="currentColor"
    strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
    <path d={d} />
    {fill && <path d={fill} fill="currentColor" stroke="none" />}
  </svg>
);
const GLYPHS = {
  outdent: <LineGlyph d="M3 5h11M3 9h8M3 13h8M3 17h11" fill="M19 8.2v5.6L15 11z" />,
  indent: <LineGlyph d="M8 5h11M11 9h8M11 13h8M8 17h11" fill="M3 8.2v5.6L7 11z" />,
  up: <LineGlyph d="M5 3.5h12M11 19V7.5M6.5 12L11 7.5l4.5 4.5" />,
  down: <LineGlyph d="M5 18.5h12M11 3v11.5M6.5 10l4.5 4.5 4.5-4.5" />,
  quote: <LineGlyph d="M4.5 3.5v15" fill="M9 6.5a2 2 0 0 1 2-2h6a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-6a2 2 0 0 1-2-2z" />,
  list: <LineGlyph d="M9 6h10M9 11h10M9 16h10" fill="M3.4 6a1.4 1.4 0 1 0 2.8 0a1.4 1.4 0 1 0-2.8 0M3.4 11a1.4 1.4 0 1 0 2.8 0a1.4 1.4 0 1 0-2.8 0M3.4 16a1.4 1.4 0 1 0 2.8 0a1.4 1.4 0 1 0-2.8 0" />,
};

/** Notes' text colours, in the phone's order; the number is what Apple stores. */
const COLOURS = [
  { colour: 1, label: "notes.purple" },
  { colour: 2, label: "notes.pink" },
  { colour: 3, label: "notes.orange" },
  { colour: 4, label: "notes.mint" },
  { colour: 5, label: "notes.blue" },
] as const;

const MARKS = [
  { mark: "bold", label: "notes.bold", glyph: <b>B</b> },
  { mark: "italic", label: "notes.italic", glyph: <i>I</i> },
  { mark: "underline", label: "notes.underline", glyph: <u>U</u> },
  { mark: "strike", label: "notes.strikethrough", glyph: <s>S</s> },
] as const;

/**
 * Notes' Aa panel: bold, italic, underline and strikethrough across the top, the paragraph
 * styles beneath with the current one ticked — as icloud.com and the Mac lay it out — and the
 * phone's line tools (indent, outdent, move up, move down) and text colours. It acts on the
 * note without taking the cursor out of it, so a style is chosen and typing goes on.
 */
export function FormatPanel({
  at,
  status,
  onClose,
  onStyle,
  onMark,
  onQuote,
  onColour,
  onIndent,
  onMove,
  onLink,
}: {
  at: { x: number; y: number };
  status: EditorStatus | null;
  onClose: () => void;
  onStyle: (style: number | null) => void;
  onMark: (mark: "bold" | "italic" | "underline" | "strike") => void;
  onQuote: () => void;
  onColour: (colour: number | null) => void;
  onIndent: (by: 1 | -1) => void;
  onMove: (by: 1 | -1) => void;
  /** Add Link, or edit the link the cursor is in. */
  onLink: () => void;
}) {
  const { t } = useI18n();
  const box = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState({ left: at.x, top: at.y });
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const left = Math.max(8, Math.min(at.x - width / 2, window.innerWidth - width - 8));
    const top = at.y + height > window.innerHeight - 8 ? Math.max(8, window.innerHeight - height - 8) : at.y;
    setPos({ left, top });
  }, [at]);
  const live = useActiveRef();
  useEffect(() => {
    const down = (e: MouseEvent) => {
      const t = e.target as HTMLElement;
      if (!box.current?.contains(t) && !t.closest(".notes-aa") && !inMenu(t)) onClose();
    };
    const key = (e: KeyboardEvent) => live.current && e.key === "Escape" && onClose();
    document.addEventListener("mousedown", down);
    window.addEventListener("keydown", key);
    window.addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("mousedown", down);
      window.removeEventListener("keydown", key);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  const current = status?.style ?? null;
  const isCurrent = (style: number | null) =>
    style === null ? current === null || current === STYLE.body : current === style;

  return (
    <Portal>
      <div
        ref={box}
        className="format-panel"
        role="dialog"
        aria-label={t("notes.format")}
        style={{ left: pos.left, top: pos.top }}
        // Clicks here never take focus from the note.
        onMouseDown={(e) => e.preventDefault()}
      >
        <div className="fp-marks" role="group" aria-label={t("notes.textStyle")}>
          {MARKS.map(({ mark, label, glyph }) => (
            <button
              key={mark}
              type="button"
              className="fp-mark"
              aria-label={t(label)}
              aria-pressed={Boolean(status?.[mark])}
              title={t(label)}
              onClick={() => onMark(mark)}
            >
              {glyph}
            </button>
          ))}
          <button
            type="button"
            className="fp-mark fp-link"
            aria-label={status?.link ? t("notes.editLink") : t("notes.addLink")}
            aria-pressed={Boolean(status?.link)}
            title={`${status?.link ? t("notes.editLink") : t("notes.addLink")} (Ctrl+K)`}
            onClick={() => {
              onClose();
              onLink();
            }}
          >
            <Link size={15} strokeWidth={2} aria-hidden="true" />
          </button>
        </div>
        <div className="fp-marks fp-lines" role="group" aria-label={t("notes.line")}>
          {[
            { label: t("notes.decreaseIndent"), glyph: GLYPHS.outdent, run: () => onIndent(-1) },
            { label: t("notes.increaseIndent"), glyph: GLYPHS.indent, run: () => onIndent(1) },
            { label: t("notes.moveLineUp"), glyph: GLYPHS.up, run: () => onMove(-1) },
            { label: t("notes.moveLineDown"), glyph: GLYPHS.down, run: () => onMove(1) },
            { label: t("notes.blockQuote"), glyph: GLYPHS.quote, run: onQuote, on: Boolean(status?.quote) },
            { label: t("notes.styleBulleted"), glyph: GLYPHS.list, run: () => onStyle(status?.style === STYLE.bulleted ? null : STYLE.bulleted), on: status?.style === STYLE.bulleted },
          ].map(({ label, glyph, run, on }) => (
            <button key={label} type="button" className="fp-mark" aria-label={label} title={label} aria-pressed={on} onClick={run}>
              {glyph}
            </button>
          ))}
        </div>
        <div className="fp-colours" role="radiogroup" aria-label={t("notes.textColour")}>
          <button
            type="button"
            role="radio"
            aria-checked={!status?.colour}
            aria-label={t("notes.noColour")}
            title={t("common.none")}
            className="fp-colour is-none"
            onClick={() => onColour(null)}
          />
          {COLOURS.map(({ colour, label }) => (
            <button
              key={colour}
              type="button"
              role="radio"
              aria-checked={status?.colour === colour}
              aria-label={t(label)}
              title={t(label)}
              className="fp-colour"
              style={{ "--swatch": `var(--note-c${colour})` } as React.CSSProperties}
              onClick={() => onColour(colour)}
            />
          ))}
        </div>
        <div className="fp-styles" role="menu" aria-label={t("notes.paragraphStyle")}>
          {STYLES.map(({ style, label, className, mark }) => (
            <button
              key={label}
              type="button"
              role="menuitemradio"
              aria-checked={isCurrent(style)}
              className={`fp-style ${className}`}
              onClick={() => {
                onStyle(style);
                onClose();
              }}
            >
              <span className="fp-check" aria-hidden="true">{isCurrent(style) && <Check size={15} strokeWidth={2} />}</span>
              {mark && <span className="fp-list-mark" aria-hidden="true">{mark}</span>}
              <span>{t(label)}</span>
            </button>
          ))}
        </div>
      </div>
    </Portal>
  );
}
