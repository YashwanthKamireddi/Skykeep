/**
 * Every app's keys on one sheet (Ctrl+/), grouped by app and laid out as a Mac app's Help
 * lists them: what it does on the left, the keys on the right. The app in front comes first.
 * The same list as the README's, kept here as data so the two are read side by side.
 */
import { useRef } from "react";
import { Portal } from "./Portal";
import { useModalFocus } from "../lib/focus";

type Row = [keys: string[][], action: string];
export const SHORTCUTS: { app: string; key: string; rows: Row[] }[] = [
  {
    app: "Everywhere",
    key: "",
    rows: [
      [[["Ctrl", "R"], ["F5"]], "Sync the app in front"],
      [[["Ctrl", ","]], "Settings"],
      [[["Ctrl", "/"]], "Keyboard shortcuts"],
    ],
  },
  {
    app: "Notes",
    key: "notes",
    rows: [
      [[["Ctrl", "N"], ["Ctrl", "Shift", "N"]], "New note / new folder"],
      [[["Ctrl", "F"], ["Ctrl", "Shift", "F"]], "Search this folder / all notes"],
      [[["↑"], ["↓"]], "Previous / next note"],
      [[["Ctrl", "Delete"]], "Delete note"],
      [[["Ctrl", "S"]], "Finish editing"],
      [[["Ctrl", "Shift", "L"], ["Ctrl", "Shift", "U"]], "Checklist / tick the item"],
    ],
  },
  {
    app: "Reminders",
    key: "reminders",
    rows: [
      [[["Ctrl", "N"], ["Ctrl", "Shift", "N"]], "New reminder / new list"],
      [[["Tab"], ["Shift", "Tab"]], "Make a subtask / move it back out"],
      [[["Space"], ["Return"]], "Tick / edit"],
      [[["Ctrl", "I"]], "Show details"],
      [[["Ctrl", "Shift", "F"], ["Ctrl", "Shift", "C"]], "Flag / complete"],
      [[["Ctrl", "T"]], "Due today"],
      [[["Ctrl", "Shift", "H"]], "Show or hide completed"],
      [[["Ctrl", "E"], ["Ctrl", "Shift", "E"]], "Show / hide all subtasks"],
      [[["Ctrl", "F"]], "Search"],
    ],
  },
  {
    app: "Calendar",
    key: "calendar",
    rows: [
      [[["Ctrl", "1–4"]], "Day, Week, Month, Year"],
      [[["T"]], "Today"],
      [[["Ctrl", "Shift", "T"]], "Go to Date"],
      [[["←"], ["→"]], "Previous / next"],
      [[["Ctrl", "N"]], "New event"],
      [[["Ctrl", "D"], ["Ctrl", "C"], ["Ctrl", "V"]], "Duplicate / copy / paste event"],
      [[["Ctrl", "Alt", "Arrows"]], "Nudge the selected event"],
      [[["Ctrl", "Z"]], "Undo the last change or deletion"],
    ],
  },
  {
    app: "Contacts",
    key: "contacts",
    rows: [
      [[["Ctrl", "N"]], "New contact"],
      [[["Ctrl", "Shift", "N"]], "New list"],
      [[["Ctrl", "L"], ["Return"]], "Edit contact"],
      [[["Delete"]], "Delete contact"],
      [[["Ctrl", "F"]], "Search"],
      [[["Ctrl", "A"]], "Select all"],
    ],
  },
  {
    app: "Drive",
    key: "drivews",
    rows: [
      [[["Return"], ["Ctrl", "↓"]], "Open"],
      [[["Ctrl", "↑"]], "Enclosing folder"],
      [[["Space"]], "Quick preview"],
      [[["F2"], ["Delete"]], "Rename / move to Recently Deleted"],
    ],
  },
  {
    app: "Photos",
    key: "photos",
    rows: [
      [[["←"], ["→"]], "Previous / next"],
      [[["."], ["F"]], "Favourite"],
      [[["I"], ["Z"]], "Info / zoom to fit or full size"],
      [[["Ctrl", "+"], ["Ctrl", "-"], ["Ctrl", "0"]], "Zoom in / out / reset"],
      [[["Space"]], "Play a video or Live Photo"],
      [[["Esc"]], "Close the viewer"],
    ],
  },
];

export function ShortcutsSheet({ front, onClose }: { front: string | null; onClose: () => void }) {
  const sheetRef = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheetRef);
  const groups = [...SHORTCUTS].sort((a, b) => Number(b.key === front && !!front) - Number(a.key === front && !!front));
  return (
    <Portal>
      <div className="modal-veil" onClick={onClose}>
        <div
          ref={sheetRef}
          className="modal-sheet shortcuts-sheet"
          role="dialog"
          aria-modal="true"
          aria-labelledby="shortcuts-title"
          onClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              onClose();
            }
          }}
        >
          <h2 id="shortcuts-title">Keyboard Shortcuts</h2>
          <div className="shortcuts-groups" tabIndex={0} aria-label="Shortcuts by app">
            {groups.map((g) => (
              <section key={g.app} aria-labelledby={`shortcuts-${g.app}`}>
                <h3 id={`shortcuts-${g.app}`}>{g.app}</h3>
                <dl>
                  {g.rows.map(([keys, action]) => (
                    <div key={action} className="shortcuts-row">
                      <dt>{action}</dt>
                      <dd>
                        {keys.map((combo, i) => (
                          <span key={i} className="shortcuts-combo">
                            {combo.map((k) => (
                              <kbd key={k}>{k}</kbd>
                            ))}
                          </span>
                        ))}
                      </dd>
                    </div>
                  ))}
                </dl>
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
