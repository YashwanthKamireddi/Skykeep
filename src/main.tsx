import React, { lazy, Suspense } from "react";
import ReactDOM from "react-dom/client";
import "./styles/tokens.css";
import "./styles/kit.css";
import "./styles/app.css";
import "./styles/reminders.css";
import "./styles/calendar.css";
import "./styles/notes.css";
import "./styles/photos.css";
import "./styles/a11y.css";
import { applyTheme, followContrast, followTheme, storedTheme } from "./lib/theme";
import { keepTinted } from "./lib/tint";
import { fadeSheetsIn } from "./lib/fade";
import { followClick } from "./lib/follow";
import { markEngine } from "./lib/engine";
import { slideThumbs } from "./lib/thumb";
import { carryDragsInPage } from "./lib/drag";
import { answerFlushes } from "./lib/flush";

// Before the first paint, so the window opens in the appearance chosen.
markEngine();
applyTheme(storedTheme());
followTheme();
followContrast();
keepTinted();
fadeSheetsIn();
slideThumbs();
// On Windows a drag inside the window is carried by the mouse (lib/drag.ts).
carryDragsInPage();
// Quit, sign-out and a close with no tray ask every window to save what is typed first.
answerFlushes();

/*
 * Every link goes the one way out: through `followClick` (lib/follow.ts), which checks what
 * kind of address it is before anything happens — the engine never follows a link by itself.
 * A permitted one leaves as a navigation, which the window refuses and hands to the app this
 * computer uses for it (see the "links" plugin in main.rs). Capturing, so a link whose own
 * handler stops the click from spreading is covered too; a middle click, which would ask for
 * a new window, is refused the same way.
 */
document.addEventListener("click", (e) => void followClick(e), true);
document.addEventListener("auxclick", (e) => {
  if ((e.target as Element | null)?.closest?.("a[href]")) e.preventDefault();
}, true);

/*
 * The engine's own menu (Back, Forward, Reload, Inspect Element) is not this app's: a reload
 * from it flashed the whole window. Text fields keep their Cut, Copy and Paste; every menu the
 * app draws itself has already taken its right-click.
 */
document.addEventListener("contextmenu", (e) => {
  if (e.defaultPrevented) return;
  const editable = (e.target as HTMLElement | null)?.closest?.("input, textarea, [contenteditable=''], [contenteditable='true'], [contenteditable='plaintext-only']");
  if (!editable) e.preventDefault();
});

// The quick-capture window (`skykeep --new reminder`) is one field, not the library — and the
// library's window never reads its code. Nor does the capture window read the library's:
// imported here outright, App brought Library with it, which fetched Settings and listened for
// reveals in a window that only ever shows one field (desktop audit H9).
const capture = new URLSearchParams(window.location.search).get("capture");
const QuickCapture = lazy(() => import("./components/QuickCapture").then((m) => ({ default: m.QuickCapture })));
const App = lazy(() => import("./App"));

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {capture === "reminder" || capture === "note" ? (
      <Suspense fallback={null}>
        <QuickCapture initial={capture} />
      </Suspense>
    ) : (
      <Suspense fallback={null}>
        <App />
      </Suspense>
    )}
  </React.StrictMode>
);

