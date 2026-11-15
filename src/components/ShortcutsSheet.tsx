/**
 * Every app's keys on one sheet (Ctrl+/), grouped by app and laid out as a Mac app's Help
 * lists them: what it does on the left, the keys on the right. The app in front comes first.
 * The same list as the README's, kept here as data so the two are read side by side.
 */
import { useRef } from "react";
import { Portal } from "./Portal";
import { useModalFocus } from "../lib/focus";

/** One action and its keys: each combination pressed together; more than one, either works. */
type Row = [keys: string[][], action: string];
export const SHORTCUTS: { app: string; key: string; rows: Row[] }[] = [
  {
    app: "Everywhere",
    key: "",
    rows: [
      [[["Ctrl", "R"], ["F5"]], "Sync the app in front"],
      [[["Ctrl", ","]], "Settings"],
      [[["F9"]], "Show or hide the sidebar"],
      [[["Ctrl", "/"]], "Keyboard shortcuts"],
    ],
  },
  {
    app: "Notes",
    key: "notes",
    rows: [
      [[["Ctrl", "N"]], "New note"],
      [[["Ctrl", "Shift", "N"]], "New folder"],
      [[["Ctrl", "F"]], "Search this folder"],
      [[["Ctrl", "Shift", "F"]], "Search all notes"],
      [[["↑"]], "Previous note"],
      [[["↓"]], "Next note"],
      [[["Ctrl", "Delete"]], "Delete note"],
      [[["Ctrl", "S"]], "Finish editing"],
      [[["Ctrl", "K"]], "Add or edit a link"],
      [[["Ctrl", "Shift", "L"]], "Checklist"],
      [[["Ctrl", "Shift", "U"]], "Tick the item"],
    ],
  },
  {
    app: "Reminders",
    key: "reminders",
    rows: [
      [[["Ctrl", "N"]], "New reminder"],
      [[["Ctrl", "Shift", "N"]], "New list"],
      [[["Tab"]], "Make a subtask"],
      [[["Shift", "Tab"]], "Move a subtask back out"],
      [[["Space"]], "Tick"],
      [[["Return"]], "Edit"],
      [[["Ctrl", "I"]], "Show details"],
      [[["Ctrl", "Shift", "F"]], "Flag"],
      [[["Ctrl", "Shift", "C"]], "Complete"],
      [[["Ctrl", "T"]], "Due today"],
      [[["Ctrl", "Shift", "H"]], "Show or hide completed"],
      [[["Ctrl", "E"]], "Show all subtasks"],
      [[["Ctrl", "Shift", "E"]], "Hide all subtasks"],
      [[["Ctrl", "F"]], "Search"],
    ],
  },
  {
    app: "Calendar",
    key: "calendar",
    rows: [
      [[["Ctrl", "1"]], "Day"],
      [[["Ctrl", "2"]], "Week"],
      [[["Ctrl", "3"]], "Month"],
      [[["Ctrl", "4"]], "Year"],
      [[["T"]], "Today"],
      [[["Ctrl", "Shift", "T"]], "Go to Date"],
      [[["←"]], "Previous"],
      [[["→"]], "Next"],
      [[["Ctrl", "N"]], "New event"],
      [[["Ctrl", "D"]], "Duplicate event"],
      [[["Ctrl", "C"]], "Copy event"],
      [[["Ctrl", "V"]], "Paste event"],
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
      [[["F2"]], "Rename"],
      [[["Delete"]], "Move to Recently Deleted"],
    ],
  },
  {
    app: "Photos",
    key: "photos",
    rows: [
      [[["←"]], "Previous"],
      [[["→"]], "Next"],
      [[["."], ["F"]], "Favourite"],
      [[["I"]], "Info"],
      [[["Z"]], "Zoom to fit or full size"],
      [[["Ctrl", "+"]], "Zoom in"],
      [[["Ctrl", "-"]], "Zoom out"],
      [[["Ctrl", "0"]], "Reset zoom"],
      [[["Space"]], "Play a video or Live Photo"],
      [[["Esc"]], "Close the viewer"],
    ],
  },
];

export function ShortcutsSheet({ front, onClose }: { front: string | null; onClose: () => void }) {
  const sheetRef = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheetRef);
  // An app's own window shows its own keys and those that work everywhere, nothing of the
  // other five apps (each is a window of its own); Skykeep's window, the overview, shows all.
  const app = SHORTCUTS.find((g) => g.key !== "" && g.key === front);
  const groups = app ? [app, ...SHORTCUTS.filter((g) => g.key === "")] : SHORTCUTS;
  return (
    <Portal>
      <div className="modal-veil" onClick={onClose}>
        <div
          ref={sheetRef}
          className={`modal-sheet shortcuts-sheet${app ? " is-one" : ""}`}
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
                          <span key={i} className="shortcuts-either">
                            {i > 0 && <span className="shortcuts-or">or</span>}
                            {/* A combination as one cap, its keys joined as they are pressed. */}
                            <kbd className="shortcuts-combo" aria-label={combo.join(" + ")}>
                              {combo.map((k, j) => (
                                <span key={k}>
                                  {j > 0 && <span className="shortcuts-plus" aria-hidden="true">+</span>}
                                  {k}
                                </span>
                              ))}
                            </kbd>
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
