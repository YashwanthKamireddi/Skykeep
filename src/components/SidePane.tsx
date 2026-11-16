/**
 * The sidebar button of an app's own window, the same in all six: at the window's top left, it
 * hides the app's sidebar and brings it back, as the button beside a Mac window's controls does.
 * F9 does the same, as in GNOME's apps. Hidden or shown is remembered for each app.
 *
 * Where an app folds its sidebar away in a narrow window with nothing in its place (Calendar,
 * Drive), the button brings the sidebar over the content instead; Escape, a click outside, or
 * (in Drive) picking a place puts it back. Where an app folds its sidebar into a picker or a
 * column of its own (Notes, Contacts, Photos, Reminders), that already reaches it, and the
 * button gives way (app.css).
 *
 * The state is two classes on the window (`side-hidden`, `side-peek`) that the stylesheet
 * reads: showing or hiding draws nothing again, and keeps each list where it was.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { PanelLeft } from "lucide-react";
import { modalOpen } from "../lib/screen";
import { useI18n } from "../lib/i18n";

/** Every app's sidebar, of which an app's window holds one. */
export const SIDEBARS = ".notes-folders, .contacts-lists, .rx-side, .px-side, .cx-side, .dx-side";

const remembered = (app: string) => `skykeep.sidebar.${app}`;

/** Asked of the window by an app with its own way to the sidebar (Calendar's More menu in a
 *  narrow window); `returnTo` takes the focus back when it is put away. */
export const SIDEBAR_ASKED = "skykeep:sidebar";
export const askForSidebar = (returnTo: HTMLElement | null) =>
  window.dispatchEvent(new CustomEvent(SIDEBAR_ASKED, { detail: { returnTo } }));

export function useSidePane(app: string | null) {
  const { t } = useI18n();
  const [hidden, setHidden] = useState(() => {
    try {
      return app !== null && localStorage.getItem(remembered(app)) === "hidden";
    } catch {
      return false;
    }
  });
  const [peek, setPeek] = useState(false);
  // Folded away by the window's width rather than by the button, read from the stylesheet's
  // own answer so the two never disagree on where the line is.
  const [folded, setFolded] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  // Where the focus goes back to when the sidebar over the content is put away.
  const opener = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!app) return;
    let frame = 0;
    const read = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const side = document.querySelector<HTMLElement>(SIDEBARS);
        setFolded(!!side && !side.closest(".side-peek") && getComputedStyle(side).display === "none");
      });
    };
    read();
    // The app's code arrives after the window does; its sidebar appears then.
    const appear = new MutationObserver(() => {
      if (document.querySelector(SIDEBARS)) {
        appear.disconnect();
        read();
      }
    });
    // Watched only until it does: an app's own changes (a grid of thousands) are never read.
    if (!document.querySelector(SIDEBARS)) appear.observe(document.body, { childList: true, subtree: true });
    window.addEventListener("resize", read);
    return () => {
      cancelAnimationFrame(frame);
      appear.disconnect();
      window.removeEventListener("resize", read);
    };
  }, [app, hidden, peek]);

  const toggle = useCallback(() => {
    if (!app) return;
    const side = document.querySelector<HTMLElement>(SIDEBARS);
    if (!side) return;
    // Folded away by the window's width (not by the button): bring it over the content.
    const narrow = !peek && getComputedStyle(side).display === "none";
    if (narrow || peek) {
      if (!peek) opener.current = (document.activeElement as HTMLElement | null) ?? button.current;
      setPeek((open) => !open);
      return;
    }
    setHidden((was) => {
      const now = !was;
      try {
        localStorage.setItem(remembered(app), now ? "hidden" : "shown");
      } catch {
        /* A window without storage keeps it for this window only. */
      }
      return now;
    });
  }, [app, hidden, peek]);

  // Asked by the app itself.
  useEffect(() => {
    if (!app) return;
    const onAsk = (e: Event) => {
      toggle();
      const returnTo = (e as CustomEvent<{ returnTo?: HTMLElement | null }>).detail?.returnTo;
      if (returnTo) opener.current = returnTo;
    };
    window.addEventListener(SIDEBAR_ASKED, onAsk);
    return () => window.removeEventListener(SIDEBAR_ASKED, onAsk);
  }, [app, toggle]);

  // F9, from anywhere in the window but a sheet over it.
  useEffect(() => {
    if (!app) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "F9" || e.ctrlKey || e.metaKey || e.altKey || e.shiftKey || e.defaultPrevented) return;
      if (modalOpen()) return;
      e.preventDefault();
      toggle();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [app, toggle]);

  // Over the content: the focus goes into it, and Escape, a click outside, or a window grown
  // wide enough to hold it again puts it back.
  useEffect(() => {
    if (!peek) return;
    const side = document.querySelector<HTMLElement>(SIDEBARS);
    side?.querySelector<HTMLElement>("button, input, [tabindex='0']")?.focus({ preventScroll: true });
    const close = (returnFocus: boolean) => {
      setPeek(false);
      if (!returnFocus) return;
      // Back to what opened it, if it is still there to take it; else the button.
      const back = [opener.current, button.current].find((el) => el?.isConnected && el.getClientRects().length > 0);
      back?.focus({ preventScroll: true });
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || modalOpen()) return;
      e.preventDefault();
      e.stopPropagation();
      close(true);
    };
    const onDown = (e: PointerEvent) => {
      const t = e.target as Element | null;
      if (!t || t.closest(".side-toggle")) return;
      if (!t.closest(SIDEBARS)) close(false);
    };
    // In Drive, picking a place is the whole errand; Calendar's switches are often several.
    const onPick = (e: MouseEvent) => {
      if ((e.target as Element | null)?.closest(".dx-side button")) close(false);
    };
    const onResize = () => {
      const s = document.querySelector<HTMLElement>(SIDEBARS);
      if (s && getComputedStyle(s).position !== "fixed") setPeek(false);
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("click", onPick);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("click", onPick);
      window.removeEventListener("resize", onResize);
    };
  }, [peek]);

  const visible = peek || (!folded && !hidden);
  const label = visible ? t("ui.hideSidebar") : t("ui.showSidebar");
  return {
    className: `${hidden ? " side-hidden" : ""}${peek ? " side-peek" : ""}${folded ? " side-folded" : ""}`,
    button: app ? (
      <button
        ref={button}
        className="icon-button side-toggle"
        aria-label={label}
        aria-expanded={visible}
        title={`${label} (F9)`}
        onClick={toggle}
      >
        <PanelLeft size={17} strokeWidth={2} aria-hidden="true" />
      </button>
    ) : null,
  };
}
