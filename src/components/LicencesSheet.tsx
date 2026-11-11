/**
 * Open-Source Licences, from Settings: every licence the app ships, grouped by licence, each
 * text with what is shipped under it and the text itself unfolded on a click. The list is the
 * one `npm run licences` wrote from Cargo.lock and package-lock.json (scripts/licences.mjs), never
 * typed here, and loaded only with this sheet: its texts are some 400 kB.
 */
import { useMemo, useRef, useState } from "react";
import list from "../licences.json";
import { Portal } from "./Portal";
import { Glyph } from "./Glyph";
import { useModalFocus } from "../lib/focus";

/** One licence text and everything shipped under it, each as "name version". */
type Licence = { id: string; name: string; users: string[]; text: string };
const licences: Licence[] = list;

/** The licences under one name (MIT License, ISC License), each text once. */
function byName(all: Licence[]): { name: string; texts: Licence[] }[] {
  const groups = new Map<string, Licence[]>();
  for (const l of all) groups.set(l.name, [...(groups.get(l.name) ?? []), l]);
  return [...groups].map(([name, texts]) => ({ name, texts }));
}

export function LicencesSheet({ onClose }: { onClose: () => void }) {
  const sheetRef = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheetRef);
  const groups = useMemo(() => byName(licences), []);
  // One text unfolded at a time: some run to hundreds of lines, and opening the next folds the
  // last away, so the list stays short enough to scan.
  const [open, setOpen] = useState<string | null>(null);
  return (
    <Portal>
      <div className="modal-veil" onClick={onClose}>
        <div
          ref={sheetRef}
          className="modal-sheet licences-sheet"
          role="dialog"
          aria-modal="true"
          aria-labelledby="licences-title"
          onClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              onClose();
            }
          }}
        >
          <h2 id="licences-title">Open-Source Licences</h2>
          <p className="licences-intro">
            Skykeep is free software under the GPL-3.0. It is built on the work below, each under its own licence.
          </p>
          <div className="licences-list" tabIndex={0} aria-label="Licences">
            {groups.map((g, n) => (
              <section key={g.name} aria-labelledby={`licences-${n}`}>
                <h3 id={`licences-${n}`}>{g.name}</h3>
                <ul>
                  {g.texts.map((l, i) => {
                    const at = `${g.name}:${i}`;
                    const unfolded = open === at;
                    return (
                      <li key={at}>
                        <button
                          className="licences-row"
                          aria-expanded={unfolded}
                          onClick={() => setOpen(unfolded ? null : at)}
                        >
                          <Glyph name="chevron" className="licences-chevron" />
                          <span className="licences-users">{l.users.join(", ")}</span>
                        </button>
                        {unfolded && <pre className="licences-text">{l.text.trim()}</pre>}
                      </li>
                    );
                  })}
                </ul>
              </section>
            ))}
          </div>
          <div className="sheet-actions">
            <button className="primary" autoFocus onClick={onClose}>
              Done
            </button>
          </div>
        </div>
      </div>
    </Portal>
  );
}
