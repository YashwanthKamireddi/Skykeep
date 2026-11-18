/**
 * The video player's clocks: where the film is and what is left, as Photos writes them.
 */

/** Seconds as Photos writes them: "0:07", "12:30", "1:02:05". */
export const clock = (s: number) => {
  if (!Number.isFinite(s) || s < 0) s = 0;
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = Math.floor(s % 60);
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}` : `${m}:${String(sec).padStart(2, "0")}`;
};

/**
 * Put `text` in `el` only when it says something new. The clocks are drawn every frame while
 * a film plays but change once a second: writing the same words sixty times a second still
 * replaced the label's text and made the engine lay the capsule out again each time.
 */
export function say(el: { textContent: string | null } | null, text: string): void {
  if (el && el.textContent !== text) el.textContent = text;
}
