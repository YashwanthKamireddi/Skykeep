/**
 * Where each app was left: the folder, list or group open and the item open in it, kept on this
 * computer so the app reopens there, as Apple's own apps do. A closed app window is let go after
 * a minute (apps.rs KEPT), and opened again it was back at All with nothing open. Each app checks
 * that what it finds here still exists before it uses it: a folder deleted meanwhile falls back
 * to the app's own start.
 */
export type Place<C> = { container: C; item: string | null };

const key = (app: string) => `quince.${app}.place`;

/** Where `app` was left, or null; `container` says whether what is stored is shaped right. */
export function keptPlace<C>(app: string, container: (c: unknown) => c is C): Place<C> | null {
  try {
    const kept = JSON.parse(window.localStorage.getItem(key(app)) ?? "null");
    if (!kept || typeof kept !== "object" || !container(kept.container)) return null;
    return { container: kept.container, item: typeof kept.item === "string" ? kept.item : null };
  } catch {
    return null;
  }
}

export function keepPlace<C>(app: string, place: Place<C>) {
  try {
    window.localStorage.setItem(key(app), JSON.stringify(place));
  } catch {
    // Reopens at the app's start instead.
  }
}

export const isText = (c: unknown): c is string => typeof c === "string";
