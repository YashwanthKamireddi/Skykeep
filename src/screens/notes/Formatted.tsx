/**
 * A note as Notes sets it: titles and headings, bulleted, dashed and numbered lists, checklists
 * with their circles, monospaced blocks, and bold, italic, underlined, struck and linked text —
 * read from Apple's own attribute runs, so what a phone or icloud.com formatted shows here
 * the same.
 *
 * A run's length counts UTF-16 units, which is how a JavaScript string counts, so the runs
 * lay over the text directly. Runs that do not add up to the text — a note edited here since
 * the last sync — are not guessed at: the caller shows the plain text instead.
 */
import { useEffect, useRef } from "react";
import { describe, drawMedia } from "./NoteEditor";
import { addressesIn, openLinkAt } from "./links";
import { listNumbers } from "./rich";
import { t } from "../../lib/i18n";
import { refusedLink, linkedNote, outsideHref } from "../../lib/links";
export type Run = {
  length: number;
  style: number | null;
  indent: number;
  checked: boolean | null;
  /** Where a numbered list starts, when not 1: set on its first item. */
  start?: number | null;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
  link: string | null;
  /** Text colour set on a phone: 1 purple, 2 pink, 3 orange, 4 mint, 5 blue. */
  emphasis?: number | null;
  /** The attachment this run's character stands for — a table, a picture — by id. */
  attachment?: string | null;
  /** A block quote line (Format › Block Quote). */
  quote?: boolean;
};

/** Apple's paragraph styles. */
export const STYLE = {
  title: 0,
  heading: 1,
  subheading: 2,
  body: 3,
  mono: 4,
  bulleted: 100,
  dashed: 101,
  numbered: 102,
  checklist: 103,
} as const;

/**
 * The runs with the paragraph beginning at `start` set to `style` — split at its edges, as
 * the write to Apple splits them — so a change shows the moment it is chosen.
 */
export function restyle(text: string, runs: Run[], start: number, style: number): Run[] {
  const nl = text.indexOf("\n", start);
  const end = nl < 0 ? text.length : nl + 1;
  const out: Run[] = [];
  let at = 0;
  for (const run of runs) {
    const from = at;
    const to = at + run.length;
    at = to;
    const cuts = [from, ...[start, end].filter((c) => c > from && c < to), to];
    for (let i = 0; i + 1 < cuts.length; i++) {
      const a = cuts[i], b = cuts[i + 1];
      const inside = a >= start && b <= end;
      out.push({
        ...run,
        length: b - a,
        ...(inside ? { style, checked: style === STYLE.checklist ? (run.style === STYLE.checklist ? run.checked : false) : null } : {}),
      });
    }
  }
  return out;
}

/** Whether `runs` describe exactly this text. */
export function runsFit(text: string, runs: Run[]): boolean {
  return runs.length > 0 && runs.reduce((n, r) => n + r.length, 0) === text.length;
}

type Span = { text: string; run: Run };
type Paragraph = { spans: Span[]; run: Run; start: number };

/** The text cut into paragraphs, each with the style of the run its line ends in. */
function paragraphs(text: string, runs: Run[]): Paragraph[] {
  const out: Paragraph[] = [];
  let current: Paragraph = { spans: [], run: runs[0], start: 0 };
  let at = 0;
  for (const run of runs) {
    let piece = text.slice(at, at + run.length);
    let offset = at;
    at += run.length;
    while (piece.length > 0) {
      const nl = piece.indexOf("\n");
      const part = nl < 0 ? piece : piece.slice(0, nl);
      if (part) current.spans.push({ text: part, run });
      if (nl < 0) {
        current.run = run;
        break;
      }
      // The newline carries the paragraph's style, as Apple stores it.
      current.run = run;
      out.push(current);
      offset += nl + 1;
      current = { spans: [], run, start: offset };
      piece = piece.slice(nl + 1);
    }
  }
  if (current.spans.length > 0 || out.length === 0) out.push(current);
  return out;
}

/** Text with its web addresses made links, as the editor draws them (notes/links.ts). */
function withAddresses(text: string): React.ReactNode {
  const found = addressesIn(text);
  if (found.length === 0) return text;
  const out: React.ReactNode[] = [];
  let at = 0;
  for (const a of found) {
    if (a.from > at) out.push(text.slice(at, a.from));
    out.push(
      <a key={a.from} className="nf-autolink" href={a.href} rel="noreferrer" onClick={(e) => { e.stopPropagation(); openLinkAt(e.nativeEvent); }}>
        {text.slice(a.from, a.to)}
      </a>,
    );
    at = a.to;
  }
  if (at < text.length) out.push(text.slice(at));
  return out;
}

/**
 * An attachment where it stands in a note shown read-only: drawn as the editor draws it — the
 * picture, the scan, the file's chip — not an invisible character on an empty line.
 */
function Attachment({ id }: { id: string }) {
  const host = useRef<HTMLSpanElement | null>(null);
  useEffect(() => {
    let gone = false;
    void describe(id).then((info) => {
      if (!gone && info && host.current) drawMedia(host.current, info);
    });
    return () => {
      gone = true;
    };
  }, [id]);
  return <span ref={host} className="nf-attachment" />;
}

function Inline({ span }: { span: Span }) {
  const { run, text } = span;
  if (run.attachment && text.includes("\ufffc")) return <Attachment id={run.attachment} />;
  let node: React.ReactNode = run.link ? text : withAddresses(text);
  if (run.bold) node = <strong>{node}</strong>;
  if (run.italic) node = <em>{node}</em>;
  if (run.underline) node = <u>{node}</u>;
  if (run.strike) node = <s>{node}</s>;
  if (run.emphasis) node = <span className={`nf-em-${run.emphasis}`}>{node}</span>;
  if (run.link && (outsideHref(run.link) || linkedNote(run.link))) {
    node = (
      <a href={run.link} rel="noreferrer" onClick={(e) => { e.stopPropagation(); openLinkAt(e.nativeEvent); }}>
        {node}
      </a>
    );
  } else if (run.link) {
    // A link to something Skykeep does not open (a file on a share, a program's own scheme)
    // is its text, saying why it is not a link.
    node = <span className="nf-link-refused" title={refusedLink()}>{node}</span>;
  }
  return <>{node}</>;
}

/**
 * The body below the title. `skip` is how much of the text the title line takes, so the
 * runs still line up with what is drawn.
 */
export function FormattedBody({
  text,
  runs,
  skip,
  onTick,
}: {
  text: string;
  runs: Run[];
  skip: number;
  onTick?: (paragraphStart: number, checked: boolean) => void;
}) {
  const all = paragraphs(text, runs).filter((p) => p.start >= skip);
  // Counted as the editor counts them: a nested list on its own, the outer one carrying on.
  const numbers = listNumbers(all.map((p) => ({ style: p.run?.style ?? STYLE.body, indent: p.run?.indent ?? 0, start: p.run?.start ?? null })));
  return (
    <div className="note-formatted">
      {all.map((p, i) => {
        const style = p.run?.style ?? STYLE.body;
        const indent = { paddingInlineStart: `${(p.run?.indent ?? 0) * 1.5}em` };
        const at = { "data-start": p.start, ...(p.run?.quote ? { "data-quote": "true" } : {}) } as Record<string, number | string>;
        const content = p.spans.length ? p.spans.map((s, j) => <Inline key={j} span={s} />) : <br />;
        if (style === STYLE.title) return <h2 key={i} className="nf-title" style={indent} {...at}>{content}</h2>;
        if (style === STYLE.heading) return <h3 key={i} className="nf-heading" style={indent} {...at}>{content}</h3>;
        if (style === STYLE.subheading) return <h4 key={i} className="nf-subheading" style={indent} {...at}>{content}</h4>;
        if (style === STYLE.mono) return <pre key={i} className="nf-mono" style={indent} {...at}>{content}</pre>;
        if (style === STYLE.checklist) {
          const done = Boolean(p.run?.checked);
          return (
            <p key={i} className={`nf-item nf-check${done ? " is-done" : ""}`} style={indent} {...at}>
              <button
                type="button"
                className="nf-circle"
                role="checkbox"
                aria-checked={done}
                aria-label={p.spans.map((s) => s.text).join("") || t("notes.checklistItem")}
                disabled={!onTick}
                onClick={(e) => {
                  e.stopPropagation();
                  onTick?.(p.start, !done);
                }}
              />
              <span>{content}</span>
            </p>
          );
        }
        if (style === STYLE.bulleted || style === STYLE.dashed || style === STYLE.numbered) {
          const mark = style === STYLE.bulleted ? "•" : style === STYLE.dashed ? "–" : `${numbers[i]}.`;
          return (
            <p key={i} className="nf-item" style={indent} {...at}>
              <span className="nf-mark" aria-hidden="true">{mark}</span>
              <span>{content}</span>
            </p>
          );
        }
        return <p key={i} className="nf-body" style={indent} {...at}>{content}</p>;
      })}
    </div>
  );
}
