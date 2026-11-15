/**
 * A slideshow, as Photos plays one: each photo whole on black, dissolving into the next. A video
 * shows its own picture, then plays from where that picture is, and the show moves on when it
 * ends. Space pauses, the arrows step, Escape ends; the controls sit in a glass capsule that
 * fades away while the pointer is still.
 *
 * Two layers, made once and never replaced, take turns: the next photo goes into the one behind,
 * made the size of the screen (fitSrc) and decoded there — on that very element — before it is
 * shown, so it is sharp from its first frame and never a blank or soft one. Only then does it
 * dissolve in over the other, its opacity set by the page frame by frame (as lib/fade does:
 * every animation WebKitGTK handed its compositor glitched on his Hyprland). The photo leaving
 * holds whole beneath it and lets go only at the end, so the screen never dims to black between
 * two photos. A layer is never torn down and built again: a new element painted nothing for a
 * frame before its picture came in, which was the flash on every step. With reduced motion the
 * change is a cut.
 *
 * A picture that cannot be had is passed over, never shown as a black frame.
 */
import { t } from "../../lib/i18n";
import { useActiveRef } from "../../lib/screen";
import { useCallback, useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, Pause, Play, X } from "lucide-react";
import { api, fitSrc, turnStyle, type Photo, type Turn } from "../../lib/ipc";

/** How long a photo stays. */
const HOLD = 4000;
/** How long one photo takes to dissolve into the next. */
const FADE = 800;
/** The longest a video plays before the show moves on. */
const FILM_CAP = 60_000;

const still = () => typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** The box a photo fills on this screen, whole, in CSS pixels. */
function boxOf(p: Photo) {
  const W = window.innerWidth;
  const H = window.innerHeight;
  const r = p.width && p.height ? p.width / p.height : 4 / 3;
  const w = Math.min(W, H * r);
  return { w, h: w / r, W, H };
}

/** The picture a photo is shown at: whole on this screen, in device pixels — never smaller. */
export const screenSrc = (p: Photo) => {
  const { w, h } = boxOf(p);
  return fitSrc(p, Math.max(w, h) * (window.devicePixelRatio || 1));
};

/** Fetched and decoded ahead, by address: the next is ready before its turn. */
const warmed = new Map<string, Promise<boolean>>();
function warm(url: string) {
  let p = warmed.get(url);
  if (!p) {
    const img = new Image();
    img.decoding = "async";
    img.src = url;
    p = img.decode().then(
      () => true,
      () => false,
    );
    warmed.set(url, p);
    if (warmed.size > 24) warmed.delete(warmed.keys().next().value as string);
  }
  return p;
}

/** One of the two layers: its picture, and its film when it holds a video. */
type Layer = { root: HTMLDivElement; img: HTMLImageElement; box: HTMLDivElement; film: HTMLVideoElement };

/**
 * `ids` are the photos in order — a whole library's worth is only its ids; each photo is fetched
 * (`load`) a few ahead of its turn and held only while the show is near it. `videoBase` is the
 * app's loopback server (video.rs); without it a video shows as its picture.
 */
export function Slideshow({
  ids,
  start,
  load,
  videoBase,
  onClose,
}: {
  ids: string[];
  start: number;
  load: (ids: string[]) => Promise<Photo[]>;
  videoBase?: string | null;
  onClose: () => void;
}) {
  const first = Math.max(0, Math.min(start, ids.length - 1));
  const have = useRef(new Map<string, Photo>());
  /** The photo at `i`, fetched with the few after it when it is not here. */
  const fetchAt = useCallback(
    async (i: number): Promise<Photo | undefined> => {
      const id = ids[i];
      const here = have.current.get(id);
      if (here) return here;
      const want = ids.slice(i, i + 4).filter((x) => !have.current.has(x));
      const got = await load(want).catch(() => [] as Photo[]);
      // Only those near where the show is are kept.
      for (const [x] of have.current) if (Math.abs(ids.indexOf(x) - i) > 12) have.current.delete(x);
      for (const p of got) have.current.set(p.id, p);
      return have.current.get(id);
    },
    [ids, load],
  );

  const [at, setAt] = useState(first);
  const [playing, setPlaying] = useState(true);
  const [awake, setAwake] = useState(true);
  const [ready, setReady] = useState(false);
  /** Playing, as the timers that outlive a render read it. */
  const playingNow = useRef(true);

  const a = useRef<Layer | null>(null);
  const b = useRef<Layer | null>(null);
  /** The layer in front, showing `shownAt`. */
  const front = useRef<"a" | "b">("a");
  const shownAt = useRef(-1);
  const asked = useRef(0);
  const fading = useRef(0);
  const hold = useRef<number | undefined>(undefined);
  const layer = (k: "a" | "b") => (k === "a" ? a.current : b.current);
  const stopFilm = (l: Layer | null) => {
    if (!l) return;
    const v = l.film;
    v.onended = v.onerror = null;
    v.pause();
    if (v.getAttribute("src")) {
      v.removeAttribute("src");
      v.load();
    }
    v.style.opacity = "0";
    l.box.hidden = true;
  };

  /** The film of a video in the front layer: over its picture once a frame of it is here. */
  const playFilm = useCallback(
    async (l: Layer, p: Photo, mine: number, next: () => void) => {
      const v = l.film;
      const box = l.box;
      const { w, h, W, H } = boxOf(p);
      box.style.width = `${w}px`;
      box.style.height = `${h}px`;
      box.style.left = `${(W - w) / 2}px`;
      box.style.top = `${(H - h) / 2}px`;
      let turn: Turn = [1, 0, 0, 1];
      let which: "video" | "original" = "video";
      try {
        turn = await api.photoVideoTurn(p.id, which);
      } catch {
        which = "original";
        turn = await api.photoVideoTurn(p.id, which).catch(() => [1, 0, 0, 1] as Turn);
      }
      if (mine !== asked.current) return;
      Object.assign(v.style, { position: "", left: "", top: "", width: "100%", height: "100%", translate: "", transform: "", maxWidth: "", maxHeight: "" }, turnStyle(turn, p.width, p.height));
      v.style.opacity = "0";
      box.hidden = false;
      const shown = () => {
        if (mine === asked.current) requestAnimationFrame(() => (v.style.opacity = "1"));
      };
      type Rvfc = HTMLVideoElement & { requestVideoFrameCallback?: (cb: () => void) => number };
      v.ontimeupdate = () => {
        if (v.currentTime > 0.04 && v.readyState >= 2) {
          v.ontimeupdate = null;
          shown();
        }
      };
      const cap = window.setTimeout(next, FILM_CAP);
      v.onended = () => {
        window.clearTimeout(cap);
        // The last frame stays: the next photo dissolves in over it.
        if (mine === asked.current) next();
      };
      v.onerror = () => {
        if (mine !== asked.current) return;
        if (which === "video") {
          which = "original";
          v.src = `${videoBase}/${p.id}/original`;
          void v.play().catch(() => undefined);
          return;
        }
        // Not playable here: its picture stays its time, as a photo's would.
        window.clearTimeout(cap);
        stopFilm(l);
        hold.current = window.setTimeout(next, HOLD);
      };
      v.src = `${videoBase}/${p.id}/${which}`;
      (v as Rvfc).requestVideoFrameCallback?.(shown);
      if (!playingNow.current) return;
      // With its sound, as Photos plays one; muted only where the engine will not start it so.
      await v.play().catch(async () => {
        v.muted = true;
        await v.play().catch(() => undefined);
      });
    },
    [videoBase],
  );

  /** To the photo `by` along, once its picture is decoded in the layer behind. */
  const go = useCallback(
    async (by: number, from?: number) => {
      const n = ids.length;
      if (!n) return;
      const mine = ++asked.current;
      window.clearTimeout(hold.current);
      let to = (((from ?? shownAt.current) + by) % n + n) % n;
      // A picture that cannot be had is passed over (a few at most), never shown black.
      for (let tries = 0; tries < Math.min(n, 6); tries++) {
        const p = await fetchAt(to);
        if (mine !== asked.current) return;
        const url = p ? screenSrc(p) : null;
        if (p && url && (await warm(url))) {
          const back = layer(front.current === "a" ? "b" : "a");
          const top = layer(front.current);
          if (!back || !top) return;
          stopFilm(back);
          back.img.src = url;
          back.img.alt = p.filename ?? t("photos.photo");
          // Decoded on the element that will show it, so its first painted frame is whole.
          await back.img.decode().catch(() => undefined);
          if (mine !== asked.current) return;
          await dissolve(back, top, shownAt.current < 0 || still());
          if (mine !== asked.current) return;
          front.current = front.current === "a" ? "b" : "a";
          shownAt.current = to;
          setAt(to);
          setReady(true);
          stopFilm(top);
          // Ready before its turn: the next one's picture.
          void fetchAt((to + 1) % n).then((q) => q && warm(screenSrc(q)));
          const next = () => {
            if (playingNow.current && mine === asked.current) void go(1);
          };
          if (p.kind === "video" && videoBase) void playFilm(back, p, mine, next);
          else if (playingNow.current && n > 1) hold.current = window.setTimeout(next, HOLD);
          return;
        }
        to = (to + Math.sign(by || 1) + n) % n;
      }
    },
    [ids, fetchAt, playFilm, videoBase],
  );

  /** The layer behind dissolves in over the one in front, which holds whole until the end. */
  const dissolve = (inc: Layer, out: Layer, cut: boolean) =>
    new Promise<void>((done) => {
      cancelAnimationFrame(fading.current);
      inc.root.style.zIndex = "2";
      out.root.style.zIndex = "1";
      if (cut) {
        inc.root.style.opacity = "1";
        out.root.style.opacity = "0";
        done();
        return;
      }
      inc.root.style.opacity = "0";
      out.root.style.opacity = "1";
      const t0 = performance.now();
      const step = (now: number) => {
        const t = Math.max(0, Math.min(1, (now - t0) / FADE));
        // Eased both ways, as a dissolve on a Mac: no jump at either end.
        const k = t < 0.5 ? 2 * t * t : 1 - (2 - 2 * t) ** 2 / 2;
        inc.root.style.opacity = String(k);
        // Whole beneath the arriving photo for nearly all of it — no dip to black where the two
        // overlap — and gone by the end where it is wider than the arriving one.
        out.root.style.opacity = String(1 - k ** 4);
        if (t < 1) fading.current = requestAnimationFrame(step);
        else {
          out.root.style.opacity = "0";
          done();
        }
      };
      fading.current = requestAnimationFrame(step);
    });

  // The first photo, then on round the show; everything let go at the end.
  useEffect(() => {
    void go(0, first);
    return () => {
      asked.current++;
      window.clearTimeout(hold.current);
      cancelAnimationFrame(fading.current);
      stopFilm(a.current);
      stopFilm(b.current);
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const toggle = useCallback(() => {
    setPlaying((was) => {
      const now = !was;
      playingNow.current = now;
      const l = layer(front.current);
      const filming = !!l?.film.getAttribute("src") && !l.box.hidden;
      if (!now) {
        window.clearTimeout(hold.current);
        l?.film.pause();
      } else if (filming) void l?.film.play().catch(() => undefined);
      else if (ids.length > 1) {
        const mine = asked.current;
        hold.current = window.setTimeout(() => mine === asked.current && void go(1), HOLD);
      }
      return now;
    });
  }, [go, ids.length]); // eslint-disable-line react-hooks/exhaustive-deps

  const live = useActiveRef();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!live.current) return;
      if (e.key === "Escape") onClose();
      else if (e.key === " ") toggle();
      else if (e.key === "ArrowRight") void go(1);
      else if (e.key === "ArrowLeft") void go(-1);
      else {
        // Every other key stops here too: the viewer and the grid behind took F, H and Delete
        // as their own and changed the photo under the show, unseen. Its default still
        // happens — Tab still moves along the show's own controls.
        e.stopPropagation();
        return;
      }
      e.preventDefault();
      e.stopPropagation();
    };
    // Before the screen's own keys, which would otherwise step the photo behind or close it.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [go, toggle, onClose]); // eslint-disable-line react-hooks/exhaustive-deps

  // A film playing in the viewer behind stops as the show begins: it went on, heard and
  // unseen, under the show.
  useEffect(() => {
    document.querySelectorAll<HTMLVideoElement>("video").forEach((v) => {
      if (!v.closest(".px-show")) v.pause();
    });
  }, []);

  // The controls show when the pointer moves and go once it rests.
  const idle = useRef<number | undefined>(undefined);
  const wake = () => {
    setAwake(true);
    window.clearTimeout(idle.current);
    idle.current = window.setTimeout(() => setAwake(false), 2200);
  };
  useEffect(() => {
    wake();
    return () => window.clearTimeout(idle.current);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const mount = (k: "a" | "b") => (root: HTMLDivElement | null) => {
    const ref = k === "a" ? a : b;
    if (!root) {
      ref.current = null;
      return;
    }
    ref.current = {
      root,
      img: root.querySelector("img") as HTMLImageElement,
      box: root.querySelector(".px-slide-film") as HTMLDivElement,
      film: root.querySelector("video") as HTMLVideoElement,
    };
  };
  const slide = (k: "a" | "b") => (
    <div ref={mount(k)} className="px-slide" style={{ opacity: 0 }} aria-hidden={k !== front.current || undefined}>
      <img alt="" decoding="sync" draggable={false} />
      <div className="px-slide-film" hidden>
        <video playsInline preload="auto" aria-label={t("photos.videoAria")} />
      </div>
    </div>
  );

  return (
    <div className={`px-show${awake ? " is-awake" : ""}`} role="dialog" aria-label={t("ui.slideshow")} aria-busy={!ready} onMouseMove={wake} onClick={wake}>
      {slide("a")}
      {slide("b")}
      <div className="px-show-bar toolbar-group" role="toolbar" aria-label={t("ui.slideshowControls")}>
        <button className="tool px-cap" aria-label={t("ui.previous")} onClick={() => void go(-1)}>
          <ChevronLeft size={18} />
        </button>
        <button className="tool px-cap" aria-label={playing ? "Pause" : "Play"} onClick={toggle}>
          {playing ? <Pause size={17} fill="currentColor" strokeWidth={0} /> : <Play size={17} fill="currentColor" strokeWidth={0} />}
        </button>
        <button className="tool px-cap" aria-label={t("ui.next")} onClick={() => void go(1)}>
          <ChevronRight size={18} />
        </button>
        <span className="px-show-count" aria-live="polite">
          {at + 1} of {ids.length}
        </span>
        <button className="tool px-cap" aria-label={t("ui.endSlideshow")} title={t("ui.endSlideshowEsc")} onClick={onClose}>
          <X size={17} />
        </button>
      </div>
    </div>
  );
}
