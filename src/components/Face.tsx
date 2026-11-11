import { useLayoutEffect, useRef, useState } from "react";

/**
 * A person's picture over their initials, as Contacts draws one.
 *
 * The initials hold the place until the picture is decoded, and the picture then takes it,
 * drawn whole in one frame (decoded before it is drawn, never shown half-way). A picture
 * already to hand (the backend keeps each on disk, the window keeps it in memory) is drawn at
 * once. Nothing fades: a fade replayed every time Contacts came back into view. One that
 * cannot be had leaves the initials, never a broken frame.
 *
 * Fetched only near the screen and decoded off the main thread: with `decoding="sync"` and no
 * lazy loading, a book of 2,000 pictured cards asked for every picture the moment Contacts
 * opened and decoded each on the main thread. The initials stay until `decode()` says the
 * picture is ready to draw, so it still arrives whole in one frame.
 */
export function Face({ src, initials }: { src: string | null; initials: string }) {
  const img = useRef<HTMLImageElement | null>(null);
  const [state, setState] = useState<"loading" | "ready" | "instant" | "failed">("loading");
  // Reset for a different picture; decided before paint whether it is already there.
  useLayoutEffect(() => {
    const el = img.current;
    setState(el && el.complete && el.naturalWidth > 0 ? "instant" : "loading");
  }, [src]);
  return (
    <>
      {state !== "ready" && state !== "instant" && <span className="face-initials">{initials}</span>}
      {src && state !== "failed" && (
        <img
          ref={img}
          className={`face-photo is-${state}`}
          src={src}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onLoad={(e) => {
            const el = e.currentTarget;
            const shown = el.src;
            el.decode()
              .catch(() => {})
              .then(() => {
                if (img.current === el && el.src === shown) setState((s) => (s === "instant" ? s : "ready"));
              });
          }}
          onError={() => setState("failed")}
        />
      )}
    </>
  );
}
