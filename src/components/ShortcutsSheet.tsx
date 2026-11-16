/**
 * Every app's keys on one sheet (Ctrl+/), grouped by app and laid out as a Mac app's Help
 * lists them: what it does on the left, the keys on the right. The app in front comes first.
 * The same list as the README's, kept here as data so the two are read side by side.
 */
import { useEffect, useRef, useState } from "react";
import { api } from "../lib/ipc";
import { t, useI18n, type Key } from "../lib/i18n";
import { Portal } from "./Portal";
import { useModalFocus } from "../lib/focus";

/** One action and its keys: each combination pressed together; more than one, either works. */
type Row = [keys: string[][], action: Key];
export const SHORTCUTS: { app: Key; key: string; rows: Row[] }[] = [
  {
    app: "shortcuts.apps.everywhere",
    key: "",
    rows: [
      [[["Ctrl", "R"], ["F5"]], "shortcuts.actions.syncTheAppInFront"],
      [[["Ctrl", ","]], "shortcuts.actions.settings"],
      [[["F9"]], "shortcuts.actions.showOrHideTheSidebar"],
      [[["Ctrl", "Shift", "P"]], "shortcuts.actions.floatOnTop"],
      [[["Ctrl", "/"]], "shortcuts.actions.keyboardShortcuts"],
    ],
  },
  {
    app: "shortcuts.apps.notes",
    key: "notes",
    rows: [
      [[["Ctrl", "N"]], "shortcuts.actions.newNote"],
      [[["Ctrl", "Shift", "N"]], "shortcuts.actions.newFolder"],
      [[["Ctrl", "F"]], "shortcuts.actions.searchThisFolder"],
      [[["Ctrl", "Shift", "F"]], "shortcuts.actions.searchAllNotes"],
      [[["↑"]], "shortcuts.actions.previousNote"],
      [[["↓"]], "shortcuts.actions.nextNote"],
      [[["Ctrl", "Delete"]], "shortcuts.actions.deleteNote"],
      [[["Ctrl", "S"]], "shortcuts.actions.finishEditing"],
      [[["Ctrl", "K"]], "shortcuts.actions.addOrEditALink"],
      [[["Ctrl", "Shift", "L"]], "shortcuts.actions.checklist"],
      [[["Ctrl", "Shift", "U"]], "shortcuts.actions.tickTheItem"],
    ],
  },
  {
    app: "shortcuts.apps.reminders",
    key: "reminders",
    rows: [
      [[["Ctrl", "N"]], "shortcuts.actions.newReminder"],
      [[["Ctrl", "Shift", "N"]], "shortcuts.actions.newList"],
      [[["Tab"]], "shortcuts.actions.makeASubtask"],
      [[["Shift", "Tab"]], "shortcuts.actions.moveASubtaskBackOut"],
      [[["Space"]], "shortcuts.actions.tick"],
      [[["Return"]], "shortcuts.actions.edit"],
      [[["Ctrl", "I"]], "shortcuts.actions.showDetails"],
      [[["Ctrl", "Shift", "F"]], "shortcuts.actions.flag"],
      [[["Ctrl", "Shift", "C"]], "shortcuts.actions.complete"],
      [[["Ctrl", "T"]], "shortcuts.actions.dueToday"],
      [[["Ctrl", "Shift", "H"]], "shortcuts.actions.showOrHideCompleted"],
      [[["Ctrl", "E"]], "shortcuts.actions.showAllSubtasks"],
      [[["Ctrl", "Shift", "E"]], "shortcuts.actions.hideAllSubtasks"],
      [[["Ctrl", "F"]], "shortcuts.actions.search"],
    ],
  },
  {
    app: "shortcuts.apps.calendar",
    key: "calendar",
    rows: [
      [[["Ctrl", "1"]], "shortcuts.actions.day"],
      [[["Ctrl", "2"]], "shortcuts.actions.week"],
      [[["Ctrl", "3"]], "shortcuts.actions.month"],
      [[["Ctrl", "4"]], "shortcuts.actions.year"],
      [[["T"]], "shortcuts.actions.today"],
      [[["Ctrl", "Shift", "T"]], "shortcuts.actions.goToDate"],
      [[["←"]], "shortcuts.actions.previous"],
      [[["→"]], "shortcuts.actions.next"],
      [[["Ctrl", "N"]], "shortcuts.actions.newEvent"],
      [[["Ctrl", "D"]], "shortcuts.actions.duplicateEvent"],
      [[["Ctrl", "C"]], "shortcuts.actions.copyEvent"],
      [[["Ctrl", "V"]], "shortcuts.actions.pasteEvent"],
      [[["Ctrl", "Alt", "Arrows"]], "shortcuts.actions.nudgeTheSelectedEvent"],
      [[["Ctrl", "Z"]], "shortcuts.actions.undoTheLastChangeOrDeletion"],
    ],
  },
  {
    app: "shortcuts.apps.contacts",
    key: "contacts",
    rows: [
      [[["Ctrl", "N"]], "shortcuts.actions.newContact"],
      [[["Ctrl", "Shift", "N"]], "shortcuts.actions.newList"],
      [[["Ctrl", "L"], ["Return"]], "shortcuts.actions.editContact"],
      [[["Delete"]], "shortcuts.actions.deleteContact"],
      [[["Ctrl", "F"]], "shortcuts.actions.search"],
      [[["Ctrl", "A"]], "shortcuts.actions.selectAll"],
    ],
  },
  {
    app: "shortcuts.apps.drive",
    key: "drivews",
    rows: [
      [[["Return"], ["Ctrl", "↓"]], "shortcuts.actions.open"],
      [[["Ctrl", "↑"]], "shortcuts.actions.enclosingFolder"],
      [[["Space"]], "shortcuts.actions.quickPreview"],
      [[["F2"]], "shortcuts.actions.rename"],
      [[["Delete"]], "shortcuts.actions.moveToRecentlyDeleted"],
    ],
  },
  {
    app: "shortcuts.apps.photos",
    key: "photos",
    rows: [
      [[["←"]], "shortcuts.actions.previous"],
      [[["→"]], "shortcuts.actions.next"],
      [[["."], ["F"]], "shortcuts.actions.favourite"],
      [[["I"]], "shortcuts.actions.info"],
      [[["Z"]], "shortcuts.actions.zoomToFitOrFullSize"],
      [[["Ctrl", "+"]], "shortcuts.actions.zoomIn"],
      [[["Ctrl", "-"]], "shortcuts.actions.zoomOut"],
      [[["Ctrl", "0"]], "shortcuts.actions.resetZoom"],
      [[["Space"]], "shortcuts.actions.playAVideoOrLivePhoto"],
      [[["Esc"]], "shortcuts.actions.closeTheViewer"],
    ],
  },
];

export function ShortcutsSheet({ front, onClose }: { front: string | null; onClose: () => void }) {
  useI18n();
  const sheetRef = useRef<HTMLDivElement | null>(null);
  useModalFocus(sheetRef);
  // An app's own window shows its own keys and those that work everywhere, nothing of the
  // other five apps (each is a window of its own); Skykeep's window, the overview, shows all.
  // Float on Top only where the desktop lets an app float (not Wayland: FloatOnTop.tsx).
  const [floats, setFloats] = useState(false);
  useEffect(() => {
    let live = true;
    api.windowFloatPossible().then((can) => live && setFloats(can), () => {});
    return () => {
      live = false;
    };
  }, []);
  const shown = SHORTCUTS.map((g) => ({
    ...g,
    rows: g.rows.filter(([, action]) => floats || action !== "shortcuts.actions.floatOnTop"),
  }));
  const app = shown.find((g) => g.key !== "" && g.key === front);
  const groups = app ? [app, ...shown.filter((g) => g.key === "")] : shown;
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
          <h2 id="shortcuts-title">{t("shortcuts.title")}</h2>
          <div className="shortcuts-groups" tabIndex={0} aria-label={t("shortcuts.byApp")}>
            {groups.map((g) => (
              <section key={g.key || "all"} aria-labelledby={`shortcuts-${g.key || "all"}`}>
                <h3 id={`shortcuts-${g.key || "all"}`}>{t(g.app)}</h3>
                <dl>
                  {g.rows.map(([keys, action]) => (
                    <div key={action} className="shortcuts-row">
                      <dt>{t(action)}</dt>
                      <dd>
                        {keys.map((combo, i) => (
                          <span key={i} className="shortcuts-either">
                            {i > 0 && <span className="shortcuts-or">{t("shortcuts.or")}</span>}
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
              {t("shortcuts.done")}
            </button>
          </div>
        </div>
      </div>
    </Portal>
  );
}
