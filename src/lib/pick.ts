import { invoke } from "@tauri-apps/api/core";

/**
 * The system's open and save dialogs, shown by the app's Rust side. A file chosen here is the
 * only kind (with one dropped on the window, or opened with Skykeep) that the commands which
 * read or write files will accept: the window itself cannot name a file (src-tauri/src/grants.rs).
 */
type PickFilter = { name: string; extensions: string[] };

/** What was chosen, or null when the dialog was cancelled. */
export async function pickOpen(options: {
  title?: string;
  multiple?: boolean;
  directory?: boolean;
  filters?: PickFilter[];
}): Promise<string[] | null> {
  const chosen = await invoke<string[] | null>("pick_open", {
    title: options.title ?? null,
    multiple: options.multiple ?? false,
    directory: options.directory ?? false,
    filters: options.filters ?? null,
  });
  return chosen && chosen.length > 0 ? chosen : null;
}

/** Where to save, or null when the dialog was cancelled. Good for one write. */
export function pickSave(options: { defaultName?: string; filters?: PickFilter[]; title?: string }) {
  return invoke<string | null>("pick_save", {
    defaultName: options.defaultName ?? null,
    filters: options.filters ?? null,
    title: options.title ?? null,
  });
}
