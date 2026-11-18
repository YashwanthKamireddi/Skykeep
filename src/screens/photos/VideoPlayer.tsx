/**
 * A video as Photos plays one: the film alone on the stage, its controls in a glass capsule
 * that floats over the bottom of it and fades away while the pointer rests — play and pause,
 * where it is, a scrubber, what is left, and sound. A click on the film plays or pauses it;
 * Space does too (the viewer's own keys still step through the photos).
 *
 * Until the film's first frame is on screen its poster stands in its place — the video's own
 * picture, sharp, made the size it is shown at — and the film is drawn over it only once a frame
 * of it has been presented: WebKitGTK paints a video black from the moment it starts loading
 * until its first frame arrives, and that black flashed on every open. While it waits on the
 * network a small spinner says so.
 *
 * The scrubber follows the film every frame (the engine's own timeupdate comes four times a
 * second, and a progress bar that jumps reads as a stutter), and dragging it seeks as it goes.
 */
import { t } from "../../lib/i18n";
import { useActiveRef, useScreenActive } from "../../lib/screen";
import { useCallback, useEffect, useRef, useState } from "react";
import { Pause, Play, Volume2, VolumeX } from "lucide-react";
import { turnStyle, type Turn } from "../../lib/ipc";
import { clock, say } from "./clock";

export function VideoPlayer({
  src,
  poster,
  label,
  style,
  turn,
  zoom = 1,
  shape,
  onError,
}: {
  src: string;
  poster: string;
  label: string;
  style?: React.CSSProperties;
  /** How the film is turned, as its file records it; not shown until it is known. */
  turn?: Turn;
  /** How much larger than its box the film is drawn (its clean aperture; see turnStyle). */
  zoom?: number;
  /** The photo's own width and height, upright: the box the film is turned into. */
  shape: { w: number; h: number };
  onError: () => void;
}) {
  const film = useRef<HTMLVideoElement | null>(null);
  const fill = useRef<HTMLDivElement | null>(null);
  const knob = useRef<HTMLDivElement | null>(null);
  const said = useRef<HTMLSpanElement | null>(null);
  const left = useRef<HTMLSpanElement | null>(null);
  const [playing, setPlaying] = useState(false);
  const [muted, setMuted] = useState(false);
  const [awake, setAwake] = useState(true);
  const [hover, setHover] = useState<{ x: number; t: number } | null>(null);
  /** A frame of the film has been presented: drawn over its poster from then on. */
  const [shown, setShown] = useState(false);
  /** Waiting on its bytes for longer than a moment. */
  const [stalled, setStalled] = useState(false);
  const stallTimer = useRef<number | undefined>(undefined);
  const waitOn = useCallback((on: boolean) => {
    window.clearTimeout(stallTimer.current);
    if (on) stallTimer.current = window.setTimeout(() => setStalled(true), 450);
    else setStalled(false);
  }, []);
  useEffect(() => {
    waitOn(true);
    return () => window.clearTimeout(stallTimer.current);
  }, [waitOn]);
  // The first frame presented, as the engine says it (requestVideoFrameCallback), or failing
  // that the clock moving: only then is the film drawn over its poster.
  useEffect(() => {
    const v = film.current;
    if (!v || shown) return undefined;
    type Rvfc = HTMLVideoElement & { requestVideoFrameCallback?: (cb: () => void) => number; cancelVideoFrameCallback?: (id: number) => void };
    const rv = v as Rvfc;
    let id = 0;
    const done = () => requestAnimationFrame(() => setShown(true));
    if (rv.requestVideoFrameCallback) id = rv.requestVideoFrameCallback(done);
    const moved = () => {
      if (v.currentTime > 0.04 && v.readyState >= 2) done();
    };
    v.addEventListener("timeupdate", moved);
    return () => {
      v.removeEventListener("timeupdate", moved);
      if (id && rv.cancelVideoFrameCallback) rv.cancelVideoFrameCallback(id);
    };
  }, [shown]);
  const dragging = useRef(false);
  // Another screen brought to the front pauses it, as leaving a film does on a Mac.
  const active = useScreenActive();
  useEffect(() => {
    if (!active) film.current?.pause();
  }, [active]);

  // Where it is, drawn straight to the scrubber and the clocks once a frame — never through
  // React, so a playing film redraws nothing else.
  const draw = useCallback(() => {
    const v = film.current;
    if (!v) return;
    const d = v.duration || 0;
    const k = d ? Math.min(1, v.currentTime / d) : 0;
    if (fill.current) fill.current.style.transform = `scaleX(${k})`;
    if (knob.current) knob.current.style.left = `${k * 100}%`;
    say(said.current, clock(v.currentTime));
    say(left.current, `−${clock(d - v.currentTime)}`);
  }, []);
  useEffect(() => {
    if (!playing) return undefined;
    let id = 0;
    const tick = () => {
      draw();
      id = requestAnimationFrame(tick);
    };
    id = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(id);
  }, [playing, draw]);

  // The controls show when the pointer moves and go once it rests — but never while paused.
  const idle = useRef<number | undefined>(undefined);
  const wake = useCallback(() => {
    setAwake(true);
    window.clearTimeout(idle.current);
    idle.current = window.setTimeout(() => setAwake(false), 2000);
  }, []);
  useEffect(() => () => window.clearTimeout(idle.current), []);

  const toggle = useCallback(() => {
    const v = film.current;
    if (!v) return;
    if (v.paused) void v.play().catch(() => {});
    else v.pause();
  }, []);
  // Space plays and pauses, before the viewer's own keys.
  const live = useActiveRef();
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (!live.current) return;
      if (e.key !== " " || (e.target as HTMLElement | null)?.closest?.("input, textarea, [contenteditable]")) return;
      // A slideshow over the viewer has Space for itself (Slideshow.tsx).
      if (document.querySelector(".px-show")) return;
      e.preventDefault();
      e.stopPropagation();
      toggle();
      wake();
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, [toggle, wake]);

  const seekTo = (clientX: number, bar: HTMLElement) => {
    const v = film.current;
    if (!v || !v.duration) return;
    const r = bar.getBoundingClientRect();
    v.currentTime = Math.max(0, Math.min(1, (clientX - r.left) / r.width)) * v.duration;
    draw();
  };

  return (
    <div className={`px-player${awake || !playing ? " is-awake" : ""}`} style={style} onPointerMove={wake}>
      {/* The video's own picture, sharp, until a frame of the film is on screen. */}
      {!(shown && turn) && <img className="px-player-poster" src={poster} alt="" aria-hidden="true" decoding="sync" draggable={false} />}
      <video
        ref={film}
        className="px-shown"
        // Turned as it was recorded, in the photo's own box (lib/ipc turnStyle); not drawn
        // until the turn is known and a frame is here, so a film never shows sideways or black.
        style={{ ...(turn ? turnStyle(turn, shape.w, shape.h, zoom) : {}), opacity: shown && turn ? 1 : 0 }}
        src={src}
        aria-label={label}
        autoPlay
        playsInline
        preload="auto"
        onClick={() => {
          toggle();
          wake();
        }}
        onPlay={() => {
          setPlaying(true);
          wake();
        }}
        onWaiting={() => waitOn(true)}
        onPlaying={() => waitOn(false)}
        onCanPlay={() => waitOn(false)}
        onSeeking={() => waitOn(true)}
        onPause={() => {
          setPlaying(false);
          draw();
        }}
        onLoadedMetadata={draw}
        onSeeked={() => {
          waitOn(false);
          draw();
        }}
        onVolumeChange={(e) => setMuted(e.currentTarget.muted)}
        onError={() => {
          waitOn(false);
          onError();
        }}
      />
      {stalled && <span className="px-player-wait" role="status" aria-label={t("ui.loadingVideo")} />}
      <div className="px-player-bar toolbar-group" role="toolbar" aria-label={t("ui.videoControls")} onClick={(e) => e.stopPropagation()}>
        <button className="tool px-cap" aria-label={playing ? t("ui.pause") : t("ui.play")} onClick={toggle}>
          {playing ? <Pause size={17} strokeWidth={0} fill="currentColor" /> : <Play size={17} strokeWidth={0} fill="currentColor" />}
        </button>
        <span className="px-player-time" ref={said}>
          0:00
        </span>
        <div
          className="px-scrub"
          role="slider"
          aria-label={t("photos.playbackPosition")}
          aria-valuemin={0}
          aria-valuemax={100}
          tabIndex={0}
          onPointerDown={(e) => {
            e.currentTarget.setPointerCapture(e.pointerId);
            dragging.current = true;
            seekTo(e.clientX, e.currentTarget);
          }}
          onPointerMove={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            const v = film.current;
            setHover({ x: e.clientX - r.left, t: v?.duration ? ((e.clientX - r.left) / r.width) * v.duration : 0 });
            if (dragging.current) seekTo(e.clientX, e.currentTarget);
          }}
          onPointerUp={() => (dragging.current = false)}
          onPointerCancel={() => (dragging.current = false)}
          onPointerLeave={() => setHover(null)}
          onKeyDown={(e) => {
            const v = film.current;
            if (!v || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
            e.preventDefault();
            e.stopPropagation();
            v.currentTime = Math.max(0, Math.min(v.duration || 0, v.currentTime + (e.key === "ArrowLeft" ? -5 : 5)));
          }}
        >
          <div className="px-scrub-track">
            <div className="px-scrub-fill" ref={fill} />
          </div>
          <div className="px-scrub-knob" ref={knob} />
          {hover && (
            <span className="px-scrub-tip" style={{ left: hover.x }}>
              {clock(hover.t)}
            </span>
          )}
        </div>
        <span className="px-player-time" ref={left}>
          −0:00
        </span>
        <button
          className="tool px-cap"
          aria-label={muted ? t("photos.unmute") : t("photos.mute")}
          onClick={() => {
            const v = film.current;
            if (v) v.muted = !v.muted;
          }}
        >
          {muted ? <VolumeX size={17} /> : <Volume2 size={17} />}
        </button>
      </div>
    </div>
  );
}
