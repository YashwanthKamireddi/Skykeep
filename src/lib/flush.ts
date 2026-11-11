/**
 * What a window still holds in memory, written down before the window or Skykeep goes.
 *
 * Notes saves a pause after the last key (NotesView), so for about a second what was typed
 * lives only in the page. Quit ends the process with `_exit`, sign-out destroys the app
 * windows, and closing a window where there is no tray closes it for real: none of them run
 * React's cleanup, and the last words typed were lost (desktop audit C1). So the backend asks
 * every window first (`FLUSH`, from apps.rs `flush_windows`) and waits a moment for each to
 * answer; a screen with something unsaved registers here what writing it down means.
 *
 * Every window answers, saved or not, so the wait is only ever as long as the slowest save.
 */
import { listen } from "@tauri-apps/api/event";
import { api, FLUSH } from "./ipc";

const flushers = new Set<() => Promise<unknown> | unknown>();

/** Write down what `fn` holds whenever the window is asked to; the returned function stops it. */
export function onFlush(fn: () => Promise<unknown> | unknown): () => void {
  flushers.add(fn);
  return () => {
    flushers.delete(fn);
  };
}

/** Every registered save, at once; one that fails does not keep the others from finishing. */
export async function flushAll(): Promise<void> {
  await Promise.allSettled([...flushers].map(async (fn) => fn()));
}

/** Answer the backend's asks for the life of the window. Called once, from main.tsx. */
export function answerFlushes() {
  void listen<number>(FLUSH, async (e) => {
    await flushAll();
    await api.windowFlushed(e.payload).catch(() => {});
  }).catch(() => {});
}
