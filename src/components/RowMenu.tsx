import { Portal } from "./Portal";
import { useActiveRef } from "../lib/screen";
import { useReturnFocus } from "../lib/focus";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

/**
 * One line of a menu, or `null` for a separator. An entry with `submenu` leads on to a menu
 * of its own, opened beside it as macOS opens one; `keep` leaves the menu open after `run`.
 */
export type MenuEntry =
  | {
      label: string;
      run?: () => void;
      danger?: boolean;
      keep?: boolean;
      submenu?: MenuEntry[];
      /** A choice that is on or off: a check beside it when on, as a Mac menu marks one. */
      checked?: boolean;
      /** A short second line under the name, smaller and quieter: what the choice does,
       *  where its name alone could be read two ways. */
      detail?: string;
    }
  | null;

type Anchor = { x: number; y: number; alignRight: boolean };

/**
 * Type-select, as every Mac menu and list has it: letters typed in quick succession go to the
 * line that begins with them; the same letter again steps on through the lines that begin
 * with it. A pause (a second) starts afresh.
 *
 * Returns a function that takes a key and the line now lit, and gives the line to light, or
 * null when the key is not typing (or nothing matches).
 */
export function useTypeSelect(labels: () => string[]) {
  const typed = useRef({ text: "", at: 0 });
  return (e: { key: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean }, from: number): number | null => {
    if (e.key.length !== 1 || e.ctrlKey || e.metaKey || e.altKey) return null;
    const now = performance.now();
    const fresh = now - typed.current.at > 1000;
    // A space begins nothing; mid-word it is part of what is typed.
    if (e.key === " " && fresh) return null;
    typed.current = { text: (fresh ? "" : typed.current.text) + e.key.toLowerCase(), at: now };
    const text = typed.current.text;
    const names = labels().map((l) => l.toLowerCase());
    const n = names.length;
    // One letter, again and again: the next line after this one that begins with it.
    const repeat = [...text].every((c) => c === text[0]);
    const want = repeat ? text[0] : text;
    const start = repeat ? from + 1 : Math.max(0, from);
    for (let k = 0; k < n; k++) {
      const i = (start + k + n) % n;
      if (names[i].startsWith(want)) return i;
    }
    return null;
  };
}

/**
 * A menu as macOS draws one: under the ⋯ that opened it or at the pointer for a right-click,
 * kept on screen; arrows to move, Return to choose, Escape or a click elsewhere to leave.
 * A line with a submenu shows ▸ and opens it beside itself on hover, Return or →, to the
 * right where there is room and the left where there is not; ← or Escape closes it again.
 * Choosing anything, at any depth, closes every level.
 */
export function RowMenu({ at, entries, onClose }: { at: Anchor; entries: MenuEntry[]; onClose: () => void }) {
  const live = useActiveRef();
  // Closed with the focus still in it (Escape, or a choice that opens nothing), the focus goes
  // back to what opened it — its ⋯, or the row a menu key opened it from. Left on the menu that
  // went, it fell to the page and the keys began again from the top.
  useReturnFocus();
  useEffect(() => {
    // Any press outside every level of this menu dismisses it.
    const down = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      if (target?.closest?.(".row-menu")) return;
      onClose();
      // A press on a button that opens a menu — this one's ⋯, or another row's — only closes
      // the open menu, as on the Mac, where the click that dismisses a menu is used up by it.
      // Let through, it opened the menu again under the pointer: the menu blinked, and could
      // not be closed from the button that opened it.
      const opener = target?.closest?.('[aria-haspopup="menu"]');
      if (!opener) return;
      const swallow = (c: MouseEvent) => {
        if (opener.contains(c.target as Node)) {
          c.stopPropagation();
          c.preventDefault();
        }
      };
      document.addEventListener("click", swallow, { capture: true, once: true });
      window.setTimeout(() => document.removeEventListener("click", swallow, true), 700);
    };
    const away = () => onClose();
    // Escape closes the menu wherever the keys are: focus is not always in it — a submenu
    // that closed under the pointer took the focus it held with it.
    const key = (e: KeyboardEvent) => {
      if (!live.current) return;
      if (e.key !== "Escape" || (e.target as HTMLElement | null)?.closest?.(".row-menu")) return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", key, true);
    document.addEventListener("mousedown", down);
    window.addEventListener("blur", away);
    window.addEventListener("resize", away);
    return () => {
      window.removeEventListener("keydown", key, true);
      document.removeEventListener("mousedown", down);
      window.removeEventListener("blur", away);
      window.removeEventListener("resize", away);
    };
  }, [onClose]);
  return (
    <Portal>
      <Panel entries={entries} anchor={{ kind: "point", ...at }} onDone={onClose} onBack={onClose} focusFirst={false} />
    </Portal>
  );
}

type PanelAnchor =
  | ({ kind: "point" } & Anchor)
  /** Beside a parent menu's line: its rectangle. */
  | { kind: "beside"; rect: DOMRect };

function Panel({
  entries,
  anchor,
  onDone,
  onBack,
  focusFirst,
  onArrive,
  hostRef,
}: {
  entries: MenuEntry[];
  anchor: PanelAnchor;
  /** Something was chosen: every level closes. */
  onDone: () => void;
  /** This level closes, back to the one that opened it. */
  onBack: () => void;
  /** Opened by the keyboard: the first line is highlighted, as on the Mac. */
  focusFirst: boolean;
  /** The pointer came into this submenu. */
  onArrive?: () => void;
  /** This level's own box, for the level that opened it to aim at. */
  hostRef?: React.MutableRefObject<HTMLDivElement | null>;
}) {
  const box = useRef<HTMLDivElement | null>(null);
  const lines = useRef<(HTMLButtonElement | null)[]>([]);
  const items = entries.filter((e): e is NonNullable<MenuEntry> => e !== null);
  const checks = items.some((e) => e.checked !== undefined);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  // Nothing highlighted until the pointer or a key says so, as a Mac menu opens.
  const [active, setActive] = useState(focusFirst ? 0 : -1);
  const [sub, setSub] = useState<{ index: number; byKeyboard: boolean } | null>(null);
  const typeSelect = useTypeSelect(() => items.map((e) => e.label));
  /**
   * Menu aim, as macOS has it: with a submenu open, a pointer heading into it — inside the
   * triangle from where it just was to the submenu's near edge — may cross other lines
   * without closing it; the switch waits a moment and is called off when it arrives. Any
   * other move (back up the menu, away from the submenu) switches at once. Waiting on every
   * move left the menu looking stuck: the lines above did not light as the pointer crossed
   * them.
   */
  const aim = useRef<number | null>(null);
  const subBox = useRef<HTMLDivElement | null>(null);
  // Where the pointer last moved, within this level. A move fires a line's mouseenter before
  // its mousemove, so on entering a line this is still where the pointer came from.
  const last = useRef<{ x: number; y: number } | null>(null);
  const heading = (x: number, y: number) => {
    const from = last.current;
    const r = subBox.current?.getBoundingClientRect();
    if (!from || !r) return false;
    // The submenu's near edge: its left side when it opened to the right, else its right.
    const edge = r.left >= from.x ? r.left : r.right;
    // Only a move toward that side is heading there at all.
    if ((edge - from.x) * (x - from.x) <= 0) return false;
    // A hand does not move in straight lines: a short submenu — one album — is aimed at
    // with room above and below it, or a move that dips a few pixels past it closes it.
    const a = { x: edge, y: r.top - 48 };
    const c = { x: edge, y: r.bottom + 48 };
    const side = (p: { x: number; y: number }, q: { x: number; y: number }) => (q.x - p.x) * (y - p.y) - (q.y - p.y) * (x - p.x);
    const d1 = side(from, a);
    const d2 = side(a, c);
    const d3 = side(c, from);
    const neg = d1 < 0 || d2 < 0 || d3 < 0;
    const pos = d1 > 0 || d2 > 0 || d3 > 0;
    return !(neg && pos);
  };
  const stopAim = () => {
    if (aim.current !== null) window.clearTimeout(aim.current);
    aim.current = null;
  };
  useEffect(() => stopAim, []);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const pad = 8;
    let left: number;
    let top: number;
    if (anchor.kind === "point") {
      left = anchor.alignRight ? anchor.x - width : anchor.x;
      top = anchor.y;
      if (top + height > window.innerHeight - pad) top = anchor.y - height - (anchor.alignRight ? 36 : 0);
    } else {
      // Beside the line, its first item level with it; to the left when the right is full.
      const r = anchor.rect;
      left = r.right + 2 + width <= window.innerWidth - pad ? r.right + 2 : r.left - 2 - width;
      top = r.top - 5;
    }
    left = Math.max(pad, Math.min(left, window.innerWidth - width - pad));
    top = Math.max(pad, Math.min(top, window.innerHeight - height - pad));
    setPos({ left, top });
    // Placed once, where it opened.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // Focus once it is placed: drawn hidden while it is measured, and a hidden element cannot
  // take focus — the keys went to whatever had it before.
  const placed = pos !== null;
  useLayoutEffect(() => {
    if (placed) box.current?.focus({ preventScroll: true });
  }, [placed]);

  const choose = (i: number, byKeyboard: boolean) => {
    const item = items[i];
    if (!item) return;
    if (item.submenu) {
      setSub({ index: i, byKeyboard });
      return;
    }
    if (!item.keep) onDone();
    item.run?.();
  };

  let n = -1;
  const open = sub ? items[sub.index] : null;
  return (
    <>
      <div
        ref={(el) => {
          box.current = el;
          if (hostRef) hostRef.current = el;
        }}
        className="row-menu"
        role="menu"
        tabIndex={-1}
        style={pos ? { left: pos.left, top: pos.top } : { left: 0, top: 0, visibility: "hidden" }}
        onMouseEnter={onArrive}
        // The pointer gone off the menu takes its highlight with it, as on the Mac — unless a
        // submenu is open from the lit line, which stays lit while it is.
        onMouseLeave={() => {
          if (!sub) setActive(-1);
        }}
        onMouseMove={(e) => {
          last.current = { x: e.clientX, y: e.clientY };
        }}
        onKeyDown={(e) => {
          // Keys belong to the deepest open level.
          if (sub) return;
          const move = (by: number) => {
            e.preventDefault();
            setActive((a) => (a < 0 ? (by > 0 ? 0 : items.length - 1) : (a + by + items.length) % items.length));
          };
          const typedTo = typeSelect(e, active);
          if (typedTo !== null) {
            e.preventDefault();
            setActive(typedTo);
          } else if (e.key.length === 1 && e.key !== " ") {
            // A letter nothing begins with: used up here, as a Mac menu takes it.
            e.preventDefault();
          } else if (e.key === "Home" || e.key === "PageUp") {
            e.preventDefault();
            setActive(0);
          } else if (e.key === "End" || e.key === "PageDown") {
            e.preventDefault();
            setActive(items.length - 1);
          } else if (e.key === "Escape" || e.key === "ArrowLeft") {
            e.preventDefault();
            e.stopPropagation();
            onBack();
          } else if (e.key === "ArrowDown") move(1);
          else if (e.key === "ArrowUp") move(-1);
          else if ((e.key === "Enter" || e.key === " " || e.key === "ArrowRight") && active >= 0) {
            e.preventDefault();
            if (e.key === "ArrowRight" && !items[active]?.submenu) return;
            choose(active, true);
          }
        }}
      >
        {entries.map((entry, i) => {
          if (!entry) return <div key={`sep${i}`} className="row-menu-sep" role="separator" />;
          n += 1;
          const index = n;
          return (
            <button
              key={`${entry.label}-${index}`}
              ref={(el) => {
                lines.current[index] = el;
              }}
              role={entry.checked === undefined ? "menuitem" : "menuitemcheckbox"}
              aria-checked={entry.checked}
              tabIndex={-1}
              aria-haspopup={entry.submenu ? "menu" : undefined}
              aria-expanded={entry.submenu ? sub?.index === index : undefined}
              className={`row-menu-item${entry.danger ? " is-danger" : ""}${index === active ? " is-active" : ""}${entry.submenu ? " has-submenu" : ""}`}
              onMouseEnter={(e) => {
                const go = () => {
                  aim.current = null;
                  // The submenu going takes its focus with it: the keys come back here.
                  if (sub && !entry.submenu) box.current?.focus({ preventScroll: true });
                  setActive(index);
                  // Hovering a line with a submenu opens it; any other line closes one.
                  setSub(entry.submenu ? { index, byKeyboard: false } : null);
                };
                stopAim();
                if (sub && sub.index !== index && heading(e.clientX, e.clientY)) aim.current = window.setTimeout(go, 280);
                else go();
              }}
              onClick={() => choose(index, false)}
            >
              {/* A menu with choices in it keeps the column for the check on every line, so
                  the words line up whether a line is checked or not. */}
              {checks && (
                <span className="row-menu-check" aria-hidden="true">
                  {entry.checked ? "✓" : ""}
                </span>
              )}
              {entry.detail ? (
                <span className="row-menu-label row-menu-two">
                  <span>{entry.label}</span>
                  <span className="row-menu-detail">{entry.detail}</span>
                </span>
              ) : (
                <span className="row-menu-label">{entry.label}</span>
              )}
              {entry.submenu && <span className="row-menu-chevron" aria-hidden="true">›</span>}
            </button>
          );
        })}
      </div>
      {open?.submenu && sub && lines.current[sub.index] && (
        <Panel
          key={sub.index}
          entries={open.submenu}
          anchor={{ kind: "beside", rect: lines.current[sub.index]!.getBoundingClientRect() }}
          onDone={onDone}
          onBack={() => {
            setSub(null);
            box.current?.focus({ preventScroll: true });
          }}
          onArrive={() => {
            // The pointer made it: the line that opened this stays the highlighted one.
            stopAim();
            setActive(sub.index);
          }}
          focusFirst={sub.byKeyboard}
          hostRef={subBox}
        />
      )}
    </>
  );
}
