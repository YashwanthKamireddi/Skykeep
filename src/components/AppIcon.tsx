import { CalendarDays, ContactRound, FolderClosed, Image as ImageGlyph, ListChecks, NotebookPen, Smartphone } from "lucide-react";

/**
 * Each app's mark in the sidebar and Settings, as a Mac sidebar marks its places (Finder, Mail):
 * one clean symbol, tinted in the app's own colour, on no tile at all. A row of colourful little
 * tiles read as a toy; a single-colour symbol reads as the system's own.
 */
const MARK: Record<string, { glyph: typeof NotebookPen; tint: string }> = {
  notes: { glyph: NotebookPen, tint: "var(--app-notes)" },
  reminders: { glyph: ListChecks, tint: "var(--app-reminders)" },
  calendar: { glyph: CalendarDays, tint: "var(--app-calendar)" },
  contacts: { glyph: ContactRound, tint: "var(--app-contacts)" },
  drivews: { glyph: FolderClosed, tint: "var(--app-drive)" },
  drive: { glyph: FolderClosed, tint: "var(--app-drive)" },
  photos: { glyph: ImageGlyph, tint: "var(--app-photos)" },
  iphone: { glyph: Smartphone, tint: "var(--accent)" },
};

export function AppIcon({ app, size = 22 }: { app: string; size?: number }) {
  const mark = MARK[app];
  if (!mark) return null;
  const Glyph = mark.glyph;
  return (
    <span className={`app-icon ai-${app}`} style={{ width: size, height: size }} aria-hidden="true">
      <Glyph size={Math.round(size * 0.82)} strokeWidth={1.9} color={mark.tint} />
    </span>
  );
}
