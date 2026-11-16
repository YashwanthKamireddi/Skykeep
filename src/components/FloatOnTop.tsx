/**
 * Float on Top (#40), as Apple names it in Notes' Window menu: an app's window kept above the
 * others while you work in something else, from the pin beside the sidebar button or Ctrl+Shift+P.
 * Remembered for each app and put back when its window opens again.
 *
 * Offered only where the desktop lets an app ask for it (Windows, X11). On Wayland there is no
 * protocol for it and GTK's request is ignored, so the pin and the shortcut aren't there at all
 * rather than there and doing nothing (main.rs, window_float_possible).
 */
import { useCallback, useEffect, useState } from "react";
import { Pin } from "lucide-react";
import { api } from "../lib/ipc";
import { modalOpen } from "../lib/screen";
import { useI18n } from "../lib/i18n";

const remembered = (app: string) => `skykeep.float.${app}`;

export function useFloatOnTop(app: string | null) {
  const { t } = useI18n();
  const [possible, setPossible] = useState(false);
  const [on, setOn] = useState(false);

  useEffect(() => {
    if (!app) return;
    let live = true;
    api.windowFloatPossible().then(
      (can) => {
        if (!live || !can) return;
        setPossible(true);
        let wanted = false;
        try {
          wanted = localStorage.getItem(remembered(app)) === "1";
        } catch {
          /* not kept: starts off */
        }
        if (wanted) api.windowFloat(true).then((now) => live && setOn(now), () => {});
      },
      () => {},
    );
    return () => {
      live = false;
    };
  }, [app]);

  const toggle = useCallback(() => {
    if (!app || !possible) return;
    const next = !on;
    setOn(next);
    api.windowFloat(next).then(
      (now) => {
        setOn(now);
        try {
          localStorage.setItem(remembered(app), now ? "1" : "0");
        } catch {
          /* kept for this window only */
        }
      },
      () => setOn(!next),
    );
  }, [app, possible, on]);

  // Ctrl+Shift+P, from anywhere in the window but a sheet over it.
  useEffect(() => {
    if (!possible) return;
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || !e.shiftKey || e.altKey || e.key.toLowerCase() !== "p") return;
      if (e.defaultPrevented || modalOpen()) return;
      e.preventDefault();
      toggle();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [possible, toggle]);

  const label = t("ui.floatOnTop");
  return possible && app
    ? (
      <button
        className="icon-button float-toggle"
        aria-label={label}
        aria-pressed={on}
        title={`${label} (Ctrl+Shift+P)`}
        onClick={toggle}
      >
        <Pin size={16} strokeWidth={2} aria-hidden="true" />
      </button>
    )
    : null;
}
