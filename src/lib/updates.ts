import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, UPDATE, type UpdateStatus } from "./ipc";

/**
 * Skykeep's own update, as this window sees it: asked of the backend once, again whenever it
 * says something moved (`quince://update`), and again when the window comes back to the front —
 * which is how the notice moves to the window in use when the one showing it is put away.
 */
export function useUpdateStatus(): [UpdateStatus | null, (s: UpdateStatus) => void] {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  useEffect(() => {
    let alive = true;
    const read = () =>
      api.updateStatus().then(
        (s) => alive && setStatus(s),
        () => {},
      );
    void read();
    const stop = listen(UPDATE, () => void read());
    const back = () => void read();
    window.addEventListener("focus", back);
    return () => {
      alive = false;
      window.removeEventListener("focus", back);
      void stop.then((u) => u()).catch(() => {});
    };
  }, []);
  return [status, setStatus];
}

/*
 * Whether someone is typing in this window: a key pressed in a text field in the last few
 * seconds, or a note with words not yet saved (NotesView holds it). A restart to install an
 * update waits for every window to be clear.
 */
const TYPING_MS = 5000;
const held = new Set<string>();
let lastKey = 0;
const watchers = new Set<() => void>();
const tell = () => watchers.forEach((w) => w());

function editable(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  return Boolean(el?.closest?.("input, textarea, [contenteditable=''], [contenteditable='true'], [contenteditable='plaintext-only']"));
}

if (typeof document !== "undefined") {
  const typed = (e: Event) => {
    if (!editable(e.target)) return;
    const was = typingNow();
    lastKey = Date.now();
    if (!was) tell();
  };
  document.addEventListener("keydown", typed, true);
  document.addEventListener("input", typed, true);
}

/** Hold "typing" under `key` while `on` (a note with words not yet saved). */
export function holdTyping(key: string, on: boolean) {
  const had = held.has(key);
  if (on) held.add(key);
  else held.delete(key);
  if (had !== on) tell();
}

function typingNow(): boolean {
  return held.size > 0 || Date.now() - lastKey < TYPING_MS;
}

/**
 * While an update could restart Skykeep, tell the backend whether this window is being typed
 * in: at once when it starts, again every few seconds while it lasts (a window that stops
 * saying so is taken as clear), and when it ends.
 */
export function useTypingReport(active: boolean) {
  useEffect(() => {
    if (!active) return undefined;
    let said: boolean | null = null;
    const say = () => {
      const now = typingNow();
      if (now || said !== now) void api.updateTyping(now).catch(() => {});
      said = now;
    };
    say();
    watchers.add(say);
    const timer = window.setInterval(say, 4000);
    return () => {
      watchers.delete(say);
      window.clearInterval(timer);
      if (said) void api.updateTyping(false).catch(() => {});
    };
  }, [active]);
}
