import { t } from "./i18n";

/**
 * A failed update in a few words, for a line that has room for only that: the Home tiles and an
 * app's own sidebar foot. The whole sentence goes in the line's tooltip and in what it says to a
 * screen reader. The sentence as the line itself ("iCloud is busy right now. Nothing was
 * changed — try again in a minute.") was cut off mid-word wherever it stood.
 */
export function shortFailure(detail: string | null | undefined): string {
  const d = (detail ?? "").toLowerCase();
  if (d.includes("busy")) return t("launcher.busyRetrying");
  if (d.includes("connection") || d.includes("offline") || d.includes("reach") || d.includes("took too long")) return t("launcher.offlineRetrying");
  return t("launcher.didntUpdate");
}
