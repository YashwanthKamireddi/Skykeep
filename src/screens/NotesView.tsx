import { Fragment, memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AppFoot } from "../components/AppFoot";
import { usePending } from "../lib/pending";
import { useErrorToast } from "../components/Toast";
import { useConfirm } from "../components/Confirm";
import { EmptyState } from "../components/EmptyState";
import { SearchField } from "../components/SearchField";
import { PaneTitle } from "../components/PaneTitle";
import { Unsent, Waiting } from "../components/Unsent";
import { useOutbox } from "../lib/outbox";
import { ArrowDownUp, ChevronLeft, Folder, FolderOpen, FolderPlus, ListChecks, LockKeyhole, LockKeyholeOpen, Paperclip, Pin, PinOff, Share, SquarePen, Trash2 } from "lucide-react";
import { pickOpen, pickSave } from "../lib/pick";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { DEFAULT_ORDER, editorDate, groupNotes, listDate, mergeEdits, splitTitle, type NoteOrder, type Section } from "./notes/model";
import { api, messageOf, nestFolders, noteMediaSrc, notesLockApi, type Note, type NoteFolder, type UnlockedNote } from "../lib/ipc";
import { LockScreen } from "./notes/LockedNote";
import { fileName, useAttachEnabled } from "./notes/attach";
import { toast } from "../components/Toast";
import { overlayOpen, useActiveRef } from "../lib/screen";
import { RowMenu } from "../components/RowMenu";
import { Spinner } from "../components/Spinner";
import { FormattedBody, type Run } from "./notes/Formatted";
import { carryRuns, runsFit, sameFormatting } from "./notes/rich";
import { NoteEditor, PICTURE_KEPT_EVENT, refreshTables, type EditorHandle, type EditorStatus } from "./notes/NoteEditor";
import { FormatPanel } from "./notes/FormatPanel";
import { NotePicture } from "./notes/NotePicture";
import { OPEN_NOTE_EVENT } from "./notes/links";
import { useReveal } from "../lib/reveal";
import { Arriving } from "../components/EmptyState";
import { useArriving, useRevision, useShownRevision } from "../lib/sync";
import { Picker } from "../components/Picker";
import { holdTyping } from "../lib/updates";
import { keepSame, same } from "../lib/same";
import { useChosen, useIsChosen, type Chosen } from "../lib/chosen";
import { useArrivals } from "../lib/arrival";
import { foundRanges, queryWords } from "./notes/found";

const ALL = "__all__";

/** A new note's first line set as its title, as Notes sets it, unless it was given a style. */
function titled(text: string, runs: Run[]): Run[] {
  const end = text.indexOf("\n");
  const firstLine = end < 0 ? text.length : end + 1;
  const out: Run[] = [];
  let at = 0;
  for (const r of runs) {
    const from = at;
    at += r.length;
    if (from >= firstLine || r.style !== null) { out.push(r); continue; }
    const inside = Math.min(at, firstLine) - from;
    out.push({ ...r, length: inside, style: 0 });
    if (r.length > inside) out.push({ ...r, length: r.length - inside });
  }
  return out;
}

/** Apple's Recently Deleted, by the record name that is the same in every language. */
const TRASH_FOLDER = "TrashFolder-CloudKit";
/** Apple's own "Notes" folder: it can be neither renamed nor deleted. */
const DEFAULT_FOLDER = "DefaultFolder-CloudKit";

/** A note's title or snippet as words: without the marks where attachments sit (U+FFFC) or
 *  a line breaks (U+2028), which drew as boxes. The backend strips them too; this covers
 *  what was stored before it did. */
const wordsOnly = (text: string) => text.replace(/[\uFFFC\u2028]/g, " ").replace(/\s+/g, " ").trim();

const FOLDERS_KEY = "quince.notes.foldersWidth";
const clampFolders = (w: number) => Math.round(Math.max(160, Math.min(420, w)));
function keptFoldersWidth(): number | null {
  try {
    const n = Number(window.localStorage.getItem(FOLDERS_KEY));
    return n > 0 ? clampFolders(n) : null;
  } catch {
    return null;
  }
}
function keepFoldersWidth(w: number) {
  try {
    window.localStorage.setItem(FOLDERS_KEY, String(w));
  } catch {
    /* a width not kept is only a width not kept */
  }
}
/** The width that shows every folder's whole name: the widest row's name as it would run
 * untruncated, with the row's icon, count and padding around it. */
function fitFolders(aside: HTMLElement | null): number | null {
  if (!aside) return null;
  let widest = 0;
  aside.querySelectorAll<HTMLElement>(".notes-folder").forEach((row) => {
    const name = row.querySelector<HTMLElement>(".notes-folder-name");
    if (!name) return;
    const others = row.scrollWidth - name.clientWidth;
    widest = Math.max(widest, others + name.scrollWidth);
  });
  if (!widest) return null;
  const pad = parseFloat(getComputedStyle(aside).paddingLeft) + parseFloat(getComputedStyle(aside).paddingRight);
  return clampFolders(widest + pad + 2);
}

/**
 * The list of notes, drawn again only when the notes change. Inline in the screen, every key
 * typed in a note — and every move of the cursor — drew every row again, which in an account
 * of thousands was felt in the typing. The open note is not one of its props either: each row
 * asks `chosen` itself (lib/chosen), so the arrows draw the two rows they move between.
 */
const NoteRows = memo(function NoteRows({
  sections,
  chosen,
  rowsRef,
  onPick,
  found,
  pictures,
}: {
  sections: Section[];
  /** The open note, as each row asks it. */
  chosen: Chosen;
  rowsRef: React.MutableRefObject<HTMLDivElement | null>;
  onPick: (note: Note, e: React.MouseEvent) => void;
  /** The words being searched for, marked where they fall in each row. */
  found: string[];
  /** Each note's first picture kept on this computer, by note (`api.notePictures`). */
  pictures: Record<string, string>;
}) {
  return (
    <div className="note-rows" ref={rowsRef} role="listbox" aria-label="Notes" data-rows="">
      {sections.map((section) => (
        <Fragment key={section.key}>
          <div className="notes-section" aria-hidden="true">{section.heading}</div>
          <div role="group" aria-label={section.heading}>
          {section.notes.map((note) => (
            <NoteRow
              key={note.guid}
              note={note}
              chosen={chosen}
              picture={pictures[note.guid] ?? null}
              onPick={onPick}
              found={found}
            />
          ))}
          </div>
        </Fragment>
      ))}
    </div>
  );
});

/**
 * One note in the list, drawn again only when something it shows changes: its note, whether
 * it is the one open, its picture. Moving the selection with the arrow keys draws the two rows
 * it moved between, not every row in the folder.
 *
 * Each note is one element, the row itself: wrapped in a box of its own, every row was two
 * boxes deep, and the separator had to reach across from one to the next.
 */
const NoteRow = memo(function NoteRow({
  note,
  chosen,
  picture,
  onPick,
  found,
}: {
  note: Note;
  chosen: Chosen;
  picture: string | null;
  onPick: (note: Note, e: React.MouseEvent) => void;
  found: string[];
}) {
  const selected = useIsChosen(chosen, note.guid);
  return (
    <button
      className="note-row"
      role="option"
      data-guid={note.guid}
      aria-selected={selected}
      onClick={(e) => onPick(note, e)}
    >
      <span className="note-row-title">
        {note.locked && (
          <LockKeyhole className="note-row-lock" size={12} strokeWidth={2.4} aria-label="Locked" />
        )}
        <span className="note-row-name"><Found text={wordsOnly(note.title) || "New Note"} words={found} /></span>
      </span>
      <span className="note-row-meta">
        <span className="note-row-when">{listDate(note.modifiedAt)}</span>
        <span className="note-row-snippet">
          {note.locked ? "Locked" : <Found text={wordsOnly(note.snippet ?? "").slice(0, 120) || "No additional text"} words={found} />}
        </span>
      </span>
      {/* The note's first picture, small, at the row's end, as Notes on the Mac shows it —
          only one already kept on this computer, so a row never waits on Apple for it. */}
      {picture && <NoteThumb key={picture} id={picture} />}
    </button>
  );
});

/**
 * A note's small picture in the list. It takes the row's end only once it is decoded — never
 * a blank square first, and never a broken one: a picture this computer no longer holds is
 * not shown. It sits inside the row's own height, so a row that gains one never grows; the
 * title and snippet just end sooner, as in Notes.
 */
function NoteThumb({ id }: { id: string }) {
  const [state, setState] = useState<"loading" | "shown" | "gone">("loading");
  if (state === "gone") return null;
  return (
    <img
      className="note-row-thumb"
      src={noteMediaSrc(id, "thumb")}
      alt=""
      draggable={false}
      decoding="async"
      hidden={state !== "shown"}
      onLoad={() => setState("shown")}
      onError={() => setState("gone")}
    />
  );
}

/** `text` with the searched-for words marked, as Notes marks them in its results. */
function Found({ text, words }: { text: string; words: string[] }) {
  const ranges = words.length ? foundRanges(text, words) : [];
  if (!ranges.length) return <>{text}</>;
  const out: React.ReactNode[] = [];
  let from = 0;
  ranges.forEach(([a, b], i) => {
    if (a > from) out.push(text.slice(from, a));
    out.push(<mark key={i} className="note-found">{text.slice(a, b)}</mark>);
    from = b;
  });
  if (from < text.length) out.push(text.slice(from));
  return <>{out}</>;
}

export function NotesView() {
  // The folders' column: as wide as it was left, or — the first time — as wide as the longest
  // folder name needs, within reason.
  const notesRef = useRef<HTMLDivElement | null>(null);
  const foldersRef = useRef<HTMLElement | null>(null);
  const [foldersWidth, setFoldersWidth] = useState<number | null>(keptFoldersWidth);

  const [folders, setFolders] = useState<NoteFolder[]>([]);
  const kept = useRef(foldersWidth !== null);
  useLayoutEffect(() => {
    if (kept.current) return;
    const fit = fitFolders(foldersRef.current);
    // Never narrower than the column was made, and never more than a third of a wide window.
    if (fit) setFoldersWidth(Math.max(208, Math.min(fit, 320)));
  }, [folders]);
  const [folderMenu, setFolderMenu] = useState<{ folder: NoteFolder; x: number; y: number; alignRight: boolean } | null>(null);
  // How the list is ordered (View menu), kept on this computer: a convenience, so a storage
  // that is missing or full only means the default order.
  const [listOrder, setListOrder] = useState<NoteOrder>(() => {
    try {
      const kept = JSON.parse(localStorage.getItem("notes.order") ?? "null");
      if (kept && ["edited", "created", "title"].includes(kept.by) && typeof kept.groupByDate === "boolean") return kept;
    } catch {
      /* the default */
    }
    return DEFAULT_ORDER;
  });
  const orderBy = (next: NoteOrder) => {
    setListOrder(next);
    try {
      localStorage.setItem("notes.order", JSON.stringify(next));
    } catch {
      /* kept for this session only */
    }
  };
  const [viewMenu, setViewMenu] = useState<{ x: number; y: number; alignRight: boolean } | null>(null);
  const [renamingFolder, setRenamingFolder] = useState<string | null>(null);
  const [folder, setFolder] = useState<string>(ALL);
  const [notes, setNotes] = useState<Note[]>([]);
  const [openGuid, setOpenGuid] = useState<string | null>(null);
  /** `openGuid` as last drawn, for work that finishes later — a save landing — to tell whether
   *  the note it was for is still the one on screen. */
  const openGuidRef = useRef(openGuid);
  openGuidRef.current = openGuid;
  /**
   * In a narrow window only one column shows, as on iPhone: the list, or — once a note is
   * picked or begun — that note, with a way back. A wide window shows both and ignores this.
   */
  const [detail, setDetail] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useErrorToast(error, () => setError(null));
  const [confirm, confirmSheet] = useConfirm();
  /** Moves with every pass that changed notes, shown or not: an edit waiting goes then. */
  const synced = useRevision("notes");
  // What the screen reads by: held while another screen is in front, and read once on coming
  // back (lib/sync).
  const revision = useShownRevision("notes");
  const arriving = useArriving("notes");
  // Tables on screen keep up with a phone's edits, as the note's text does.
  useEffect(() => {
    if (revision) refreshTables();
  }, [revision]);
  /**
   * The body being typed, held apart from the note until it is saved — with the note it was
   * typed in. Only that note's editor ever shows it: held bare, the text typed in one note was
   * still the draft for the first frame of the next, and a note read ahead opened in that very
   * frame — its editor made with the other note's words, which the next key then saved over it.
   */
  const [drafted, setDrafted] = useState<{ guid: string; text: string } | null>(null);
  const draft = drafted !== null && drafted.guid === openGuid ? drafted.text : null;
  /** `draft`, for effects that read it without re-running on every keystroke. */
  const draftRef = useRef<string | null>(null);
  draftRef.current = draft;
  /** Counts every change made in the editor, formatting as much as text. */
  const [edits, setEdits] = useState(0);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);

  /** What is typed in the search field. While it holds anything, the list is the results. */
  const [query, setQuery] = useState("");
  const searching = query.trim().length > 0;
  /**
   * Where a search looks: every folder, as Notes' search does (Ctrl+Shift+F), or only the
   * folder in front (Ctrl+F). Means nothing in All iCloud, which is every folder already.
   */
  const [scope, setScope] = useState<"all" | "folder">("all");
  const scoped = searching && scope === "folder" && folder !== ALL;
  const searchRef = useRef<HTMLInputElement | null>(null);
  const rowsRef = useRef<HTMLDivElement | null>(null);

  /** Notes outside Recently Deleted, and how many each folder holds. */
  const [counts, setCounts] = useState<{ all: number; per: Map<string, number> }>({
    all: 0,
    per: new Map(),
  });

  const { hold, over } = usePending<Note>();
  /** Edits kept here and not yet at Apple, and any Apple turned down. */
  const outbox = useOutbox("notes");
  /** Each read is numbered: one overtaken by a later one (a key typed since) never draws. */
  const reads = useRef(0);
  const read = useCallback(async () => {
    const mine = ++reads.current;
    api
      .noteCounts()
      .then(([all, per]) => setCounts({ all, per: new Map(per) }))
      .catch(() => {
        // Counts are a courtesy; the list is what matters.
      });
    const [found, list] = await Promise.all([
      api.noteFolders(),
      // Search reaches every folder, as Apple's does — a note is found wherever it is filed.
      searching
        ? api.notesSearch(query).then((found) =>
            scoped && Array.isArray(found) ? found.filter((n) => n.folderGuid === folder) : found)
        : api.notesIn(folder === ALL ? null : folder),
    ]);
    if (mine !== reads.current) return;
    setFolders(Array.isArray(found) ? found : []);
    // Each note unchanged keeps its row as it was (lib/same).
    setNotes((was) => keepSame(was, over(Array.isArray(list) ? list : []), (n) => n.guid));
    setLoaded(true);
  }, [folder, query, searching, scoped, over]);

  // Reads the library and nothing else. `revision` ticks when a refresh finishes, whether
  // someone pressed for it or the timer came round, and that is the screen's whole
  // relationship with the network.
  useEffect(() => {
    let cancelled = false;
    read().catch((e) => {
      if (!cancelled) {
        setError(messageOf(e));
        setLoaded(true);
      }
    });
    return () => {
      cancelled = true;
    };
    // `outbox.tick`: an edit sent, or turned down, changes what the library holds.
  }, [read, revision, outbox.tick]);

  /**
   * Each note's first picture this computer already holds, for the small picture at the end of
   * its row. Asked again when the library moves and when a note draws a picture (and so keeps
   * it) — a burst of pictures in one note is one ask — and kept as it was when nothing changed,
   * so no row is drawn again for it.
   */
  const [pictures, setPictures] = useState<Record<string, string>>({});
  useEffect(() => {
    let live = true;
    let timer: number | undefined;
    const ask = () => {
      api.notePictures().then(
        (found) => live && found && setPictures((was) => (same(was, found) ? was : found)),
        () => {
          // Only the small pictures are missing; the list itself is unaffected.
        },
      );
    };
    const kept = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(ask, 120);
    };
    ask();
    window.addEventListener(PICTURE_KEPT_EVENT, kept);
    return () => {
      live = false;
      window.clearTimeout(timer);
      window.removeEventListener(PICTURE_KEPT_EVENT, kept);
    };
  }, [revision, outbox.tick]);

  /**
   * Every write goes to Apple first and the library is re-read after, so nothing on screen
   * is a guess about what was stored.
   */
  /**
   * A pin, a move, a delete shows the moment it is asked for, as on the phone; then it goes
   * to Apple and the list is re-read. A refusal re-reads, which puts the note back, and says
   * why.
   */
  const write = useCallback(
    async (run: () => Promise<void>, predict?: (rows: Note[]) => Note[]) => {
      setBusy(true);
      setError(null);
      if (predict) setNotes((rows) => predict(rows));
      // Kept over any refresh until Apple has it; let go before the read that confirms it.
      const release = predict ? hold(predict) : () => {};
      try {
        await run();
        release();
        await read();
        return true;
      } catch (e) {
        release();
        setError(messageOf(e));
        read().catch(() => {});
        return false;
      } finally {
        release();
        setBusy(false);
      }
    },
    [read, hold]
  );

  /** Mirrors `composing` for effects that must not re-run when it changes. */
  const composingRef = useRef<{ folderGuid: string | null; body: string } | null>(null);

  // Keep a note open across refreshes where possible; fall back to the first — but never
  // while a new note is being written, which has nothing open behind it on purpose.
  useEffect(() => {
    if (composingRef.current) return;
    if (notes.length === 0) return setOpenGuid(null);
    setOpenGuid((current) =>
      current && notes.some((n) => n.guid === current) ? current : notes[0].guid
    );
  }, [notes]);

  // A link to another note, followed: that note opened — from All iCloud when the folder in
  // front does not hold it — or, when it is not in this account's notes, said so.
  const following = useRef<string | null>(null);
  const follow = (guid: string) => {
    if (notes.some((n) => n.guid === guid)) {
      setOpenGuid(guid);
      setDetail(true);
      return;
    }
    following.current = guid;
    if (folder !== ALL) setFolder(ALL);
    else void read();
  };
  const followRef = useRef(follow);
  followRef.current = follow;
  useEffect(() => {
    const on = (e: Event) => followRef.current((e as CustomEvent<string>).detail);
    window.addEventListener(OPEN_NOTE_EVENT, on);
    return () => window.removeEventListener(OPEN_NOTE_EVENT, on);
  }, []);
  // A note found by the search across everything: opened as a link to it opens it.
  useReveal("notes", (r) => followRef.current(r.id));
  useEffect(() => {
    const guid = following.current;
    if (!guid || folder !== ALL || notes.length === 0) return;
    following.current = null;
    if (notes.some((n) => n.guid === guid)) {
      setOpenGuid(guid);
      setDetail(true);
    } else {
      setError("That note isn’t in your notes. It may have been deleted, or it belongs to someone else.");
    }
  }, [notes, folder]);

  const open = useMemo(
    () => notes.find((n) => n.guid === openGuid) ?? null,
    [notes, openGuid]
  );

  // A different note means a different draft. Switching away from unsaved typing would
  // otherwise carry it into the next note and save it over that one.
  //
  // Except for the note just created from what is being typed: that is the same note, and
  // what was typed while it was being created is still to be saved onto it.
  const adopting = useRef<string | null>(null);
  useEffect(() => {
    if (adopting.current !== null && adopting.current === openGuid) {
      adopting.current = null;
      return;
    }
    setDrafted((d) => (d !== null && d.guid === openGuid ? d : null));
    setSaved(false);
  }, [openGuid]);

  /**
   * The open note's text, fetched on its own.
   *
   * A listed note carries no body, so this is the only thing that knows what the note says —
   * and until it arrives nothing may be edited or saved. An editor showing an empty box over
   * a note whose text had not loaded yet would write that emptiness back the moment anyone
   * touched it, which is the worst thing this app could do.
   *
   * Kept beside the guid it belongs to, so a slow fetch for one note can never be mistaken
   * for the body of the next one opened.
   */
  const [loadedBody, setLoadedBody] = useState<{ guid: string; text: string } | null>(null);

  /** How the open note is set, from Apple's attribute runs, read with its body. */
  const [loadedRuns, setLoadedRuns] = useState<{ guid: string; runs: Run[] } | null>(null);
  const loadedRunsRef = useRef(loadedRuns);
  loadedRunsRef.current = loadedRuns;
  /** The line the cursor is in and its marks, for the toolbar to show as on. */
  const [status, setStatus] = useState<EditorStatus | null>(null);
  const [formatAt, setFormatAt] = useState<{ x: number; y: number } | null>(null)

  /**
   * Saving is not something anyone should have to remember.
   *
   * Apple's Notes has no save button because a note is not a document you commit — it is
   * just there, and what you typed is what it says. Typing schedules a save; leaving the
   * note or the field takes it immediately, so nothing is ever waiting on a timer when you
   * walk away.
   */
  const pending = useRef<{ guid: string; body: string; runs: Run[] | null } | null>(null);

  /**
   * The notes beside the open one, read ahead — text and formatting — so stepping to one with
   * the arrow keys draws it in the same frame, rather than a frame of "Opening…" and then the
   * note. Only ever what the library holds after every save so far (read behind them), let go
   * of whenever the library changes, and a note typed into is never taken from here.
   */
  const ahead = useRef(new Map<string, { text: string; runs: Run[] }>());
  /** Moves each time the library changes, so a read ahead begun before it is not kept. */
  const aheadAt = useRef(0);
  useEffect(() => {
    ahead.current.clear();
    aheadAt.current += 1;
  }, [revision, outbox.tick]);

  // The words and their formatting are read together and arrive together: read one after the
  // other, the note was drawn plain and then again formatted — a flash on every note opened.
  useEffect(() => {
    setFormatAt(null);
    if (!openGuid) {
      setLoadedBody(null);
      setLoadedRuns(null);
      return undefined;
    }
    let cancelled = false;
    setLoadedBody((current) => (current?.guid === openGuid ? current : null));
    Promise.all([
      api.noteBody(openGuid),
      // Unreadable formatting still opens the note — as text only (formatSafe stays false),
      // which saves the words and leaves Apple's formatting as it is.
      api.noteRuns(openGuid).then((runs) => (Array.isArray(runs) ? runs : []), () => null),
    ])
      .then(([text, runs]) => {
        if (cancelled) return;
        setLoadedRuns((cur) => (runs !== null ? { guid: openGuid, runs } : cur?.guid === openGuid ? cur : { guid: openGuid, runs: [] }));
        // The library holds no text for it: not yet arrived, or a note Skykeep can't read.
        // Said as such, never an "Opening…" that never ends; the next read after a sync
        // replaces it the moment the text is there.
        setNoText(text === null ? openGuid : null);
        if (text === null) return;
        // Never over what is being typed. A refresh follows every save, and adopting its
        // answer under someone's cursor is how a half-saved note replaced a whole one. Typing
        // already kept, with nothing new since, is not being typed: what Apple made of it —
        // merged with another device's edit, once sent — is taken in.
        setDrafted((d) => {
          const typing = d !== null && d.guid === openGuid ? d.text : null;
          const settled = typing !== null && !pending.current && base.current?.guid === openGuid && typing === base.current.text;
          if (typing === null || settled) setLoadedBody({ guid: openGuid, text });
          return d;
        });
      })
      .catch((e) => {
        if (!cancelled) setError(messageOf(e));
      });
    return () => {
      cancelled = true;
    };
  }, [openGuid, revision, outbox.tick]);


  /** What the open note says, or null while that is still unknown. Read ahead, it is known at
   *  once (`ahead`) — until its own read lands. */
  const bodyIn = open !== null && loadedBody?.guid === open.guid;
  // Never for a note with an edit still waiting to go: only its own read says what it holds.
  const early = open && !bodyIn && pending.current?.guid !== open.guid ? ahead.current.get(open.guid) : undefined;
  const body = bodyIn ? loadedBody!.text : (early?.text ?? null);
  const runs = bodyIn ? (loadedRuns?.guid === open!.guid ? loadedRuns.runs : null) : (early?.runs ?? null);
  /**
   * Whether this note's formatting can be written from the editor: only when the runs read
   * describe the text exactly. Otherwise the editor shows it plain and saves the text alone,
   * which keeps Apple's formatting where it is — never a plain copy written over it.
   */
  const formatSafe = body !== null && runsFit(body, runs);
  /** The editor, to hand it what Apple holds and to run the toolbar's commands. */
  const editorRef = useRef<EditorHandle | null>(null);
  /** A new note's id once it is made, so the editor it was typed in stays the one shown. */
  const adoptedKey = useRef<string | null>(null);
  /** Which New Note this is: each gets an editor of its own, so a second New Note can never
   *  start out holding the first one's text (and save it again as a copy). */
  const composeSeq = useRef(0);
  /** The formatting on screen now, beside `draft` — the text on screen now. */
  const draftRuns = useRef<Run[] | null>(null);

  const dirty = draft !== null && body !== null && draft !== body;

  /**
   * The text Apple last confirmed for the open note: what an edit began from. Sent with each
   * save, so the save can tell an edit made on another device since and keep both.
   */
  const base = useRef<{ guid: string; text: string; runs?: Run[] | null } | null>(null);
  /** One save at a time: a second, started while the first was out, could land last with
   *  older text. Saves chain behind this. */
  const inFlight = useRef<Promise<void>>(Promise.resolve());

  const flush = useCallback(() => {
    const run = inFlight.current.then(async () => {
      const next = pending.current;
      if (!next) return;
      pending.current = null;
      const from = base.current?.guid === next.guid ? base.current.text : null;
      const fromRuns = base.current?.guid === next.guid ? (base.current.runs ?? null) : null;
      // Straight to the library, which keeps it and sends it behind (the outbox): no controls
      // made busy and no list read again for it — every pause in typing did both, and the next
      // save waited behind that read. The outbox's own word, when it has sent it, reads again.
      let saved: string | null = null;
      setError(null);
      // A read ahead begun before this save lands is not what the note will say: let go.
      aheadAt.current += 1;
      try {
        saved = await api.noteSave(next.guid, next.body, from, next.runs, next.runs ? fromRuns : null);
      } catch (e) {
        setError(messageOf(e));
      }
      if (saved === null) {
        // Not sent: the edit goes back in line (unless newer typing already replaced it), to
        // go with the next pause, the next sync or leaving the note — never dropped.
        if (!pending.current) pending.current = next;
        return;
      }
      const kept: string = saved;
      // Read ahead before this save, it is not what the note says now.
      ahead.current.delete(next.guid);
      aheadAt.current += 1;
      // Its row says what the library now says of it — title, preview, time edited — read
      // back rather than worked out here: the library keeps the title Apple holds while the
      // title line is untouched, and moves the time on only when something really changed. A
      // copy of those rules here had drifted from them, retitling notes and moving them.
      const row = await api.noteRow(next.guid).catch(() => null);
      if (row) {
        setNotes((rows) => {
          const at = rows.findIndex((n) => n.guid === row.guid);
          if (at < 0) return rows;
          const was = rows[at];
          if (was.title === row.title && was.snippet === row.snippet && was.modifiedAt === row.modifiedAt) return rows;
          const out = rows.slice();
          out[at] = { ...was, title: row.title, snippet: row.snippet, modifiedAt: row.modifiedAt };
          return out;
        });
      }
      const keptRuns = next.runs ? (kept === next.body ? next.runs : carryRuns(next.body, next.runs, kept)) : null;
      // Left since: the library has it, and the note now open is another's — nothing on screen
      // is this note's to update. Laid over the open note, its text became that note's.
      if (openGuidRef.current !== next.guid) {
        if (base.current?.guid === next.guid) base.current = null;
        return;
      }
      base.current = { guid: next.guid, text: kept, runs: keptRuns };
      setSaved(true);
      // What Apple now holds becomes what this note reads, text and formatting together, so
      // what is on screen and what was saved agree without a refresh.
      adopted.current = `${next.guid}\u0000${kept}\u0000${JSON.stringify(keptRuns ?? (loadedRunsRef.current?.guid === next.guid ? loadedRunsRef.current.runs : null))}`;
      setLoadedBody({ guid: next.guid, text: kept });
      if (keptRuns) setLoadedRuns({ guid: next.guid, runs: keptRuns });
      // Apple holds `kept` — ours, or ours with another device's edit combined. What is on
      // screen takes it in: unchanged if nothing was typed since, combined if something was.
      // The draft is never handed back to the library's copy, which can lag a save, and was
      // how a five-line note once became a single letter.
      if (kept !== next.body) {
        setDrafted((d) => {
          const now = d !== null && d.guid === next.guid ? d.text : next.body;
          const merged = now === next.body ? kept : (mergeEdits(next.body, now, kept) ?? now);
          if (merged !== now) {
            // Another device's words, taken into the editor with the formatting moved along.
            const laid = draftRuns.current ? carryRuns(now, draftRuns.current, merged) : null;
            draftRuns.current = laid;
            editorRef.current?.replace(merged, laid);
            if (merged !== kept) pending.current = { guid: next.guid, body: merged, runs: next.runs ? laid : null };
          }
          return { guid: next.guid, text: merged };
        });
      }
    });
    inFlight.current = run.catch(() => {});
    return run;
  }, []);

  // An edit that could not be sent goes again when a sync has just succeeded — Apple is
  // reachable — rather than waiting for more typing.
  useEffect(() => {
    if (pending.current) void flush();
  }, [synced, flush]);

  const type = useCallback(
    (typed: string, typedRuns: Run[]) => {
      // Nothing may be typed into a note whose text has not arrived.
      if (!open || body === null) return;
      // The first keystroke in a note marks what it read as the base of the edit.
      if (base.current?.guid !== open.guid) base.current = { guid: open.guid, text: body, runs: formatSafe ? runs : null };
      draftRuns.current = typedRuns;
      ahead.current.delete(open.guid);
      setDrafted({ guid: open.guid, text: typed });
      setEdits((n) => n + 1);
      setSaved(false);
      // Compared only when the text is back to what Apple holds — never on an ordinary key,
      // which in a long note would be a second walk of the whole thing per keystroke. The
      // formatting is compared as formatting, not as runs: Apple's come in pieces where the
      // editor's are merged, and that difference alone queued a save of a note only opened.
      const unchanged = typed === body && (!formatSafe || sameFormatting(typed, typedRuns, runs));
      pending.current = unchanged ? null : { guid: open.guid, body: typed, runs: formatSafe ? typedRuns : null };
    },
    [open, body, runs, formatSafe]
  );

  // A pause in typing is the signal. Long enough not to write on every keystroke, short
  // enough that closing the window a second later has already saved.
  useEffect(() => {
    if (!pending.current) return undefined;
    const timer = window.setTimeout(() => void flush(), 900);
    return () => window.clearTimeout(timer);
    // `edits` as well as the text: a change of style alone leaves the text as it was.
  }, [draft, edits, flush]);

  // Leaving the note takes whatever is outstanding with it, before the draft is cleared; the
  // next note's edit begins from what that note reads, never from this one's.
  // Only the note being left loses its base. Compared with the note now open instead, this
  // closure's `openGuid` is the old one: a new note made from a draft (null → its guid) had
  // its base wiped the moment it became real, and the next save merged from a lagging copy —
  // which wrote the whole text again on every save.
  useEffect(() => {
    const leaving = openGuid;
    return () => {
      void flush().then(() => {
        if (leaving && base.current?.guid === leaving) base.current = null;
      });
    };
  }, [openGuid, flush]);
  useEffect(() => {
    if (base.current && base.current.guid !== openGuid) base.current = null;
  }, [openGuid]);

  /**
   * A note nobody has typed into is not a note. Apple discards an empty new one the moment
   * you look away rather than syncing a "New Note" placeholder to every device, and that
   * had been the actual bug: pressing New note wrote junk to iCloud immediately, even if
   * nothing was ever typed.
   *
   * So composing happens locally first. It becomes a real, synced note the moment there is
   * something worth keeping — a pause after typing, or leaving it — and simply evaporates
   * if you never type at all.
   */
  const [composing, setComposingState] = useState<{
    folderGuid: string | null;
    body: string;
  } | null>(null);
  // A note written on the phone, landing in the list on screen, fades in (lib/arrival) — not
  // one written here, which is being composed until it lands.
  const noteIds = useMemo(() => notes.map((n) => n.guid), [notes]);
  useArrivals(rowsRef, noteIds, `${folder}\u0000${searching ? query : ""}\u0000${scoped}\u0000${listOrder.by}`, outbox.waiting.size > 0 || composing !== null);
  const setComposing = useCallback(
    (
      next:
        | { folderGuid: string | null; body: string }
        | null
        | ((
            c: { folderGuid: string | null; body: string } | null
          ) => { folderGuid: string | null; body: string } | null)
    ) => {
      setComposingState((current) => {
        const value = typeof next === "function" ? next(current) : next;
        composingRef.current = value;
        return value;
      });
    },
    []
  );
  // Words typed and not yet saved: a restart to install an update waits for them (lib/updates).
  const unsaved = dirty || Boolean(composing?.body.trim());
  useEffect(() => {
    holdTyping("notes", unsaved);
    return () => holdTyping("notes", false);
  }, [unsaved]);

  /**
   * Turn what is being typed into a real note, once, without taking the page away.
   *
   * It used to swap the box being typed into for a different one: for as long as the create
   * took, nothing was open — "Select a note" — and focus fell out onto the window, so the
   * next keystroke went nowhere and Ctrl+A selected the whole app instead of the note. The
   * box that went away could also fire its blur and create the note a second time, which is
   * where three "Untitled" notes came from. Now the same box stays, a create in flight is not
   * started again, and the note it makes is adopted in place, with anything typed during the
   * create saved onto it afterwards.
   */
  const creating = useRef(false);
  const commitComposing = useCallback(async () => {
    const draft = composingRef.current;
    if (!draft || creating.current) return;
    if (!draft.body.trim()) {
      // Nothing was ever typed. There is nothing to keep, so nothing is sent.
      setComposing(null);
      return;
    }
    creating.current = true;
    setBusy(true);
    setError(null);
    try {
      const guid = await api.noteCreate(draft.folderGuid, draft.body);
      const latest = composingRef.current?.body ?? draft.body;
      adopting.current = guid;
      // The same editor goes on as this note: nothing remounts under the cursor.
      adoptedKey.current = guid;
      setLoadedBody({ guid, text: draft.body });
      base.current = { guid, text: draft.body };
      // The note is made from its text; its formatting follows straight after — the first
      // line a title, as Notes makes it, and whatever was set while composing.
      const typedRuns = draftRuns.current && runsFit(latest, draftRuns.current) ? titled(latest, draftRuns.current) : null;
      setDrafted({ guid, text: latest });
      setEdits((n) => n + 1);
      pending.current = { guid, body: latest, runs: typedRuns };
      // Its formatting known from the start — what was set while composing, or the title line
      // Notes gives a new note — so the editor writes formatting from the first key. Without
      // it a key typed before the first save dropped the title, and every save after was
      // words only, so what was set here never reached Apple.
      const plain: Run = { length: latest.length, style: null, indent: 0, checked: null, bold: false, italic: false, underline: false, strike: false, link: null };
      setLoadedRuns({ guid, runs: typedRuns ?? titled(latest, latest.length ? [plain] : []) });
      // In the list at once, so the note is open — and its editor still there — before the
      // list is read again.
      const now = Math.floor(Date.now() / 1000);
      setNotes((ns) => ns.some((n) => n.guid === guid) ? ns : [{
        guid, folderGuid: draft.folderGuid, title: splitTitle(latest).title || "New Note", snippet: null, body: null,
        locked: false, pinned: false, createdAt: now, modifiedAt: now, editable: true, etag: null,
      } as Note, ...ns]);
      setOpenGuid(guid);
      setComposing(null);
      await read();
    } catch (e) {
      // The write failed, not the typing — it stays on screen to try again.
      setError(messageOf(e));
    } finally {
      creating.current = false;
      setBusy(false);
    }
  }, [read, setComposing]);

  // The same pause-then-save rhythm as an existing note, up until there is a real note to
  // hand it to.
  useEffect(() => {
    if (!composing || !composing.body.trim()) return undefined;
    const timer = window.setTimeout(() => void commitComposing(), 900);
    return () => window.clearTimeout(timer);
  }, [composing, commitComposing]);

  /** Finish composing before doing anything that would otherwise abandon it mid-thought. */
  const selectNote = useCallback(
    async (guid: string) => {
      if (composing) await commitComposing();
      setOpenGuid(guid);
      setDetail(true);
    },
    [composing, commitComposing]
  );

  const create = useCallback(async () => {
    // Nothing is written in Recently Deleted, as in Notes, where New Note is dimmed there: a
    // note begun here was made inside the deleted notes, and went with them.
    if (folder === TRASH_FOLDER) return;
    // What was typed a moment ago is handed over first, to the note it was typed in.
    editorRef.current?.flushChange();
    if (composingRef.current) await commitComposing();
    await flush();
    composeSeq.current += 1;
    adoptedKey.current = null;
    setComposing({ folderGuid: folder === ALL ? null : folder, body: "" });
    setOpenGuid(null);
    setDetail(true);
    // Straight into the new note, as Apple's New Note does.
    window.requestAnimationFrame(() => editorRef.current?.focus());
  }, [commitComposing, flush, folder, setComposing]);

  /**
   * Delete goes to Recently Deleted, as on the phone; from there, Delete Now is final and is
   * asked about first, in Apple's words.
   */
  const inTrash = open?.folderGuid === TRASH_FOLDER;
  /**
   * Folders, as Notes manages them. Each shows its change at once — a new "New Folder" ready
   * to be named, a new name, a folder gone — and a refusal re-reads and says why.
   */
  const newFolder = async (parent: string | null) => {
    const standIn = `pending-folder-${Date.now()}`;
    setFolders((fs) => [...fs, { guid: standIn, title: "New Folder", sortOrder: 0, parentGuid: parent } as NoteFolder]);
    try {
      const guid = await api.noteFolderCreate("New Folder", parent);
      await read();
      setRenamingFolder(guid);
    } catch (e) {
      setFolders((fs) => fs.filter((f) => f.guid !== standIn));
      setError(messageOf(e));
    }
  };
  const renameFolder = async (f: NoteFolder, title: string) => {
    setRenamingFolder(null);
    const name = title.trim();
    if (!name || name === f.title) return;
    setFolders((fs) => fs.map((x) => (x.guid === f.guid ? { ...x, title: name } : x)));
    try {
      await api.noteFolderRename(f.guid, name);
      await read();
    } catch (e) {
      setError(messageOf(e));
      read().catch(() => {});
    }
  };
  const deleteFolder = async (f: NoteFolder) => {
    // Apple's own words for it.
    const sure = await confirm({
      title: `Delete \u201c${f.title}\u201d?`,
      body: "All notes and any subfolders will be deleted.",
      action: "Delete",
      danger: true,
      symbol: "trash",
    });
    if (!sure) return;
    setFolders((fs) => fs.filter((x) => x.guid !== f.guid && x.parentGuid !== f.guid));
    if (folder === f.guid) setFolder(ALL);
    try {
      await api.noteFolderDelete(f.guid);
      await read();
    } catch (e) {
      setError(messageOf(e));
      read().catch(() => {});
    }
  };

  const remove = useCallback(async () => {
    if (!open) return;
    if (
      open.folderGuid === TRASH_FOLDER &&
      !(await confirm({
        title: "Delete this note now?",
        body: "It will be deleted immediately. You can’t undo this action.",
        action: "Delete",
        danger: true,
        symbol: "trash",
      }))
    ) {
      return;
    }
    const gone = open.guid;
    // The note below opens in its place, or the one above at the end, as in Notes: the order
    // is the list's as shown, pinned notes first.
    const order = groupNotes(notes, new Date(), listOrder).flatMap((g) => g.notes.map((n) => n.guid));
    const at = order.indexOf(gone);
    const next = order[at + 1] ?? order[at - 1] ?? null;
    setOpenGuid(next);
    await write(() => api.noteDelete(gone), (rows) => rows.filter((n) => n.guid !== gone));
  }, [open, write, notes, confirm, listOrder]);

  /** Out of Recently Deleted and back into Notes — Apple's "Notes" folder where there is one. */
  const recover = useCallback(async () => {
    if (!open) return;
    const back = folders.some((f) => f.guid === DEFAULT_FOLDER) ? DEFAULT_FOLDER : null;
    const guid = open.guid;
    await write(() => api.noteMove(guid, back), (rows) => rows.filter((n) => n.guid !== guid));
  }, [open, write, folders]);

  const moveTo = useCallback(
    async (guid: string) => {
      if (!open) return;
      const to = guid === ALL ? null : guid;
      const moving = open.guid;
      await write(
        () => api.noteMove(moving, to),
        // Out of the folder on screen; in All it stays, filed somewhere else.
        (rows) => folder === ALL
          ? rows.map((n) => (n.guid === moving ? { ...n, folderGuid: to } : n))
          : rows.filter((n) => n.guid !== moving),
      );
    },
    [open, write, folder]
  );

  const pin = useCallback(async () => {
    if (!open) return;
    const guid = open.guid;
    const pinned = !open.pinned;
    await write(
      () => api.notePin(guid, pinned),
      (rows) => rows.map((n) => (n.guid === guid ? { ...n, pinned } : n)),
    );
  }, [open, write]);

  /**
   * Locked notes opened this session, by note: their text, in this window's memory only —
   * never the library, never disk. Lock Now lets every one go, and the passwords with them.
   */
  const [unlocked, setUnlocked] = useState<Record<string, UnlockedNote>>({});
  const opened = open?.locked ? (unlocked[open.guid] ?? null) : null;
  const lockNow = useCallback(() => {
    setUnlocked({});
    notesLockApi.noteLockForget().catch(() => {});
  }, []);
  /** How the last note was picked: a click goes on into its password field, an arrow key stays in the list. */
  const pickedBy = useRef<"pointer" | "keys">("keys");

  /**
   * Adding a picture or file: the paperclip, or a file dropped on the note. Both exist only
   * while the backend allows it (see notes/attach.ts).
   */
  const attachOn = useAttachEnabled();
  const [attaching, setAttaching] = useState(false);
  /** Files on their way into the note, shown at once with a picture of each where there is
   *  one — beside the text, never in it, so nothing half-added is ever saved. */
  const [adding, setAdding] = useState<{ name: string; src: string | null }[]>([]);
  const [dropping, setDropping] = useState(false);
  const attach = useCallback(
    async (paths: string[]) => {
      if (!open || open.locked || !open.editable || paths.length === 0) return;
      const guid = open.guid;
      // What is typed goes first, so the file lands in the text Apple holds.
      editorRef.current?.flushChange();
      await flush();
      const shown = editorRef.current?.current();
      let text = shown?.text ?? body;
      let at = editorRef.current?.caret() ?? text?.length ?? 0;
      setAttaching(true);
      setAdding(paths.map((p) => ({ name: fileName(p), src: null })));
      // Each file's picture, made on this computer while it uploads (a moment for a photo).
      const previews: string[] = [];
      paths.forEach((p, i) =>
        void notesLockApi
          .noteAttachPreview(p)
          .then((buf) => {
            if (!buf || buf.byteLength === 0) return;
            const src = URL.createObjectURL(new Blob([buf]));
            previews.push(src);
            setAdding((list) => list.map((x, j) => (j === i ? { ...x, src } : x)));
          })
          .catch(() => {}),
      );
      try {
        // Each file on its own: one refused (a folder, a system file, too large) leaves the
        // rest to go in, and is named once at the end.
        let added = 0;
        const refused: string[] = [];
        for (const path of paths) {
          try {
            const done = await notesLockApi.noteAttachFile(guid, path, at, text);
            text = done.body;
            at = done.offset + 1;
            added += 1;
          } catch (e) {
            refused.push(messageOf(e));
          }
        }
        if (added > 0) {
          toast({
            message:
              paths.length === 1
                ? `Added “${fileName(paths[0])}”`
                : added === paths.length
                  ? `Added ${added} files`
                  : `Added ${added} of ${paths.length} files`,
            tone: "done",
          });
        }
        if (refused.length > 0) {
          setError(refused.length === 1 ? refused[0] : `${refused[0]} ${refused.length - 1} more weren’t added either.`);
        }
        // The note as Apple now holds it, file in place — kept here as it answered (any typing
        // still on its way laid over it), so read from the library rather than asked again.
        const [nextBody, nextRuns] = await Promise.all([api.noteBody(guid), api.noteRuns(guid)]);
        if (nextBody !== null) setLoadedBody({ guid, text: nextBody });
        setLoadedRuns({ guid, runs: Array.isArray(nextRuns) ? nextRuns : [] });
        await read();
      } catch (e) {
        setError(messageOf(e));
      } finally {
        setAttaching(false);
        setAdding([]);
        previews.forEach((u) => URL.revokeObjectURL(u));
      }
    },
    [open, flush, body, read]
  );
  const pickAttachment = async () => {
    try {
      const chosen = await pickOpen({ multiple: true, title: "Add to Note" });
      if (!chosen) return;
      await attach(chosen);
    } catch (e) {
      setError(messageOf(e));
    }
  };
  const live = useActiveRef();
  const canAttachRef = useRef(false);
  const attachRef = useRef(attach);
  attachRef.current = attach;
  useEffect(() => {
    if (!attachOn) return undefined;
    let stop: (() => void) | undefined;
    let cancelled = false;
    // Over the note itself, on the screen in front: a file dropped anywhere else is not for it.
    const overNote = (x: number, y: number) => {
      const ratio = window.devicePixelRatio || 1;
      return Boolean((document.elementFromPoint(x / ratio, y / ratio) as HTMLElement | null)?.closest?.(".note-body"));
    };
    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          const p = event.payload;
          if (!live.current || !canAttachRef.current) return setDropping(false);
          if (p.type === "over" || p.type === "enter") setDropping(overNote(p.position.x, p.position.y));
          else if (p.type === "leave") setDropping(false);
          else if (p.type === "drop") {
            setDropping(false);
            if (overNote(p.position.x, p.position.y)) void attachRef.current(p.paths);
          }
        })
        .then((unlisten) => {
          if (cancelled) unlisten();
          else stop = unlisten;
        })
        .catch(() => {});
    } catch {
      // Outside the app shell there is no webview to listen to; the paperclip still works.
    }
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [attachOn, live]);

  // A save confirmation is a moment, not a state.
  useEffect(() => {
    if (!saved) return undefined;
    const timer = window.setTimeout(() => setSaved(false), 2000);
    return () => window.clearTimeout(timer);
  }, [saved]);

  // Editable once the words AND their formatting are in. Opened on the words alone, the editor
  // came up plain; a key typed before the formatting arrived was then saved as the note's
  // formatting, and took its titles, checklists and bold off every device.
  // A note in Recently Deleted is read, not edited, as on the phone: recovering it is how to
  // edit it again. Editable here, what was typed into a deleted note was saved to it.
  const canEdit = Boolean(composing) || Boolean(open && !open.locked && open.editable && !inTrash && body !== null && runs !== null);
  const canAttach = attachOn && !composing && !inTrash && Boolean(open && !open.locked && open.editable && body !== null);
  canAttachRef.current = canAttach;

  /** Delete from the keyboard, asked first: a key is easier to press by mistake than a button. */
  const removeAsked = useCallback(async () => {
    if (!open || open.locked) return;
    // Recently Deleted asks its own, final question.
    if (open.folderGuid === TRASH_FOLDER) return remove();
    const name = wordsOnly(open.title) || "New Note";
    const sure = await confirm({
      title: `Delete \u201c${name}\u201d?`,
      body: "It moves to Recently Deleted, where you can recover it.",
      action: "Delete",
      danger: true,
      symbol: "trash",
    });
    if (sure) await remove();
  }, [open, remove, confirm]);

  /** The row of a note, to put focus on it. */
  const focusRow = (guid: string | null) =>
    window.requestAnimationFrame(() => {
      const row = guid ? rowsRef.current?.querySelector<HTMLElement>(`[data-guid="${CSS.escape(guid)}"]`) : null;
      (row ?? rowsRef.current?.querySelector<HTMLElement>(".note-row"))?.focus();
    });

  /** Up and Down through the list as it is shown, pinned notes first. */
  const step = useCallback(
    (by: 1 | -1) => {
      const order = groupNotes(notes, new Date(), listOrder).flatMap((g) => g.notes.map((n) => n.guid));
      if (order.length === 0) return;
      const at = openGuid ? order.indexOf(openGuid) : -1;
      const next = at < 0 ? order[0] : order[Math.max(0, Math.min(order.length - 1, at + by))];
      pickedBy.current = "keys";
      if (next !== openGuid) void selectNote(next);
      focusRow(next);
    },
    [notes, openGuid, selectNote, listOrder]
  );

  /** Into the search field: this folder (Ctrl+F) or every folder (Ctrl+Shift+F). */
  const find = useCallback((where: "all" | "folder") => {
    setScope(where);
    const field = searchRef.current;
    field?.focus();
    field?.select();
  }, []);

  /** Out of search, back to the list where the open note is. */
  const leaveSearch = useCallback(() => {
    setQuery("");
    setScope("all");
    focusRow(openGuid);
  }, [openGuid]);

  // The keys, as Notes on a Mac has them (Ctrl here). Only while Notes is the screen in front,
  // and never over a sheet or a menu, which answer their own keys.
  const keys = useRef({ create, find, removeAsked, step, leaveSearch, searching, newFolder });
  keys.current = { create, find, removeAsked, step, leaveSearch, searching, newFolder };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!live.current || e.defaultPrevented || e.isComposing) return;
      if (overlayOpen()) return;
      const target = e.target as HTMLElement | null;
      const typing = Boolean(target?.closest?.("input, textarea, select, [contenteditable='true']"));
      const mod = (e.ctrlKey || e.metaKey) && !e.altKey;
      const k = e.key.toLowerCase();
      const k_ = keys.current;
      if (mod && !e.shiftKey && k === "n") {
        e.preventDefault();
        if (!e.repeat) void k_.create();
      } else if (mod && e.shiftKey && k === "n") {
        // New Folder, as Shift-Command-N makes one on a Mac: ready to be named.
        e.preventDefault();
        if (!e.repeat) void k_.newFolder(null);
      } else if (mod && k === "f") {
        e.preventDefault();
        k_.find(e.shiftKey ? "all" : "folder");
      } else if (typing) {
        return;
      } else if (e.key === "Escape" && k_.searching) {
        e.preventDefault();
        k_.leaveSearch();
      } else if (mod && !e.shiftKey && (e.key === "Backspace" || e.key === "Delete")) {
        e.preventDefault();
        if (!e.repeat) void k_.removeAsked();
      } else if (!mod && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
        // Not from a control with arrows of its own — a picker, the format panel.
        if (target && target !== document.body && !target.closest(".note-rows, .note-body, .notes-list-head")) return;
        if (target?.closest(".note-body") && target.closest("button, a, [role='checkbox']")) return;
        e.preventDefault();
        k_.step(e.key === "ArrowDown" ? 1 : -1);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [live]);

  /** Every change in the editor: to the note being composed, or to the open one. */
  const apply = (next: string, nextRuns: Run[]) => {
    const c = composingRef.current;
    if (c) {
      draftRuns.current = nextRuns;
      // Known at once, not when React gets to it: New Note pressed in the same moment hands
      // this text over first (`create`), and must not find the note still empty.
      composingRef.current = { ...c, body: next };
      setComposing((was) => (was ? { ...was, body: next } : was));
    } else type(next, nextRuns);
  };

  // What Apple holds, taken into the editor when nothing is being typed — a change made on
  // the phone arrives in the open note without it being reopened.
  // Keyed to Apple's copy alone: a save, a keystroke or a re-render is never a reason to
  // put the editor back to an older text.
  const adopted = useRef<string | null>(null);
  useEffect(() => {
    if (!open || body === null) return;
    const key = `${open.guid}\u0000${body}\u0000${JSON.stringify(runs)}`;
    if (adopted.current === key) return;
    adopted.current = key;
    // Only over nothing new in this note: no typing at all, or none since what Apple last
    // confirmed. Another note's edit still on its way is no reason to keep this one's editor
    // from what it holds.
    if (pending.current?.guid === open.guid || (draftRef.current !== null && draftRef.current !== base.current?.text)) return;
    editorRef.current?.replace(body, formatSafe ? runs : null);
    draftRuns.current = formatSafe ? runs : null;
    if (draftRef.current !== null && draftRef.current !== body) {
      setDrafted(null);
      base.current = null;
    }
  }, [open, body, runs, formatSafe]);

  /** The note as it is on screen, typing not yet sent included, saved as a Markdown file. */
  const exportNote = async () => {
    if (!open || body === null) return;
    // Read off the editor itself: the very text on screen, with its formatting.
    const shown = editorRef.current?.current();
    const text = shown?.text ?? draft ?? body;
    const set = shown?.runs ?? runs ?? [];
    const name = (text.split("\n")[0].trim() || "Note").replace(/[\\/:*?"<>|]/g, "-").slice(0, 80);
    let path: string | null;
    try {
      path = await pickSave({ defaultName: `${name}.md`, filters: [{ name: "Markdown", extensions: ["md"] }] });
    } catch (e) {
      setError(messageOf(e));
      return;
    }
    if (!path) return;
    try {
      await api.noteExportMarkdown(text, set, path);
    } catch (e) {
      setError(messageOf(e));
    }
  };

  const leave = () => {
    editorRef.current?.flushChange();
    void (composingRef.current ? commitComposing() : flush());
  };

  // Another folder is another list, and a narrow window shows that list, not a note from the last.
  useEffect(() => setDetail(false), [folder]);

  const sections = useMemo(() => groupNotes(notes, new Date(), listOrder), [notes, listOrder]);
  /** What a search is looking for, marked in the results and in the note opened from them. */
  const foundWords = useMemo(() => (searching ? queryWords(query) : []), [searching, query]);
  /** The folders as nested, worked out when they change rather than on every key typed. */
  const nested = useMemo(() => nestFolders(folders), [folders]);

  /** A row clicked: the one function for every row, so the list is not drawn again for it. */
  const pick = (note: Note, e: React.MouseEvent) => {
    pickedBy.current = e.detail > 0 ? "pointer" : "keys";
    // A locked note clicked again, already open: on into its password field.
    if (e.detail > 0 && note.locked && note.guid === openGuid)
      document.querySelector<HTMLInputElement>(".notes .note-lock-field")?.focus();
    void selectNote(note.guid);
  };
  const pickRef = useRef(pick);
  pickRef.current = pick;
  const pickRow = useCallback((note: Note, e: React.MouseEvent) => pickRef.current(note, e), []);
  /** The open note, told to the rows of the list one by one (lib/chosen). */
  const chosenNote = useChosen(openGuid ? [openGuid] : []);

  // The notes either side of the open one, read ahead once it has opened (see `ahead`): behind
  // every save already on its way, and dropped if the library changed meanwhile.
  const bodyShown = body !== null;
  useEffect(() => {
    if (!openGuid || !bodyShown) return undefined;
    const order = sections.flatMap((g) => g.notes);
    const at = order.findIndex((n) => n.guid === openGuid);
    if (at < 0) return undefined;
    const near = [order[at + 1], order[at - 1]].filter(
      (n): n is Note =>
        Boolean(n && !n.locked && n.editable && !ahead.current.has(n.guid) && pending.current?.guid !== n.guid),
    );
    if (!near.length) return undefined;
    const when = aheadAt.current;
    let live = true;
    void inFlight.current.then(() =>
      Promise.all(
        near.map(async (n) => {
          const [text, runs] = await Promise.all([api.noteBody(n.guid), api.noteRuns(n.guid)]);
          if (!live || when !== aheadAt.current || text === null || !Array.isArray(runs)) return;
          if (ahead.current.size > 16) ahead.current.clear();
          ahead.current.set(n.guid, { text, runs });
        }),
      ).catch(() => {}),
    );
    return () => {
      live = false;
    };
  }, [openGuid, bodyShown, sections]);

  // "Opening…" only for a note slow to arrive: one read in a moment draws straight away, and
  // the spinner it used to show first was a flash on every note opened.
  const waiting = Boolean(open && !open.locked && (body === null || (open.editable && runs === null)));
  const [slow, setSlow] = useState(false);
  /** The note whose read just came back with no text in the library. */
  const [noText, setNoText] = useState<string | null>(null);
  useEffect(() => {
    setSlow(false);
    if (!waiting) return undefined;
    const t = window.setTimeout(() => setSlow(true), 300);
    return () => window.clearTimeout(t);
  }, [waiting, openGuid]);
  const heading = searching
    ? "Search"
    : folder === ALL
      ? "All iCloud"
      : (folders.find((f) => f.guid === folder)?.title ?? "Notes");
  const saveFailed = error !== null;

  return (
    <div
      className={`notes${detail ? " is-detail" : ""}`}
      ref={notesRef}
      style={foldersWidth ? ({ "--folders-w": `${foldersWidth}px` } as React.CSSProperties) : undefined}
    >
      <NotePicture />
      <div
        className="notes-folders-edge"
        role="separator"
        aria-orientation="vertical"
        aria-label="Folder column width"
        title="Drag to resize. Double-click to fit the names."
        onPointerDown={(e) => {
          const box = notesRef.current?.getBoundingClientRect();
          if (!box) return;
          // A drag here resizes, and only that: left to the browser it also began a text
          // selection, swept over every label it crossed.
          e.preventDefault();
          e.currentTarget.setPointerCapture(e.pointerId);
          const move = (m: PointerEvent) => setFoldersWidth(clampFolders(m.clientX - box.left));
          const up = () => {
            window.removeEventListener("pointermove", move);
            window.removeEventListener("pointerup", up);
            kept.current = true;
            setFoldersWidth((w) => {
              if (w) keepFoldersWidth(w);
              return w;
            });
          };
          window.addEventListener("pointermove", move);
          window.addEventListener("pointerup", up);
        }}
        onDoubleClick={() => {
          const w = fitFolders(foldersRef.current);
          if (w) {
            kept.current = true;
            setFoldersWidth(w);
            keepFoldersWidth(w);
          }
        }}
      />
      <aside className="notes-folders" aria-label="Note folders" ref={foldersRef}>
        <button className="source-row notes-folder" aria-current={folder === ALL} onClick={() => setFolder(ALL)}>
          <FolderOpen className="notes-folder-glyph" size={16} strokeWidth={2} aria-hidden="true" />
          <span className="source-name notes-folder-name">All iCloud</span>
          <span className="count notes-folder-count">{counts.all || ""}</span>
        </button>
        {nested
          .filter(({ folder: f }) => f.guid !== TRASH_FOLDER)
          .map(({ folder: f, depth }) => {
            const own = f.guid !== DEFAULT_FOLDER;
            if (renamingFolder === f.guid) {
              return (
                <input
                  key={f.guid}
                  className="field notes-folder-rename"
                  defaultValue={f.title}
                  aria-label={`Rename ${f.title}`}
                  autoFocus
                  onFocus={(e) => e.currentTarget.select()}
                  onBlur={(e) => void renameFolder(f, e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") e.currentTarget.blur();
                    if (e.key === "Escape") { e.currentTarget.value = f.title; e.currentTarget.blur(); }
                  }}
                  style={depth > 0 ? { paddingLeft: `${8 + depth * 16}px` } : undefined}
                />
              );
            }
            const openMenu = (x: number, y: number, alignRight: boolean) =>
              setFolderMenu({ folder: f, x, y, alignRight });
            return (
              <div
                key={f.guid}
                className={`notes-folder-row${folderMenu?.folder.guid === f.guid ? " is-menu" : ""}`}
                onContextMenu={(e) => {
                  e.preventDefault();
                  // Apple's own "Notes" folder has no menu: it cannot be renamed or deleted.
                  if (own) openMenu(e.clientX, e.clientY, false);
                }}
              >
                <button
                  className="source-row notes-folder"
                  aria-current={folder === f.guid}
                  onClick={() => setFolder(f.guid)}
                  style={depth > 0 ? { paddingLeft: `${8 + depth * 16}px` } : undefined}
                >
                  <Folder className="notes-folder-glyph" size={16} strokeWidth={2} aria-hidden="true" />
                  <span className="source-name notes-folder-name" title={f.title}>{f.title}</span>
                  <span className="count notes-folder-count">{counts.per.get(f.guid) || ""}</span>
                </button>
                {own && (
                  <button
                    className="icon-button small round row-more notes-folder-more"
                    aria-label={`Actions for ${f.title}`}
                    aria-haspopup="menu"
                    onClick={(e) => {
                      const r = e.currentTarget.getBoundingClientRect();
                      openMenu(r.right, r.bottom + 4, true);
                    }}
                  >
                    <span aria-hidden="true">•••</span>
                  </button>
                )}
              </div>
            );
          })}
        {folders.some((f) => f.guid === TRASH_FOLDER) && (
          <button
            className="source-row notes-folder"
            aria-current={folder === TRASH_FOLDER}
            onClick={() => setFolder(TRASH_FOLDER)}
          >
            <Trash2 className="notes-folder-glyph" size={16} strokeWidth={2} aria-hidden="true" />
            <span className="source-name notes-folder-name">Recently Deleted</span>
            <span className="count notes-folder-count">{counts.per.get(TRASH_FOLDER) || ""}</span>
          </button>
        )}
        <button className="side-add" onClick={() => void newFolder(null)}>
          <FolderPlus size={15} strokeWidth={2} aria-hidden="true" />
          New Folder
        </button>
        {folderMenu && (
          <RowMenu
            at={folderMenu}
            onClose={() => setFolderMenu(null)}
            entries={[
              { label: "Rename Folder", run: () => setRenamingFolder(folderMenu.folder.guid) },
              { label: "New Folder", run: () => void newFolder(folderMenu.folder.guid) },
              null,
              { label: "Delete Folder", danger: true, run: () => void deleteFolder(folderMenu.folder) },
            ]}
          />
        )}
        <AppFoot />
      </aside>

      <section className="notes-list" aria-label="Notes">
        <header className="pane-head notes-list-head">
          <PaneTitle count={`${notes.length} ${notes.length === 1 ? "note" : "notes"}`}>{heading}</PaneTitle>
          <button
            className="icon-button"
            aria-label="View Options"
            title="View Options"
            aria-haspopup="menu"
            aria-expanded={!!viewMenu}
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              setViewMenu({ x: r.right, y: r.bottom + 4, alignRight: true });
            }}
          >
            <ArrowDownUp size={17} strokeWidth={2} aria-hidden="true" />
          </button>
          {viewMenu && (
            <RowMenu
              at={viewMenu}
              onClose={() => setViewMenu(null)}
              entries={[
                { label: "Date Edited", checked: listOrder.by === "edited", run: () => orderBy({ ...listOrder, by: "edited" }) },
                { label: "Date Created", checked: listOrder.by === "created", run: () => orderBy({ ...listOrder, by: "created" }) },
                { label: "Title", checked: listOrder.by === "title", run: () => orderBy({ ...listOrder, by: "title" }) },
                ...(listOrder.by === "title"
                  ? []
                  : [null, { label: "Group By Date", checked: listOrder.groupByDate, run: () => orderBy({ ...listOrder, groupByDate: !listOrder.groupByDate }) }]),
              ]}
            />
          )}
          <button
            className="icon-button"
            aria-label="New note"
            title="New Note"
            onClick={() => void create()}
            disabled={busy || folder === TRASH_FOLDER}
          >
            <SquarePen size={18} strokeWidth={2} aria-hidden="true" />
          </button>
        </header>
        <SearchField
          ref={searchRef}
          className="notes-search"
          placeholder={folder !== ALL && scope === "folder" ? `Search ${folders.find((f) => f.guid === folder)?.title ?? "This Folder"}` : "Search"}
          aria-label="Search notes"
          aria-keyshortcuts="Control+F Control+Shift+F"
          value={query}
          onChange={setQuery}
          onClear={() => {
            setQuery("");
            setScope("all");
            searchRef.current?.focus();
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              leaveSearch();
            } else if (e.key === "ArrowDown" && searching) {
              // On into what was found.
              e.preventDefault();
              focusRow(openGuid && notes.some((n) => n.guid === openGuid) ? openGuid : null);
            }
          }}
        />
        {searching && folder !== ALL && (
          <div className="segmented notes-scope" role="group" aria-label="Search in">
            <button type="button" className="segment" aria-pressed={scope === "all"} onClick={() => setScope("all")}>
              All iCloud
            </button>
            <button type="button" className="segment" aria-pressed={scope === "folder"} onClick={() => setScope("folder")}>
              <span className="notes-scope-name">{folders.find((f) => f.guid === folder)?.title ?? "This Folder"}</span>
            </button>
          </div>
        )}

        {/* Shown only when the folder column has folded away. */}
        <div className="notes-folder-picker">
          <Picker
            label="Folder"
            value={folder}
            onChange={setFolder}
            options={[
              { value: ALL, label: "All iCloud" },
              ...folders.map((f) => ({ value: f.guid, label: f.title })),
            ]}
          />
        </div>

        <Unsent refused={outbox.refused} what="an edit to a note" onResend={(r) => void outbox.resend(r)} onDiscard={(r) => void outbox.discard(r)} />

        {/* One list box of notes, a group per heading: the open note is its selected option.
            The heading is drawn for the eye; the group's name says it to a screen reader. */}
        <NoteRows sections={sections} chosen={chosenNote} rowsRef={rowsRef} onPick={pickRow} found={foundWords} pictures={pictures} />

        {loaded && notes.length === 0 && !error && !(arriving && !searching) && (
          searching ? (
            <EmptyState className="notes-empty">
              Nothing matches “{query.trim()}”{scoped ? " in this folder" : ""}.
            </EmptyState>
          ) : folder === TRASH_FOLDER ? (
            <EmptyState className="notes-empty" icon={<Trash2 size={28} strokeWidth={1.5} />}>
              Recently Deleted is empty.
            </EmptyState>
          ) : (
            <EmptyState className="notes-empty" icon={<SquarePen size={28} strokeWidth={1.5} />}>
              Notes you write here appear on all your devices.
            </EmptyState>
          )
        )}
        {notes.length === 0 && !searching && (
          <Arriving service="notes" what="notes" className="notes-empty" />
        )}
      </section>

      <article className={`note-body${dropping ? " is-dropping" : ""}`} aria-label="Note">
        {(composing || open) && (
          <div className="note-bar">
            <button
              className="compact-back"
              onClick={() => {
                // Leaving the note takes what was typed with it, as leaving any note does.
                void (composingRef.current ? commitComposing() : flush());
                setDetail(false);
              }}
            >
              <ChevronLeft size={18} strokeWidth={2.25} aria-hidden="true" />
              <span>{heading}</span>
            </button>
            <p
              className={`note-when${
                busy || (!saveFailed && (dirty || composing?.body.trim())) || (open && outbox.waiting.has(open.guid)) || saved
                  ? ""
                  : " is-date"
              }`}
            >
              {busy || (!saveFailed && (dirty || composing?.body.trim()))
                ? "Saving…"
                : open && outbox.waiting.has(open.guid)
                  ? <Waiting shown />
                  : saved
                  ? "Saved"
                  : editorDate(composing ? Math.floor(Date.now() / 1000) : (open?.modifiedAt ?? null))}
            </p>
            {!composing && open && !open.locked && (
              <div className="toolbar-group note-bar-actions">
                {!inTrash && open.editable && (
                  <>
                    <button
                      className={`tool notes-aa${formatAt ? " is-on" : ""}`}
                      aria-label="Format"
                      aria-haspopup="dialog"
                      aria-expanded={Boolean(formatAt)}
                      title="Choose a style to apply to text"
                      disabled={!canEdit}
                      // Keeps the cursor where it is in the note while the panel is open.
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={(e) => {
                        const r = e.currentTarget.getBoundingClientRect();
                        setFormatAt((at) => (at ? null : { x: r.left + r.width / 2, y: r.bottom + 6 }));
                      }}
                    >
                      Aa
                    </button>
                    {canAttach && (
                      <button
                        className="tool"
                        aria-label="Add a picture or file"
                        title="Add a picture or file"
                        disabled={attaching || busy}
                        onMouseDown={(e) => e.preventDefault()}
                        onClick={() => void pickAttachment()}
                      >
                        <Paperclip size={17} strokeWidth={2} aria-hidden="true" />
                      </button>
                    )}
                    <button
                      className="tool"
                      aria-label="Make a checklist"
                      aria-pressed={Boolean(status?.checklist)}
                      title="Make a checklist"
                      disabled={!canEdit}
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => editorRef.current?.toggleChecklist()}
                    >
                      <ListChecks size={18} strokeWidth={2} aria-hidden="true" />
                    </button>
                    {formatAt && (
                      <FormatPanel
                        at={formatAt}
                        status={status}
                        onClose={() => setFormatAt(null)}
                        onStyle={(style) => editorRef.current?.setStyle(style)}
                        onMark={(mark) => editorRef.current?.toggleMark(mark)}
                        onQuote={() => editorRef.current?.toggleQuote()}
                        onColour={(c) => editorRef.current?.setColour(c)}
                        onIndent={(by) => editorRef.current?.indent(by)}
                        onMove={(by) => editorRef.current?.moveLines(by)}
                      />
                    )}
                  </>
                )}
                {!inTrash && <Picker
                  pullDown
                  label={`Folder for ${open.title}`}
                  value={open.folderGuid ?? ALL}
                  disabled={busy}
                  onChange={(guid) => void moveTo(guid)}
                  options={[
                    // "No folder" only for an account without Apple's default "Notes" folder;
                    // with one, it was a second "Notes" that meant something else.
                    ...(folders.some((f) => f.guid === DEFAULT_FOLDER)
                      ? []
                      : [{ value: ALL, label: "Notes" }]),
                    ...folders
                      .filter((f) => f.guid !== TRASH_FOLDER)
                      .map((f) => ({ value: f.guid, label: f.title })),
                  ]}
                />}
                {!inTrash && !open.locked && (
                  <button
                    className="tool"
                    aria-label="Export as Markdown"
                    title="Export as Markdown"
                    onClick={() => void exportNote()}
                    disabled={body === null}
                  >
                    <Share size={17} strokeWidth={2} aria-hidden="true" />
                  </button>
                )}
                {!inTrash && (
                  <button
                    className="tool"
                    aria-label={open.pinned ? "Unpin Note" : "Pin Note"}
                    aria-pressed={open.pinned}
                    title={open.pinned ? "Unpin Note" : "Pin Note"}
                    onClick={() => void pin()}
                    disabled={busy}
                  >
                    {open.pinned ? (
                      <PinOff size={17} strokeWidth={2} aria-hidden="true" />
                    ) : (
                      <Pin size={17} strokeWidth={2} aria-hidden="true" />
                    )}
                  </button>
                )}
                {inTrash && (
                  <button className="tool has-label is-accent" onClick={() => void recover()} disabled={busy}>
                    Recover
                  </button>
                )}
                {inTrash ? (
                  <button className="tool has-label danger" onClick={() => void remove()}>
                    Delete Now
                  </button>
                ) : (
                  <button
                    className="tool"
                    aria-label="Delete"
                    title="Delete"
                    onClick={() => void remove()}
                  >
                    <Trash2 size={17} strokeWidth={2} aria-hidden="true" />
                  </button>
                )}
              </div>
            )}
            {!composing && open?.locked && opened && (
              <div className="toolbar-group note-bar-actions">
                <button className="tool has-label note-lock-now" onClick={lockNow} title="Lock every opened note again">
                  <LockKeyholeOpen size={16} strokeWidth={2} aria-hidden="true" />
                  Lock Now
                </button>
              </div>
            )}
          </div>
        )}

        {/*
          * One editor for a new note and a saved one, always in the same place, so the box
          * being typed into is never taken away when a new note becomes a real one. The
          * first line is a title field and the rest a body, but they are one text: Enter in
          * the title splits it, Backspace at the start of the body joins it, and what is
          * saved is always the two put back together.
          */}
        {canEdit ? (
          <>
          <div className="note-editor"
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "s") {
                e.preventDefault();
                leave();
              }
            }}
          >
            <NoteEditor
              // One editor per note, so nothing typed in one can be shown in another.
              key={composing || (open && adoptedKey.current === open.guid) ? `composing-${composeSeq.current}` : open?.guid}
              ref={editorRef}
              text={composing ? composing.body : (draft ?? body ?? "")}
              runs={composing ? draftRuns.current : formatSafe ? runs : null}
              editable={!attaching}
              autoFocus={Boolean(composing)}
              label={composing ? "New note" : `${open?.title ?? "Note"}`}
              placeholder={composing ? "New Note" : undefined}
              onChange={apply}
              onStatus={setStatus}
              onBlur={leave}
              found={composing ? undefined : foundWords}
            />
          </div>
          {attaching && (
            // The note waits while a file goes in, so nothing typed meanwhile is lost when
            // it comes back from Apple with the file in place.
            <div className="note-adding" role="status" aria-live="polite">
              <div className="note-adding-card">
                {adding.map((f, i) => (
                  <div className="note-adding-file" key={`${f.name}-${i}`}>
                    {f.src ? (
                      <img className="note-adding-thumb" src={f.src} alt="" draggable={false} />
                    ) : (
                      <span className="note-adding-thumb is-file" aria-hidden="true">
                        <Paperclip size={18} strokeWidth={2} />
                      </span>
                    )}
                    <span className="note-adding-name">{f.name}</span>
                  </div>
                ))}
                <span className="note-adding-state">
                  <Spinner small /> Adding to the note
                </span>
              </div>
            </div>
          )}
          </>
        ) : open?.locked && opened ? (
          /* Opened with its password: read here, written on an Apple device. */
          <div className="note-editor is-readonly is-unlocked">
            <h1 className="note-title">{splitTitle(opened.body).title}</h1>
            {runsFit(opened.body, opened.runs) ? (
              <div className="note-text note-read">
                <FormattedBody text={opened.body} runs={opened.runs} skip={splitTitle(opened.body).title.length + 1} />
              </div>
            ) : (
              <pre className="note-text">{splitTitle(opened.body).rest}</pre>
            )}
            <p className="note-readonly">Locked notes open read-only here. Edit them on your iPhone, iPad or Mac.</p>
          </div>
        ) : open?.locked ? (
          <LockScreen
            guid={open.guid}
            title={wordsOnly(open.title)}
            autoFocus={pickedBy.current === "pointer"}
            onOpen={(note) => setUnlocked((all) => ({ ...all, [open.guid]: note }))}
          />
        ) : waiting ? (
          /* Nothing to type into until the note's own text has arrived: an empty box over a
             note whose body had not loaded would write that emptiness back. Said only once it
             is slow in coming. */
          open && noText === open.guid ? (
            <EmptyState className="note-loading">
              This note's text isn't here yet. If it doesn't appear after the next sync, Skykeep
              can't read it, but it's safe in iCloud: open it on your iPhone, iPad or Mac.
            </EmptyState>
          ) : (
            slow && <EmptyState busy className="note-loading">Opening…</EmptyState>
          )
        ) : open ? (
          <div className="note-editor is-readonly">
            <h1 className="note-title">{splitTitle(body ?? "").title}</h1>
            {runs && body !== null && runsFit(body, runs) ? (
              <div className="note-text note-read">
                <FormattedBody text={body} runs={runs} skip={splitTitle(body).title.length + 1} />
              </div>
            ) : (
              <pre className="note-text">{splitTitle(body ?? "").rest}</pre>
            )}
            {inTrash ? (
              <p className="note-readonly">Recover this note to edit it.</p>
            ) : (
              <p className="note-readonly">
                Read-only here, to keep everything in it safe. A quick edit on your iPhone, iPad or
                Mac usually makes it editable again.
              </p>
            )}
          </div>
        ) : (
          loaded && <EmptyState className="note-pick">Select a note.</EmptyState>
        )}
      </article>
      {confirmSheet}
    </div>
  );
}
