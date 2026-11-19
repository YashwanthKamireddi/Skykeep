/**
 * What a row in Drive is, drawn rather than typed.
 *
 * A folder used to be the character "▸" and a file was its extension in capitals inside a
 * grey box. Both render at whatever weight and baseline the font feels like, so a list of
 * them never looked like one set of icons. These are one grid, one stroke weight, and they
 * carry the only distinction that matters at a glance: a folder you go into, a file you open,
 * and roughly what kind of file it is.
 */

/** The families Apple's own file browser tints apart. */
export type FileFamily =
  | "folder"
  | "app"
  | "image"
  | "video"
  | "audio"
  | "document"
  | "sheet"
  | "slides"
  | "archive"
  | "code"
  | "other";

const BY_EXTENSION: Record<string, FileFamily> = {
  jpg: "image", jpeg: "image", png: "image", gif: "image", heic: "image",
  webp: "image", tiff: "image", bmp: "image", svg: "image", raw: "image", dng: "image",
  mp4: "video", mov: "video", m4v: "video", avi: "video", mkv: "video", webm: "video",
  mp3: "audio", m4a: "audio", wav: "audio", aac: "audio", flac: "audio", aiff: "audio",
  pdf: "document", doc: "document", docx: "document", pages: "document",
  txt: "document", md: "document", rtf: "document", epub: "document",
  xls: "sheet", xlsx: "sheet", numbers: "sheet", csv: "sheet",
  ppt: "slides", pptx: "slides", key: "slides",
  zip: "archive", gz: "archive", tar: "archive", rar: "archive", "7z": "archive",
  js: "code", ts: "code", tsx: "code", jsx: "code", py: "code", rs: "code",
  json: "code", html: "code", css: "code", sh: "code", go: "code", java: "code",
};

/** Tints kept apart enough to tell at 20px, and legible on both a light and a dark ground. */
const TINT: Record<FileFamily, string> = {
  folder: "#3b82f6",
  app: "#8b5cf6",
  image: "#ec4899",
  video: "#6366f1",
  audio: "#f97316",
  document: "#ef4444",
  sheet: "#22c55e",
  slides: "#f59e0b",
  archive: "#a16207",
  code: "#0ea5e9",
  other: "#94a3b8",
};

export function familyOf(kind: string, extension: string | null): FileFamily {
  if (kind === "FOLDER") return "folder";
  if (kind === "APP_LIBRARY") return "app";
  return BY_EXTENSION[(extension ?? "").toLowerCase()] ?? "other";
}

/** A closed folder, the way a file browser draws one. */
const FOLDER = (
  <path
    d="M2.5 5.2c0-.9.7-1.6 1.6-1.6h2.6c.5 0 1 .2 1.3.6l.8.9h5.1c.9 0 1.6.7 1.6 1.6v6.1c0 .9-.7 1.6-1.6 1.6H4.1c-.9 0-1.6-.7-1.6-1.6z"
    fill="currentColor"
    stroke="none"
  />
);

/** A sheet of paper with the corner turned, which is what every file browser means by a file. */
const PAGE = (
  <>
    <path
      d="M4 2.6h5.1L14 7.4v7.1c0 .6-.5 1.1-1.1 1.1H4c-.6 0-1.1-.5-1.1-1.1V3.7c0-.6.5-1.1 1.1-1.1z"
      fill="currentColor"
      fillOpacity="0.16"
      stroke="currentColor"
      strokeWidth="1.2"
    />
    <path d="M9 2.7v3.6c0 .6.5 1.1 1.1 1.1h3.6" fill="none" stroke="currentColor" strokeWidth="1.2" />
  </>
);

/** A stack of panes, for a bundle the phone treats as one app's store. */
const APP = (
  <>
    <rect x="2.6" y="2.6" width="5.4" height="5.4" rx="1.5" fill="currentColor" stroke="none" />
    <rect x="10" y="2.6" width="5.4" height="5.4" rx="1.5" fill="currentColor" fillOpacity="0.55" stroke="none" />
    <rect x="2.6" y="10" width="5.4" height="5.4" rx="1.5" fill="currentColor" fillOpacity="0.55" stroke="none" />
    <rect x="10" y="10" width="5.4" height="5.4" rx="1.5" fill="currentColor" fillOpacity="0.3" stroke="none" />
  </>
);

export function FileGlyph({ kind, extension }: { kind: string; extension: string | null }) {
  const family = familyOf(kind, extension);
  return (
    <svg
      className="file-glyph"
      viewBox="0 0 18 18"
      width="18"
      height="18"
      style={{ color: TINT[family] }}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {family === "folder" ? FOLDER : family === "app" ? APP : PAGE}
    </svg>
  );
}

/** Actions on a row, at the same weight as everything else drawn here. */
export type ActionName = "download" | "rename" | "move" | "delete" | "upload" | "newFolder" | "info";

const ACTIONS: Record<ActionName, React.ReactNode> = {
  download: <path d="M8 2.8v7.4M4.8 7.2 8 10.4l3.2-3.2M3.2 12.6h9.6" />,
  rename: <path d="m10.4 3.2 2.4 2.4-7 7H3.4V10.2zM9.2 4.4l2.4 2.4" />,
  move: <path d="M2.6 5.2a1.4 1.4 0 0 1 1.4-1.4h2.3l1 1.2h5.1a1.4 1.4 0 0 1 1.4 1.4M6.4 10.6h6.4M10.4 8.2l2.4 2.4-2.4 2.4" />,
  delete: <path d="M3.4 4.8h9.2M6.4 4.8V3.4h3.2v1.4M5 4.8l.6 8h4.8l.6-8M7 7v3.6M9 7v3.6" />,
  upload: <path d="M8 13.2V5.8M4.8 9 8 5.8 11.2 9M3.2 3.4h9.6" />,
  info: <path d="M8 13.4A5.4 5.4 0 1 0 8 2.6a5.4 5.4 0 0 0 0 10.8zM8 7.4v3.4M8 5.2v.1" />,
  newFolder: <path d="M2.6 5.2a1.4 1.4 0 0 1 1.4-1.4h2.3l1 1.2h5.1a1.4 1.4 0 0 1 1.4 1.4v5.4a1.4 1.4 0 0 1-1.4 1.4H4a1.4 1.4 0 0 1-1.4-1.4zM8 7.6v3.4M6.3 9.3h3.4" />,
};

export function ActionGlyph({ name }: { name: ActionName }) {
  return (
    <svg
      viewBox="0 0 16 16"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.35"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {ACTIONS[name]}
    </svg>
  );
}
