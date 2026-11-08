import { convertFileSrc, invoke } from "@tauri-apps/api/core";

export type Status = {
  hasStoredSession: boolean;
  signedIn: boolean;
  libraryReady: boolean;
  /** Signed in only while Skykeep runs: no system keyring would keep the session. */
  notKept: boolean;
};

/** A change made here waiting for Apple. */
export type WaitingChange = { service: string; record: string; kind: string };
/** A change Apple turned down for good, kept with what it held. */
export type RefusedChange = {
  id: number;
  service: string;
  record: string;
  kind: string;
  reason: string;
  madeAt: number;
  /** What was typed: the note's text, the reminder's title, the person's name. */
  content: string;
};
export type OutboxStatus = { waiting: WaitingChange[]; refused: RefusedChange[] };
export type SignInResult = "complete" | "needs_two_factor";

/** `mode` is how Apple reaches the number: "sms", or "voice" for one it calls. */
export type TrustedPhone = { id: number; maskedNumber: string; mode?: string };
export type TwoFactorOptions = {
  trustedDeviceCount: number;
  phones: TrustedPhone[];
  tooManyCodesSent: boolean;
  /** The account signs in with a security key, which Skykeep can't use yet. */
  securityKey?: boolean;
};

/** How one service's refresh ended. `paused`: Advanced Data Protection keeps it until the
 * next approval — waiting, not failed. `preparing`: Apple is still preparing it (a Photos
 * library just turned on) — nothing was read, and it is looked at again in a while. */
export type SyncOutcome = "ok" | "unavailable" | "failed" | "paused" | "preparing";

export type ServiceReport = {
  service: string;
  label: string;
  outcome: SyncOutcome;
  /** What came back, or what went wrong. */
  detail: string | null;
  lastSyncedAt: number | null;
};

/** Everything one refresh pass did. */
export type Refresh = {
  services: ServiceReport[];
  /** The stalest part of the library — the only freshness the window can claim. */
  asOf: number | null;
  tookMs: number;
  /**
   * The services whose stored rows this changed, when the backend knows: a screen reads again
   * only for its own. Absent, every screen reads again.
   */
  changed?: string[];
  /**
   * For services in `changed` whose items the backend could place, which ones: a screen that
   * takes them redraws those rows alone. A service in `changed` and not here is read whole.
   */
  items?: Record<string, ItemChanges>;
};

/** What a pass did to one app's items, by their ids. */
export type ItemChanges = { added: string[]; changed: string[]; removed: string[] };

/** Emitted by the backend whenever a refresh finishes, background ones included. */
/** Where each uploading file's progress is announced. */
export const UPLOAD = "quince://upload";
export const PHOTO_UPLOAD = "quince://photo-upload";

/** One file's upload, as it moves. */
export type UploadEvent = {
  id: string;
  batch: string;
  name: string;
  /** The folders it sits in below the one it was dropped on. */
  within: string;
  sent: number;
  total: number;
  state: "preparing" | "waiting" | "sending" | "verifying" | "done" | "failed" | "cancelled" | "planned";
  /** A folder being made, rather than a file being sent. */
  folder?: boolean;
  error: string | null;
};

/** Where each downloading file's progress is announced (a folder, or a selection, into Downloads). */
export const DOWNLOAD = "quince://download";

/**
 * One step of a download: a file as it moves (a small one says only that it landed), a folder
 * that could not be read — or, for the batch, `found` while the tree is counted and `planned`
 * once all of it is known.
 */
export type DownloadEvent = {
  id: string;
  batch: string;
  name: string;
  /** The folders it sits in inside Downloads, as saved. */
  within: string;
  /** Bytes so far; on `found` and `planned`, how many files. */
  sent: number;
  /** Bytes in all; on `found` and `planned`, the bytes of every file. */
  total: number;
  state: "downloading" | "done" | "failed" | "cancelled" | "found" | "planned";
  /** A folder that could not be read or made, rather than a file. */
  folder: boolean;
  error: string | null;
};

/** What a download into Downloads came to. */
export type DownloadReport = {
  /** What it is known by: downloading the same again resumes it. */
  key: string;
  /** One path for each thing chosen that is there, whole or in part. */
  saved: string[];
  /** What did not arrive, one line each. */
  failed: string[];
  /** Failures past those named. */
  moreFailed: number;
  cancelled: boolean;
  /** Why it stopped short on its own: the disk full or about to be, the connection lost. */
  problem: string | null;
  /** It can be gone on with where it was left. */
  resumable: boolean;
};

/** A download that did not finish, from before — cancelled, stopped, or cut off by quitting. */
export type UnfinishedDownload = { key: string; title: string; items: DriveItem[]; saved: string[] };

/** Told whenever Skykeep's own update moves on; each window asks for its status again. */
export const UPDATE = "quince://update";

/** Who updates this copy of Skykeep: itself, or the package manager it came from. */
export type UpdateInstall = "here" | "packageManager" | "flatpak" | "store" | "development" | "manual";
export type UpdateStatus = {
  install: UpdateInstall;
  /** Check for Updates Automatically. */
  auto: boolean;
  current: string;
  phase: "idle" | "checking" | "current" | "available" | "downloading" | "ready" | "waiting" | "installing" | "failed";
  version: string | null;
  notes: string | null;
  received: number;
  total: number | null;
  /** What a downloaded update waits for before Skykeep restarts. */
  waitingFor: "changes" | "typing" | null;
  problem: string | null;
  checkedAt: number | null;
  /** This window is the one that shows the update notice. */
  prompt: boolean;
};

export const SYNCED = "quince://synced";
/** An alert was clicked: open Reminders, where that reminder is. */
export const OPEN_REMINDER = "quince://open-reminder";
/** A lapsed session signed itself back in only as far as Apple's code. */
export const NEEDS_CODE = "quince://needs-code";
/** Apple signed the session out and wants the password again. */
export const NEEDS_SIGNIN = "quince://needs-signin";
/** Where an Advanced Data Protection approval stands changed (`DataAccess`). */
export const DATA_ACCESS = "quince://data-access";

/**
 * Advanced Data Protection: whether this session may read the end-to-end encrypted services
 * yet. `open` — nothing waits on an approval; `waiting` — the approval is on the person's
 * devices, or given and the keys on their way; `needed` — one is needed and none is on its
 * way; `notSent` — Apple couldn't put it on any of their devices; `webAccessOff` — "Access
 * iCloud Data on the Web" is off.
 */
export type DataAccess = {
  state: "open" | "waiting" | "needed" | "notSent" | "webAccessOff";
  /** The services waiting on it: `notes`, `reminders`, `photos`, `drive`. */
  services: string[];
  /** Why the last step failed, in words, when it did. */
  problem: string | null;
};

export const api = {
  /** One refresh for every service. Never rejects for a single service failing. */
  /** `asked`: the person pressed Sync Now, so Advanced Data Protection's approval may be asked for. */
  syncAll: (asked?: boolean) => invoke<Refresh>("sync_all", { asked: asked ?? false }),
  /** What the library already knows about its own freshness, fetching nothing. */
  syncStatus: () => invoke<Refresh>("sync_status"),
  /** A refresh of only the services named — one app's "update now". */
  syncSome: (services: string[], asked?: boolean) =>
    invoke<Refresh>("sync_some", { services, asked: asked ?? false }),

  status: () => invoke<Status>("auth_status"),
  /** Where a returning person goes: their library, the code Apple wants, or sign-in. */
  resume: () => invoke<"library" | "code" | "signin">("auth_resume"),
  signIn: (appleId: string, password: string) =>
    invoke<SignInResult>("auth_sign_in", { appleId, password }),
  submitCode: (code: string) => invoke<void>("auth_submit_code", { code }),
  twoFactorOptions: () => invoke<TwoFactorOptions>("auth_two_factor_options"),
  resendCode: () => invoke<void>("auth_resend_code"),
  /** The code screen is open: Apple sends the code, unless this sign-in already asked. */
  requestCode: () => invoke<void>("auth_request_code"),
  sendSms: (phoneId: number) => invoke<void>("auth_send_sms", { phoneId }),
  submitSmsCode: (phoneId: number, code: string) =>
    invoke<void>("auth_submit_sms_code", { phoneId, code }),
  signOut: (keep: boolean) => invoke<void>("auth_sign_out", { keep }),
  /** What of the account has not reached iCloud yet: asked before a sign-out. */
  unsent: () => invoke<{ changes: number; files: number; photos: number }>("account_unsent"),
  /** What waits to reach Apple, and what Apple turned down. */
  outboxStatus: () => invoke<OutboxStatus>("outbox_status"),
  /** What the network lock lets Skykeep reach: Apple's domains, and each subscribed calendar's host. */
  networkReach: () => invoke<{ apple: string[]; subscribed: string[]; updates: string[] }>("network_reach"),
  /** Skykeep's own updates, as this window shows them (updates.rs). */
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  /** Check for Updates, from Settings: a failure is said. */
  updateCheck: () => invoke<UpdateStatus>("update_check"),
  /** Update and Restart: download, wait until nothing would be lost, install, restart. */
  updateApply: () => invoke<void>("update_apply"),
  /** Later: the notice goes from every window; a download is kept. */
  updateLater: () => invoke<void>("update_later"),
  updateAutoSet: (on: boolean) => invoke<void>("update_auto_set", { on }),
  /** This window is being typed in, or no longer is: a restart to update waits for it. */
  updateTyping: (typing: boolean) => invoke<void>("update_typing", { typing }),
  /** A refused change, sent again. */
  outboxResend: (id: number) => invoke<void>("outbox_resend", { id }),
  /** A refused change let go of: Apple's version stands. */
  outboxDiscard: (id: number) => invoke<void>("outbox_discard", { id }),
  services: () => invoke<string[]>("account_services"),

  eventSave: (draft: EventWrite) => invoke<string>("event_save", { draft }),
  eventAlerts: (guid: string) => invoke<number[]>("event_alerts", { guid }),
  /** An event's guests and their answers, the organizer first. Read only. */
  eventGuests: (guid: string) => invoke<Guest[]>("event_guests", { guid }),
  eventRepeat: (guid: string) => invoke<EventRepeat | null>("event_repeat", { guid }),
  eventDelete: (guid: string, scope?: EventScope) => invoke<void>("event_delete", { guid, scope: scope ?? null }),
  calendarSync: (from: string, to: string, timezone: string) =>
    invoke<SyncSummary>("calendar_sync", { from, to, timezone }),
  /** Events holding every word of the query, soonest first. */
  calendarSearch: (query: string) => invoke<CalendarEvent[]>("calendar_search", { query }),
  calendarEvents: (from: number, to: number) =>
    invoke<CalendarEvent[]>("calendar_events", { from, to }),
  calendarList: () => invoke<Calendar[]>("calendar_list"),
  photosList: (view: PhotoView) => invoke<Photo[]>("photos_list", { view }),
  /** Every photo a grid shows, as it lays them out: ids, grouping dates and marks, no photos. */
  photosIndex: (source: PhotoSource, order: PhotoOrder | null) => invoke<PhotoIndex>("photos_index", { source, order }),
  /** A source's photos from `offset`, in its index's order: the first screen, before the index. */
  /** `limit` photos of a source: its first, or those just after `after` — the last photo
   * shown, by its index key and id. */
  photosPage: (source: PhotoSource, order: PhotoOrder | null, after: { key: number; id: string } | null, limit: number) =>
    invoke<Photo[]>("photos_page", { source, order, after, limit }),
  /** Photos by id, in the order asked; any gone are left out. */
  photosGet: (ids: string[]) => invoke<Photo[]>("photos_get", { ids }),
  photosCounts: () => invoke<[PhotoView, number][]>("photos_counts"),
  /** The library's months, newest first, each with its count and key photos (Years and Months). */
  photosMonths: () => invoke<PhotoMonth[]>("photos_months"),
  /** The albums the person made, each with how many photos it holds. */
  /** Each album, how many it shows, and its cover — the first photo it shows. */
  photosAlbums: () => invoke<[PhotoAlbum, number, string | null][]>("photos_albums"),
  photosInAlbum: (album: string) => invoke<Photo[]>("photos_in_album", { album }),
  /** A video (or a Live Photo's moving part) opened in this computer's own player. */
  /** How a video's picture is turned, as its file records it (served upright: drawn here). */
  photoVideoTurn: (id: string, which: "original" | "video") => invoke<Turn>("photo_video_turn", { id, which }),
  /** How much larger than its box a film is drawn, so only its clean aperture shows (a Live
   *  Photo's moving part is recorded with a margin the still does not have). */
  photoVideoZoom: (id: string, which: "original" | "video") => invoke<number>("photo_video_zoom", { id, which }),
  /** The iCloud Drive folder on this computer: whether it is up, where, and what stopped it. */
  driveFolderStatus: () => invoke<DriveFolder>("drive_folder_status"),
  driveFolderSet: (on: boolean) => invoke<DriveFolder>("drive_folder_set", { on }),
  driveFolderOpen: () => invoke<void>("drive_folder_open"),
  /** The iCloud Photos folder on this computer: the same, for photos (read-only for now). */
  photosFolderStatus: () => invoke<DriveFolder>("photos_folder_status"),
  /** What the iCloud Drive folder keeps on this computer; null while the folder is off. */
  driveKept: () => invoke<string[] | null>("drive_kept"),
  /** One search across every app's library: the best few of each and how many in all, or
   * (`only`) the next `each` of one app from its `skip`th. Null when a newer `seq` overtook it. */
  searchEverything: (query: string, o: { seq: number; each?: number; only?: string; skip?: number }) =>
    invoke<SearchFound | null>("search_everything", { query, ...o }),
  driveKeep: (drivewsid: string, on: boolean) => invoke<string[]>("drive_keep", { drivewsid, on }),
  photosFolderSet: (on: boolean) => invoke<DriveFolder>("photos_folder_set", { on }),
  /** Open at Login: whether it is on, the command it runs, and whether this desktop runs it. */
  loginItem: () => invoke<LoginItem>("login_item_status"),
  loginItemSet: (on: boolean) => invoke<LoginItem>("login_item_set", { on }),
  photosFolderOpen: () => invoke<void>("photos_folder_open"),
  photoPlay: (id: string) => invoke<void>("photo_play", { id }),
  /** Where the window's player fetches videos from (a loopback server), or null without one. */
  videoBase: () => invoke<string | null>("video_base"),
  /** What this computer's player can decode; `missing` names the packages it lacks. */
  videoSupport: () => invoke<{ h264: boolean; hevc: boolean; missing: string[] }>("video_support"),
  /** The window full screen or back; answers whether it now is. */
  windowFullscreen: (on: boolean) => invoke<boolean>("window_fullscreen", { on }),
  /** What the window shows pictures at, so the app makes them before they are shown. */
  photosPrepare: (shown: { cell: number; fit: boolean; stageW: number; stageH: number; dpr: number; view?: PhotoView | null }) =>
    invoke<void>("photos_prepare", { shown }),
  /** Which photo library the views show (the library switcher), and whether a Shared Library
   *  exists to choose. */
  photosLibrary: () => invoke<{ showing: PhotoLibrary; hasShared: boolean }>("photos_library"),
  /** The library the views show from now on. */
  photosSetLibrary: (which: PhotoLibrary) => invoke<void>("photos_set_library", { which }),
  /** Shared albums, made or joined, newest first — from Apple each time. */
  photosSharedAlbums: () => invoke<SharedAlbum[]>("photos_shared_albums"),
  /** The Hidden album unlocked with the Apple ID password; refused with why (wrong, or a wait). */
  photosUnlockHidden: (password: string) => invoke<void>("photos_unlock_hidden", { password }),
  photosLockHidden: () => invoke<void>("photos_lock_hidden"),
  /** Whether Windows Hello can unlock Hidden here (Windows, with Hello set up). */
  photosHelloAvailable: () => invoke<boolean>("photos_hello_available"),
  photosUnlockHiddenHello: () => invoke<"verified" | "declined" | "unavailable">("photos_unlock_hidden_hello"),
  /** Every photo of one burst, in the order they were shot. */
  photosBurst: (burst: string) => invoke<Photo[]>("photos_burst", { burst }),
  /** Favorite, Delete (to Recently Deleted) or Recover; answers the photo as Apple now has it. */
  photosSetFlag: (ids: string[], flag: "favorite" | "deleted" | "hidden" | "expunged", on: boolean) =>
    invoke<Photo[]>("photos_set_flag", { ids, flag, on }),
  photoSetFlag: (id: string, flag: "favorite" | "deleted" | "hidden" | "expunged", on: boolean) =>
    invoke<Photo>("photo_set_flag", { id, flag, on }),
  /** The original saved to Downloads; answers where. */
  /** As icloud.com's Download Options: the original (the default), the highest quality, or
   * the most compatible. Answers where it was saved. */
  photoDownload: (id: string, want?: "originals" | "highestQuality" | "mostCompatible") =>
    invoke<string>("photo_download", { id, want: want ?? null }),
  /** A photo on the clipboard as a picture. On Linux and Windows it is put there by the app
   * and nothing comes back; elsewhere the PNG comes back for the window to put there. */
  photoCopy: (id: string) => invoke<ArrayBuffer>("photo_copy", { id }),
  /** Title and caption together, as Info saves them. */
  photoSetText: (id: string, title: string, caption: string) => invoke<Photo>("photo_set_text", { id, title, caption }),
  photoSetRating: (id: string, rating: number) => invoke<Photo>("photo_set_rating", { id, rating }),
  photoSetKeywords: (id: string, keywords: string[]) => invoke<Photo>("photo_set_keywords", { id, keywords }),
  photoOpenMap: (id: string) => invoke<void>("photo_open_map", { id }),
  /** When it was taken, in unix seconds. */
  photoSetDate: (id: string, takenAt: number) => invoke<Photo>("photo_set_date", { id, takenAt }),
  /** Every word in a name, title, caption, keyword, year or month; newest first. */
  photosSearch: (query: string) => invoke<Photo[]>("photos_search", { query }),
  /** Photos and videos (or folders of them) into the library. Answers the batch; progress
   * arrives as PHOTO_UPLOAD events, in the same shape as Drive's. */
  /** Into the library, and into `album` too when one is open. */
  photosUpload: (paths: string[], album?: string) => invoke<string>("photos_upload", { paths, album: album ?? null }),
  /** An album — or a folder — at the top or in the folder `parent`. */
  photosAlbumCreate: (name: string, parent?: string, folder?: boolean) =>
    invoke<PhotoAlbum>("photos_album_create", { name, parent: parent ?? null, folder: folder ?? false }),
  photosAlbumRename: (id: string, name: string) => invoke<void>("photos_album_rename", { id, name }),
  photosAlbumDelete: (id: string) => invoke<void>("photos_album_delete", { id }),
  photosAlbumAdd: (album: string, ids: string[]) => invoke<void>("photos_album_add", { album, ids }),
  photosAlbumRemove: (album: string, ids: string[]) => invoke<void>("photos_album_remove", { album, ids }),
  photosUploadCancel: (ids: string[], batch?: string) =>
    invoke<void>("photos_upload_cancel", { ids, batch: batch ?? null }),
  /** New Calendar: Apple's next unused colour; a name already in use is refused. */
  calendarCreate: (title: string) => invoke<string>("calendar_create", { title }),
  calendarUpdate: (guid: string, change: { title?: string; colour?: string }) =>
    invoke<void>("calendar_update", { guid, title: change.title ?? null, colour: change.colour ?? null }),
  calendarDelete: (guid: string) => invoke<void>("calendar_delete", { guid }),

  reminderCounts: (until: number) => invoke<ReminderCounts>("reminder_counts", { until }),
  reminderLists: () => invoke<ReminderList[]>("reminder_lists"),
  reminderGroups: () => invoke<ReminderGroup[]>("reminder_groups"),
  /** The headings inside one list, in the order they were made. */
  reminderSections: (listGuid: string) =>
    invoke<ReminderSection[]>("reminder_sections", { listGuid }),
  remindersIn: (listGuid: string, includeDone: boolean) =>
    invoke<Reminder[]>("reminders_in", { listGuid, includeDone }),
  remindersDueBetween: (from: number, to: number) =>
    invoke<Reminder[]>("reminders_due_between", { from, to }),
  /** A repeat, or null to stop it; `endAt` (seconds) ends it on that date, null never;
   *  `daysOfWeek` (1 Sunday … 7 Saturday) the weekdays a weekly one falls on, empty for its
   *  due date's day. */
  reminderSetRepeat: (guid: string, listGuid: string, repeat: Repeat | null, endAt: number | null = null, daysOfWeek: number[] = []) =>
    invoke<void>("reminder_set_repeat", { guid, listGuid, repeat, endAt, daysOfWeek }),
  reminderSetDone: (guid: string, listGuid: string, done: boolean) =>
    invoke<void>("reminder_set_done", { guid, listGuid, done }),
  reminderEdit: (guid: string, listGuid: string, edit: ReminderEdit) =>
    invoke<void>("reminder_edit", { guid, listGuid, edit }),
  reminderListCreate: (title: string) => invoke<void>("reminder_list_create", { title }),
  reminderListRename: (listGuid: string, title: string) =>
    invoke<void>("reminder_list_rename", { listGuid, title }),
  reminderReorder: (listGuid: string, guid: string, before: string | null) =>
    invoke<void>("reminder_reorder", { listGuid, guid, before }),
  reminderListColour: (listGuid: string, colour: string) =>
    invoke<void>("reminder_list_colour", { listGuid, colour }),
  reminderListDelete: (listGuid: string) =>
    invoke<void>("reminder_list_delete", { listGuid }),
  /** Reminders still to do holding every word typed, across every list. */
  remindersSearch: (query: string) => invoke<Reminder[]>("reminders_search", { query }),
  remindersSmart: (kind: SmartList, until: number) =>
    invoke<Reminder[]>("reminders_smart", { kind, until }),
  reminderCreate: (
    listGuid: string,
    title: string,
    implied: { dueAt?: number; allDay?: boolean; flagged?: boolean } = {},
  ) => invoke<string>("reminder_create", { listGuid, title, ...implied }),
  reminderDelete: (guid: string, listGuid: string) =>
    invoke<void>("reminder_delete", { guid, listGuid }),

  noteFolders: () => invoke<NoteFolder[]>("note_folders"),
  noteParagraph: (guid: string, start: number, change: { style?: number; checked?: boolean; expect?: string }) =>
    invoke<void>("note_paragraph", { guid, start, ...change }),
  /** A table's cells, row by row, by the attachment id a note's text names it by. */
  noteTable: (id: string) => invoke<string[][] | null>("note_table", { id }),
  /** Change one cell of a table; answers the table as Apple now holds it. */
  /** What a note's attachment is — a picture, a document, a recording, a file. */
  noteAttachmentInfo: (id: string) => invoke<NoteAttachment>("note_attachment_info", { id }),
  /** Open a note's attachment in the app this computer uses for it. */
  noteAttachmentOpen: (id: string) => invoke<void>("note_attachment_open", { id }),
  /** Each note's first picture this computer already holds, by note: the attachment whose
   *  small picture the list shows beside it (`noteMediaSrc(id, "thumb")`). Nothing fetched. */
  notePictures: () => invoke<Record<string, string>>("note_pictures"),
  noteTableSetCell: (id: string, row: number, column: number, text: string) =>
    invoke<string[][]>("note_table_set_cell", { id, row, column, text }),
  /** An empty row added at `at` (0 the top); answers the table as Apple now holds it. */
  noteTableAddRow: (id: string, at: number) => invoke<string[][]>("note_table_add_row", { id, at, column: false }),
  /** An empty column added at `at` (0 the leading edge). */
  noteTableAddColumn: (id: string, at: number) => invoke<string[][]>("note_table_add_row", { id, at, column: true }),
  /** The row (or column) at `at` deleted; answers the table as Apple now holds it. */
  noteTableRemove: (id: string, at: number, column: boolean) => invoke<string[][]>("note_table_remove", { id, at, column }),
  noteRuns: (guid: string) => invoke<import("../screens/notes/Formatted").Run[]>("note_runs", { guid }),
  noteFolderCreate: (title: string, parent: string | null) =>
    invoke<string>("note_folder_create", { title, parent }),
  noteFolderRename: (guid: string, title: string) => invoke<void>("note_folder_rename", { guid, title }),
  noteFolderDelete: (guid: string) => invoke<void>("note_folder_delete", { guid }),
  notesIn: (folderGuid: string | null) => invoke<Note[]>("notes_in", { folderGuid }),
  /** [notes outside Recently Deleted, [folder guid, count]…]. */
  noteCounts: () => invoke<[number, [string, number][]]>("note_counts"),
  /** Notes holding every word typed, from every folder but Recently Deleted. */
  notesSearch: (query: string) => invoke<Note[]>("notes_search", { query }),
  /**
   * One note's text, whole. Null when there is no such note any more.
   *
   * A listed note carries no body — the list only draws a title and a line of preview, and
   * carrying every word of every note into the window on each refresh is what makes a folder
   * of thousands slow.
   */
  noteBody: (guid: string) => invoke<string | null>("note_body", { guid }),
  /** One note's row as the library lists it now — title, preview, time edited — with no body.
   *  Null when there is no such note any more. */
  noteRow: (guid: string) => invoke<Note | null>("note_row", { guid }),
  noteExportMarkdown: (body: string, runs: unknown[], path: string) =>
    invoke<void>("note_export_markdown", { body, runs, path }),
  /**
   * Kept on this computer at once and sent behind (the outbox), merged then with whatever
   * Apple holds against `base`. Answers with the text as kept.
   */
  noteSave: (
    guid: string,
    body: string,
    base: string | null,
    runs: import("../screens/notes/Formatted").Run[] | null = null,
    baseRuns: import("../screens/notes/Formatted").Run[] | null = null,
  ) => invoke<string>("note_save", { guid, body, base, runs, baseRuns }),
  noteCreate: (folderGuid: string | null, body: string) =>
    invoke<string>("note_create", { folderGuid, body }),
  noteDelete: (guid: string) => invoke<void>("note_delete", { guid }),
  noteMove: (guid: string, folderGuid: string | null) =>
    invoke<void>("note_move", { guid, folderGuid }),

  /** Pinned notes lead their list, here and on every device. */
  notePin: (guid: string, pinned: boolean) => invoke<void>("note_pin", { guid, pinned }),

  /** Create when the guid is empty, otherwise save changes. Returns Apple's id for it. */
  /** `base` is the card as the form opened it: only what changed from it is written. */
  contactSave: (contact: Contact, base: Contact | null = null) => invoke<string>("contact_save", { contact, base }),
  contactDelete: (guid: string) => invoke<void>("contact_delete", { guid }),
  contactsList: (search: string | null) => invoke<Contact[]>("contacts_list", { search }),
  /** Cards that look like one person. */
  contactsDuplicates: () => invoke<Duplicates[]>("contacts_duplicates"),
  /** These cards as one, shown at once and sent shortly after. Answers the kept card's id. */
  contactsMerge: (guids: string[]) => invoke<string>("contacts_merge", { guids }),
  /** Take back a merge iCloud does not have yet. Answers whether it was. */
  contactsMergeUndo: (kept: string) => invoke<boolean>("contacts_merge_undo", { kept }),
  /** These people as a vCard file in Downloads, written by Apple. Returns the path. */
  contactsExport: (guids: string[], fileName: string) => invoke<string>("contacts_export", { guids, fileName }),
  /** Everyone in a vCard file, added to Contacts. Returns [added, could not be read]. */
  /** vCard and CSV files, any number: [imported, could not be read]. */
  /** How many came in, could not be read, and were in Contacts already. */
  contactsImport: (paths: string[]) => invoke<[number, number, number]>("contacts_import", { paths }),
  /** The card that is this person's own, if they have said. */
  contactMe: () => invoke<string | null>("contact_me"),
  contactSetMe: (guid: string) => invoke<void>("contact_set_me", { guid }),
  /** Contacts' lists — Apple's groups — in the order Apple keeps them. */
  contactLists: () => invoke<ContactList[]>("contact_lists"),
  /**
   * Make a list (no guid), rename one, add people to it or take them out, or delete it.
   * Membership goes as changes, laid over the list as Apple holds it. Returns its id.
   */
  contactListWrite: (change: { guid?: string; name?: string; add?: string[]; remove?: string[]; delete?: boolean }) =>
    invoke<string>("contact_list_write", change),

  driveSyncFolder: (folderId: string) => invoke<number>("drive_sync_folder", { folderId }),
  driveChildren: (folderId: string) => invoke<DriveItem[]>("drive_children", { folderId }),
  driveItem: (drivewsid: string) => invoke<DriveItem | null>("drive_item", { drivewsid }),
  /** Saves to Downloads; returns the path it landed at. */
  driveDownload: (drivewsid: string, item?: DriveItem) => invoke<string>("drive_download", { drivewsid, item: item ?? null }),
  /**
   * Folders, or a selection of files and folders, into Downloads as the same tree. Progress
   * arrives as DOWNLOAD events under `batch`.
   */
  driveDownloadItems: (batch: string, title: string, items: DriveItem[]) =>
    invoke<DownloadReport>("drive_download_items", { batch, title, items }),
  /** Downloads that did not finish, oldest first: downloading one again resumes it. */
  driveDownloadsUnfinished: () => invoke<UnfinishedDownload[]>("drive_downloads_unfinished"),
  /** No longer offer to resume this download; what it saved stays. */
  driveDownloadForget: (key: string) => invoke<void>("drive_download_forget", { key }),
  /** Stop a download — a whole batch, or the batches these files are in. What arrived stays. */
  driveDownloadCancel: (ids: string[], batch?: string) =>
    invoke<void>("drive_download_cancel", { ids, batch: batch ?? null }),
  /** The files opened most recently, newest first. */
  driveRecents: () => invoke<RecentItem[]>("drive_recents"),
  /**
   * Put files from this machine into a Drive folder. Paths only — the bytes are read and
   * streamed in the backend, never carried through a message.
   *
   * Resolves with the files that did not go up, one line each, so a single refusal does not
   * read as the whole upload failing.
   */
  /** Files and folders into a Drive folder. Progress arrives as UPLOAD events. */
  /** Stop uploads — these, or a whole batch. One already being filed finishes. */
  driveUploadCancel: (ids: string[], batch?: string) => invoke<void>("drive_upload_cancel", { ids, batch: batch ?? null }),
  driveUpload: (batch: string, folderId: string, paths: string[]) =>
    invoke<string[]>("drive_upload", { batch, folderId, paths }),
  driveCreateFolder: (parentId: string, name: string) =>
    invoke<void>("drive_create_folder", { parentId, name }),
  driveRename: (drivewsid: string, name: string) =>
    invoke<void>("drive_rename", { drivewsid, name }),
  /** Deleted only as shown: a folder that changed since comes back as it now stands, to be
   * asked about, and goes only as the version `expect` names. */
  driveDelete: (drivewsid: string, expect: string | null = null) =>
    invoke<DriveRemoved>("drive_delete", { drivewsid, expect }),
  driveTrash: () => invoke<TrashItem[]>("drive_trash"),
  driveThumbnails: (folderId: string) => invoke<Record<string, string>>("drive_thumbnails", { folderId }),
  driveDuplicate: (drivewsid: string) => invoke<void>("drive_duplicate", { drivewsid }),
  storageInfo: () => invoke<{ usedBytes: number; totalBytes: number }>("storage_info"),
  /** Who is signed in, as Apple names them. */
  accountInfo: () => invoke<AccountInfo>("account_info"),
  /** Ask Apple for the account again, after something was done at icloud.com. */
  accountRefresh: () => invoke<AccountInfo>("account_refresh"),
  /** Where an Advanced Data Protection approval stands. */
  dataAccess: () => invoke<DataAccess>("data_access_status"),
  /** Put the approval on the person's devices again, and follow it. */
  dataAccessAsk: () => invoke<DataAccess>("data_access_ask"),
  /** Settings' switches, as the backend obeys them. */
  /** Write a plain diagnostic report to Downloads; answers where. Nothing is sent anywhere. */
  diagnosticsSave: () => invoke<string>("diagnostics_save"),
  /** Show a file saved to Downloads in the file manager. */
  downloadReveal: (path: string) => invoke<void>("download_reveal", { path }),
  settingsGet: () => invoke<{ reminderAlerts: boolean; eventAlerts: boolean }>("settings_get"),
  settingsSet: (key: "reminderAlerts" | "eventAlerts", on: boolean) => invoke<void>("settings_set", { key, on }),
  drivePutBack: (item: DriveItem) => invoke<void>("drive_put_back", { item }),
  driveDeleteForever: (item: DriveItem) => invoke<void>("drive_delete_forever", { item }),
  /** Recover, or delete for good, all of these in one request; what could not be, with why. */
  driveTrashOut: (items: DriveItem[], recover: boolean) =>
    invoke<{ drivewsid: string; reason: string }[]>("drive_trash_out", { items, recover }),
  driveMove: (drivewsid: string, destinationId: string) =>
    invoke<void>("drive_move", { drivewsid, destinationId }),
  /** A copy of a file or folder put into a folder, named as Finder names a copy. */
  driveCopy: (drivewsid: string, destinationId: string) =>
    invoke<void>("drive_copy", { drivewsid, destinationId }),
  /** The desktop's clipboard: files copied in the file manager, as paths, or else its text. */
  /** What is on the clipboard, to offer Paste; grants nothing. */
  clipboardRead: () => invoke<{ files: string[]; text: string | null }>("clipboard_read"),
  /** Drive's Paste: what is on the clipboard, its files granted for uploading. */
  clipboardPaste: () => invoke<{ files: string[]; text: string | null }>("clipboard_paste"),
};

/** Tauri rejects with a plain string from our commands; keep it readable either way. */
export type Calendar = {
  guid: string;
  title: string;
  colour: string | null;
  ctag: string | null;
  sortOrder: number;
  enabled: boolean;
  /** False for a subscribed feed or a calendar shared for reading only. */
  writable: boolean;
  /** The one Apple puts a new event in. */
  isDefault: boolean;
};

/**
 * Where a new event should go.
 *
 * Apple names the default calendar outright, so it is asked rather than guessed. Taking the
 * first of the list is the mistake that put every new reminder in whichever list happened to
 * sort first — and the first calendar can also be one nothing may be written to.
 */
export function defaultCalendar(calendars: Calendar[]): Calendar | null {
  const writable = calendars.filter((c) => c.writable);
  return writable.find((c) => c.isDefault) ?? writable[0] ?? null;
}

/** What the app sends to create or change an event. A guid means it already exists. */
export type EventWrite = {
  guid: string | null;
  calendarGuid: string;
  title: string;
  location: string | null;
  startsAt: number;
  endsAt: number;
  allDay: boolean;
  timezone: string | null;
  notes: string | null;
  url: string | null;
  /** Set the alerts to exactly these minutes before the start (0 at the time); left out,
   *  the event keeps the alerts Apple holds. */
  alerts?: number[];
  /** Set the repeat to this (null: never) when `repeatChanged`; otherwise it is left as it is. */
  repeat?: EventRepeat | null;
  repeatChanged?: boolean;
  /** For one occurrence of a repeating event: whether the change is for it alone or the series. */
  scope?: EventScope;
  /** The event as the form opened it, its etag with it: what this save was made from. */
  base?: CalendarEvent;
};

/** How an event repeats, in icloud.com's words. */
/** Someone invited to an event. `status` is "pending" until they answer. */
export type Guest = {
  name: string | null;
  email: string | null;
  status: "accepted" | "declined" | "tentative" | "pending";
  isMe: boolean;
  isOrganizer: boolean;
};

export type EventRepeat = { freq: "daily" | "weekly" | "monthly" | "yearly"; interval: number; count: number | null };

export type CalendarEvent = {
  guid: string;
  calendarGuid: string;
  title: string;
  location: string | null;
  /** Unix seconds, UTC. */
  startsAt: number;
  endsAt: number;
  allDay: boolean;
  timezone: string | null;
  url: string | null;
  notes: string | null;
  etag: string | null;
};

export type SyncSummary = { calendars: number; events: number };

export type ReminderList = {
  guid: string;
  title: string;
  colour: string | null;
  ctag: string | null;
  sortOrder: number;
  enabled: boolean;
  /** Apple's name for the list's glyph, such as "fitness". */
  emblem?: string | null;
  /** The group this list is filed under. */
  groupGuid?: string | null;
  /** Shown as a tile beside Today and Scheduled. */
  pinned?: boolean;
  /** How the list orders its reminders: "manual", "displayDate_asc", … */
  sorting?: string | null;
};

/** A folder of lists in the Reminders sidebar. */
export type ReminderGroup = { guid: string; title: string; sortOrder: number };

/** A heading inside a reminders list. */
export type ReminderSection = {
  guid: string;
  listGuid: string;
  title: string;
  createdAt: number | null;
};

export type SmartList = "today" | "scheduled" | "flagged" | "all" | "completed";

/** What every entry in the Reminders sidebar is holding. */
export type ReminderCounts = {
  today: number;
  scheduled: number;
  flagged: number;
  all: number;
  /** Every reminder ticked off. */
  completed: number;
  /** [list guid, outstanding count] */
  lists: [string, number][];
};

/**
 * The most reminders any one view is handed at once.
 *
 * Kept in step with the same figure in the library. "All" on an account ten years deep is
 * genuinely unbounded, and the whole of it crossing into this window is what the plan rules
 * out; the true totals are counted separately, so a view that reaches this can say so.
 */
export const MOST_AT_ONCE = 1000;

/** How often a reminder comes back. Frequency: 0 daily, 1 weekly, 2 monthly, 3 yearly. */
export type Repeat = { frequency: number; interval: number };

/** Anything omitted is left untouched; `null` clears a field. */
export type ReminderEdit = {
  title?: string;
  notes?: string | null;
  dueAt?: number | null;
  /** With `dueAt`: a day, not a time. */
  allDay?: boolean;
  flagged?: boolean;
  completed?: boolean;
  /** Apple's scale: 0 none, 1 high, 5 medium, 9 low. */
  priority?: number;
  /** Move the reminder to another list. */
  listGuid?: string;
  /** Nest under another reminder, or null to bring it back out to the top level. */
  parentGuid?: string | null;
};

export type Reminder = {
  guid: string;
  listGuid: string;
  title: string;
  notes: string | null;
  dueAt: number | null;
  /** null while outstanding */
  completedAt: number | null;
  priority: number;
  flagged: boolean;
  /** The reminder this one is a subtask of, if any. */
  parentGuid: string | null;
  /** Apple's repeat frequency: 0 daily, 1 weekly, 2 monthly, 3 yearly. null if it never repeats. */
  recurFrequency: number | null;
  recurInterval: number | null;
  /** The section of its list this sits in, if any. */
  sectionGuid?: string | null;
  /** Due on a day rather than at a time. */
  allDay?: boolean;
  /** The named zone its time is kept in ("America/New_York"), as the phone's Time Zone row. */
  timezone?: string | null;
  etag: string | null;
};

export type NoteFolder = {
  guid: string;
  title: string;
  sortOrder: number;
  /** The folder this one sits inside, where it sits inside one at all. */
  parentGuid: string | null;
};

/**
 * Folders in the order they are filed, each with how deep it sits.
 *
 * Apple Notes nests folders, and flattening them turns somebody's filing into one long list
 * with every level jumbled together. A folder whose parent has been deleted is shown at the
 * top rather than dropped — it still holds notes.
 */
export function nestFolders(folders: NoteFolder[]): { folder: NoteFolder; depth: number }[] {
  const byParent = new Map<string | null, NoteFolder[]>();
  const known = new Set(folders.map((f) => f.guid));
  for (const f of folders) {
    const under = f.parentGuid && known.has(f.parentGuid) ? f.parentGuid : null;
    byParent.set(under, [...(byParent.get(under) ?? []), f]);
  }

  const out: { folder: NoteFolder; depth: number }[] = [];
  const seen = new Set<string>();
  const walk = (parent: string | null, depth: number) => {
    for (const folder of byParent.get(parent) ?? []) {
      // A folder cannot be its own ancestor, but nothing about the data guarantees that, and
      // a cycle here would hang the window rather than show a wrong tree.
      if (seen.has(folder.guid)) continue;
      seen.add(folder.guid);
      out.push({ folder, depth });
      walk(folder.guid, depth + 1);
    }
  };
  walk(null, 0);
  for (const folder of folders) {
    if (!seen.has(folder.guid)) out.push({ folder, depth: 0 });
  }
  return out;
}

export type Note = {
  guid: string;
  folderGuid: string | null;
  title: string;
  snippet: string | null;
  body: string | null;
  /** Locked by its owner, so Apple holds it encrypted and there is nothing to show. */
  locked: boolean;
  /** False when Apple's document can't be rewritten without losing what surrounds the text. */
  editable: boolean;
  pinned: boolean;
  createdAt: number | null;
  modifiedAt: number | null;
  etag: string | null;
};

export type ContactMethod = { kind: string; label: string | null; value: string };

export type ContactList = {
  guid: string;
  name: string;
  /** The contacts in it, by guid. */
  members: string[];
  etag: string | null;
};

/** Which events a change to one day of a repeating event is for. */
export type RecentItem = DriveItem & { openedAt: number | null };

export type EventScope = "this" | "future" | "all";

export type Contact = {
  guid: string;
  firstName: string | null;
  middleName: string | null;
  lastName: string | null;
  photoUrl: string | null;
  photoSignature: string | null;
  /** The square chosen, `x,y,width,height` in the picture's own pixels, from its bottom left. */
  photoCrop?: string | null;
  company: string | null;
  isCompany: boolean;
  birthday: string | null;
  notes: string | null;
  etag: string | null;
  /**
   * Every number, email, link, address (its lines), date (`date`, as Apple writes a birthday),
   * related person (`related`), social profile (`profile`) and instant message (`im`, labelled
   * by its service). Labels are Apple's own (`MOBILE` or `_$!<Mobile>!$_`) or as typed.
   */
  methods: ContactMethod[];
  /** The rest of the name and work, as Apple's card holds them. */
  prefix?: string | null;
  suffix?: string | null;
  nickname?: string | null;
  phoneticFirstName?: string | null;
  phoneticLastName?: string | null;
  phoneticCompany?: string | null;
  jobTitle?: string | null;
  department?: string | null;
};

/** Cards that look like one person, and what they share: `name`, `phone`, `email`. */
export type Duplicates = { guids: string[]; reasons: string[] };

/** What to show, whichever name fields a contact happens to have. */
/**
 * Where the interface loads a contact's picture from.
 *
 * Its own scheme rather than a data URI over IPC: the webview fetches and caches it the way
 * it does any image, so a list of a hundred people does not push a hundred pictures through
 * a message channel every time it scrolls.
 */
/** Who is signed in. `photoContact` is their own card, when it has a picture. */
export type AccountInfo = {
  name: string | null;
  email: string | null;
  webAccessAllowed?: boolean | null;
  dataProtection?: boolean | null;
  /** Apple has new iCloud terms to agree to at icloud.com; until then nothing can be read. */
  termsUpdateNeeded?: boolean;
  /** Apple wants the account finished at icloud.com first. */
  repairNeeded?: boolean;
  photoContact?: string | null;
};

export type NoteAttachment = {
  id: string;
  uti: string;
  kind: "image" | "drawing" | "table" | "document" | "audio" | "video" | "inline" | "file";
  filename: string | null;
  title: string | null;
  width: number | null;
  height: number | null;
  size: number | null;
  hasPreview: boolean;
};

/** A note attachment's picture — the file itself, or Apple's rendering of it. */
export function noteMediaSrc(id: string, part: "file" | "inline" | "preview" | "drawing" | "drawing-dark" | "thumb"): string {
  return convertFileSrc(`note/${id}/${part}`, "quince");
}

/** A picture attached to a reminder (`Attachment/<UUID>`), fetched from Apple once and kept. */
export function reminderImageSrc(attachment: string): string {
  return convertFileSrc(`rimg/${attachment.replace(/^Attachment\//, "")}`, "quince");
}

/** One of a photo's pictures, served from the disk cache (fetched from Apple once). */
export function pictureSrc(id: string, size: "thumb" | "medium" | "full" | "original" | "video"): string {
  return convertFileSrc(`pic/${id}/${size}`, "quince");
}

/** The longer sides a picture is made at for the screen (photos.rs FITS). */
export const FITS = [128, 256, 384, 512, 768, 1024, 1536, 2048, 2560, 3072, 3840];

/**
 * A photo's picture made `long` device pixels on its longer side — the step at or above it —
 * resampled with care by the app. The engine then draws it at its own size: shrunk by
 * WebKitGTK instead, a large picture came out jagged.
 */
export function fitSrc(photo: string | Pick<Photo, "id" | "master">, long: number): string {
  const step = FITS.find((f) => f >= long) ?? FITS[FITS.length - 1];
  if (typeof photo === "string") return convertFileSrc(`pic/${photo}/fit${step}`, "quince");
  return convertFileSrc(`pic/${photo.id}~${pictureVersion(photo.master)}/fit${step}`, "quince");
}

/**
 * What a photo's pictures are made from, as a short mark in their address: its original (the
 * master — every picture here is made from it, see photos.rs). Two addresses alike are the same
 * pixels, so the engine keeps the picture for good (the scheme says so for a marked address)
 * and never asks for it again; a photo whose original changed has a new address and is fetched.
 */
export function pictureVersion(master: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < master.length; i++) h = Math.imul(h ^ master.charCodeAt(i), 0x01000193) >>> 0;
  return h.toString(36);
}

/** A shared album, as the last sync found it (iOS 27's shared collections). */
export type SharedAlbum = {
  /** Its zone in Apple's Photos: what it is known by. */
  zone: string;
  /** `private`: made by this account; `shared`: one it was invited to. */
  database: "private" | "shared";
  owner: string | null;
  title: string | null;
  /** When Apple ends it, unix seconds. */
  expiresAt: number | null;
  count: number;
  /** Its newest photo, for its picture. */
  cover: string | null;
};

export type PhotoAlbum = {
  id: string;
  name: string;
  position: number;
  changeTag: string | null;
  /** A folder of albums, not an album of photos. */
  folder?: boolean;
  /** A shared album, by its zone: read from it, and looked at only (nothing in it is edited). */
  shared?: string;
  /** The folder it sits in; none at the top. */
  parent?: string | null;
};

/** Where a grid's photos come from (quince-core photos::Source). */
/** Photos' library switcher: both libraries, the personal one, or the Shared Library. */
export type PhotoLibrary = "both" | "personal" | "shared";

export type PhotoSource = { view: PhotoView } | { album: string } | { query: string } | { burst: string } | { shared: string };
/** How a sortable view is sorted; null keeps the source's own order. */
export type PhotoOrder = { by: "taken" | "added"; oldest: boolean };
/** A grid's layout before its photos (quince-core photos::PhotoIndex). */
export type PhotoIndex = {
  ids: string[];
  /** What each is grouped by: when taken, or when added if sorted by that. */
  keys: number[];
  /** Packed: bits 0–1 the kind (0 image, 1 video, 2 Live), bit 2 a favorite, bits 3–5 its stars. */
  marks: number[];
  first: number | null;
  last: number | null;
};
/** One month of the library as Years and Months show it (quince-core photos::PhotoMonth). */
export type PhotoMonth = {
  year: number;
  /** 1–12. */
  month: number;
  count: number;
  videos: number;
  /** The photo that stands for it: a favourite if it has one, else the still nearest its middle. */
  key: string;
  keyFavorite: boolean;
  /** Up to three more, spread through it, newest first. */
  more: string[];
};
export type PhotoView = "library" | "favorites" | "videos" | "screenshots" | "panoramas" | "portrait" | "recents" | "hidden" | "rated" | "recentlyDeleted";

export type Photo = {
  id: string;
  master: string;
  /** Unix seconds. */
  takenAt: number;
  addedAt: number | null;
  kind: "image" | "video" | "live";
  width: number;
  height: number;
  duration: number | null;
  filename: string | null;
  size: number | null;
  uti: string | null;
  favorite: boolean;
  hidden: boolean;
  deleted: boolean;
  deletedAt: number | null;
  /** In Recently Deleted: when Apple deletes it for good, unix seconds. */
  expungeAt?: number | null;
  /** The burst it was shot in; a burst shows as its chosen photo. */
  burst?: string | null;
  burstPick?: boolean;
  /** How many photos its burst holds. */
  burstCount?: number;
  /** Its title and caption, as typed in Photos' Info. */
  title?: string | null;
  caption?: string | null;
  /** Its star rating, 0 (none) to 5. */
  rating?: number;
  keywords?: string[];
  /** Where it was taken, in degrees. */
  latitude?: number | null;
  longitude?: number | null;
  subtype: number;
  changeTag: string | null;
};

/** The picture of the contact with this id, served by the backend. */
export function contactPhotoSrc(guid: string): string {
  return convertFileSrc(`photo/${guid}`, "quince");
}

/**
 * A contact's picture, named by which picture and which square of it, so a picture changed
 * here (or on the phone) is fetched anew rather than drawn from the webview's cache.
 */
export function photoSrc(c: Contact): string | null {
  if (!c.photoUrl) return null;
  const which = c.photoSignature ?? (c.photoUrl.startsWith("data:") ? textHash(c.photoUrl) : c.photoUrl);
  return `${contactPhotoSrc(c.guid)}?v=${encodeURIComponent(`${which}|${c.photoCrop ?? ""}`)}`;
}

/** A short, stable name for a long text. */
function textHash(text: string): string {
  let h = 5381;
  for (let i = 0; i < text.length; i += 7) h = ((h * 33) ^ text.charCodeAt(i)) >>> 0;
  return `${text.length.toString(36)}${h.toString(36)}`;
}

/** One thing the search across everything found (search.rs `Hit`): marks are [start, end)
 * in UTF-16 units, where the typed words fall. */
export type SearchHit = {
  id: string;
  within: string | null;
  title: string;
  titleMarks: [number, number][];
  detail: string | null;
  detailMarks: [number, number][];
  when: number | null;
  allDay: boolean;
  picture: boolean;
  locked: boolean;
};
export type SearchGroup = { app: string; count: number; hits: SearchHit[] };
export type SearchFound = { seq: number; groups: SearchGroup[] };

export function displayName(c: Contact): string {
  // A company's card goes by the company, as Contacts shows it, whoever is named on it.
  if (c.isCompany && c.company?.trim()) return c.company.trim();
  const full = [c.firstName, c.middleName, c.lastName].filter(Boolean).join(" ").trim();
  if (full) return full;
  if (c.company?.trim()) return c.company;
  return c.methods[0]?.value ?? "No Name";
}

/** A folder as Apple holds it now against how it was shown (`drive::FolderNow`). */
export type FolderNow = { items: number; added: number; changed: number; gone: number; etag: string | null };
/** What a delete from the window did: done, or not — the folder changed since it was shown. */
export type DriveRemoved = { kind: "done" } | ({ kind: "changed" } & FolderNow);

export type DriveItem = {
  drivewsid: string;
  parentId: string | null;
  name: string;
  extension: string | null;
  /** FOLDER, APP_LIBRARY or FILE. */
  kind: string;
  size: number | null;
  childCount: number | null;
  createdAt: number | null;
  modifiedAt: number | null;
  etag: string | null;
};

/** An item in Drive's Recently Deleted: when it goes for good, and where Recover puts it. */
export type TrashItem = DriveItem & { expiresAt: number | null; restorePath: string | null };

export const DRIVE_ROOT = "FOLDER::com.apple.CloudDocs::root";

export const isContainer = (item: DriveItem) =>
  item.kind === "FOLDER" || item.kind === "APP_LIBRARY";

/** The container an item belongs to: the middle of `<TYPE>::<zone>::<id>`. */
export const zoneOf = (drivewsid: string): string | null =>
  drivewsid.split("::")[1] || null;

/**
 * Whether Drive's own items can be copied into, or out of, a folder.
 *
 * An app library (Keynote's, Shortcuts', Obsidian's vault) is a zone of its own. Files from
 * this machine go up into one, filed in its zone; Drive copies its items only beside the
 * original, so a copy across zones is not offered.
 */
export const takesDriveCopies = (folderId: string): boolean =>
  (zoneOf(folderId) ?? "com.apple.CloudDocs") === "com.apple.CloudDocs";

export const driveName = (item: DriveItem) =>
  item.extension && !isContainer(item) ? `${item.name}.${item.extension}` : item.name;

export function messageOf(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong. Try again.";
}

/** A film's turn as its file records it: the matrix `[a, b, c, d]`, each -1, 0 or 1. */
export type Turn = [number, number, number, number];

/**
 * How a film is drawn, turned as it was recorded, in a box the shape of the photo (`w` by
 * `h`, upright). The film is served as the camera wrote it (the backend leaves its turn to
 * the window, which WebKitGTK honoured in one way of drawing and not another); here it is
 * laid out in its own shape and turned into the box with the file's own matrix — CSS's
 * matrix() and the file's are the same reading — a phone's quarter turn, or the front
 * camera's quarter turn and mirror. `w` and `h` are the photo as shown — upright — never the
 * sensor's sides Apple records: given those, a portrait Live Photo's film was laid out in a
 * landscape box, turned, and came out half the size of its still. `zoom` draws it that much
 * larger, for a film recorded with a margin round what the still shows (photoVideoZoom).
 */
export function turnStyle(turn: Turn | null | undefined, w: number, h: number, zoom = 1): Record<string, string> {
  const z = zoom > 1 && zoom <= 1.5 ? zoom : 1;
  if (!turn || (turn[0] === 1 && turn[1] === 0 && turn[2] === 0 && turn[3] === 1) || !w || !h) return z === 1 ? {} : { transform: `scale(${z})` };
  const quarter = turn[0] === 0;
  return {
    position: "absolute",
    left: "50%",
    top: "50%",
    width: quarter ? `${(100 * h) / w}%` : "100%",
    height: quarter ? `${(100 * w) / h}%` : "100%",
    maxWidth: "none",
    maxHeight: "none",
    translate: "-50% -50%",
    transform: `matrix(${turn[0] * z}, ${turn[1] * z}, ${turn[2] * z}, ${turn[3] * z}, 0, 0)`,
  };
}

/** The iCloud Drive folder on this computer (see src-tauri/src/mount.rs). */
export type LoginItem = {
  on: boolean;
  command: string;
  honoured: boolean;
  /** Whether Skykeep keeps the login entry itself (not in the Flatpak). */
  available?: boolean;
  /** Why the switch can't change it: Windows holds the Store install's startup task off in
   * Task Manager, or on or off by policy. */
  elsewhere?: string | null;
};

export type DriveFolder = {
  on: boolean;
  path: string;
  problem: string | null;
  /** Files written into the folder and not yet at Apple (the Drive folder only). */
  waiting?: { files: number; sending: number; failure: string | null } | null;
  /** False where this system cannot have the folder yet (Windows): the row is not shown. */
  available: boolean;
};

/** How much room the local copy takes on this computer, in bytes on disk (src-tauri/src/settings.rs). */
export type LocalStorage = {
  library: number;
  photos: number;
  notes: number;
  drive: number;
  other: number;
  total: number;
  /** What Clear Cached Pictures gives back. */
  clearable: number;
};

export const settingsApi = {
  localStorage: () => invoke<LocalStorage>("local_storage"),
  /** Removes only pictures Apple still has; answers the room used after. */
  localStorageClear: () => invoke<LocalStorage>("local_storage_clear"),
  /** False when this desktop has no notification service; null where unknown or always shown. */
  alertsShown: () => invoke<boolean | null>("alerts_shown"),
};

/** Photos' originals and previews kept on this computer, held to a cap (src-tauri/src/photo_cache.rs). */
export type PhotoCache = {
  /** Originals and previews on disk, in bytes. */
  used: number;
  /** The cap chosen. */
  cap: number;
  /** What it is held to now: the cap, or less while the disk is short of space. */
  limit: number;
  /** The grid's thumbnails, which always stay. */
  thumbnails: number;
  /** The caps that can be chosen, smallest first. */
  steps: number[];
};
/** Told when the cache changed in the background (a trim after the cap was lowered). */
export const PHOTO_CACHE = "quince://photo-cache";
export const photoCacheApi = {
  status: () => invoke<PhotoCache>("photos_cache_status"),
  /** Answers at once; the trim a lower cap starts reports through PHOTO_CACHE. */
  set: (bytes: number) => invoke<PhotoCache>("photos_cache_set", { bytes }),
  clear: () => invoke<PhotoCache>("photos_cache_clear"),
};

/* ── Intake: files opened with Skykeep, and quick capture (src-tauri/src/intake.rs) ── */

/** Told to a window when a file has arrived for it; it collects with `intakeApi.pending`. */
export const INTAKE = "quince://intake";
/** Told to the quick-capture window when it is asked for again, with the kind now wanted. */
export const CAPTURE = "quince://capture";
/** Told to Calendar's window to start a new event (`skykeep --new event`, the jump list;
 *  Library.tsx). A window made for it is asked in its address instead: `?new=event`. */
export const NEW_EVENT = "quince://new-event";

/** An event read from a calendar file: what `eventSave` takes, less the calendar. */
export type ImportEvent = {
  title: string;
  location: string | null;
  startsAt: number;
  endsAt: number;
  allDay: boolean;
  timezone: string | null;
  notes: string | null;
  url: string | null;
  alerts: number[] | null;
  repeat: EventRepeat | null;
  /** The same title at the same time is already in a calendar here: left out. */
  duplicate: boolean;
};

export type ImportContact = { name: string; detail: string | null; known: boolean };

/** What a file opened with Skykeep holds, read on this computer before anything is sent. */
export type Arrival =
  | { kind: "events"; file: string; events: ImportEvent[]; refused: { title: string; reason: string }[] }
  | { kind: "contacts"; file: string; path: string; contacts: ImportContact[] }
  | { kind: "unreadable"; file: string; reason: string };

export const intakeApi = {
  /** What has arrived for this window, taken from the queue. */
  pending: () => invoke<Arrival[]>("intake_pending"),
  /** The quick-capture window, closed. */
  captureClose: () => invoke<void>("intake_capture_close"),
  /** The main window, for signing in. */
  openMain: () => invoke<void>("intake_open_main"),
};

/**
 * A postal address part by part, as Apple keeps it (`contact_addresses`). The names are
 * Apple's own, which is also what an edit names a part by.
 */
export type PostalAddress = {
  label: string | null;
  /** Every line before the town, line breaks and all. */
  street: string | null;
  subLocality: string | null;
  city: string | null;
  subAdministrativeArea: string | null;
  state: string | null;
  postalCode: string | null;
  country: string | null;
  /** Apple's two-letter code, lower case on the cards it writes. */
  countryCode: string | null;
};

/** One part of an address, by Apple's name for it. */
export type AddressPart = Exclude<keyof PostalAddress, "label">;

/**
 * One change to a card's addresses. `index` is a place in the list as it stands after the
 * edits before it in the same batch, starting from the list the form opened.
 */
export type AddressEdit =
  | { op: "setField"; index: number; field: AddressPart; value: string | null }
  | { op: "setLabel"; index: number; label: string | null }
  | { op: "add"; address: PostalAddress }
  | { op: "remove"; index: number };

export const contactsApi = {
  /** These cards alone, whole — those still here. */
  get: (guids: string[]) => invoke<Contact[]>("contacts_get", { guids }),
  /** One card's addresses, part by part. */
  addresses: (guid: string) => invoke<PostalAddress[]>("contact_addresses", { guid }),
  /** Edits laid over the card's addresses at Apple; answers the list as written. */
  addressEdit: (guid: string, base: PostalAddress[] | null, edits: AddressEdit[]) =>
    invoke<PostalAddress[]>("contact_address_edit", { guid, base, edits }),
};

/** An early alert: `count` units before a reminder is due. Unit 0 minutes, 1 hours, 2 days,
 * 3 weeks, 4 months — Apple's numbering. */
export type EarlyAlert = { id: string; count: number; unit: number };

/** What sits beside one reminder: its tags (no `#`), link and early alerts. */
export type ReminderDetail = {
  guid: string;
  tags: string[];
  url: string | null;
  earlyAlerts: EarlyAlert[];
  /** File or image attachments, pictures included. */
  otherAttachments: number;
  /** Its pictures' attachment ids, as the phone lists them. */
  images: string[];
};

/** A repeat whole, as the phone's Repeat and End Repeat rows show it. Frequency is Apple's:
 *  0 daily, 1 weekly, 2 monthly, 3 yearly, 4 hourly. `other` names rule fields (chosen days)
 *  this app keeps but does not read. */
export type RepeatRule = { frequency: number; interval: number; endAt: number | null; occurrenceCount: number; daysOfWeek: number[]; other: string[] };
/** What a reminder's row shows beside its date, as the phone's list does. */
export type ReminderMarks = {
  guid: string;
  daysOfWeek: number[];
  urgent: boolean;
  location: PlaceAlarm | null;
  messaging: string | null;
  url: string | null;
};
/** A place a reminder rings at; `proximity` is "arriving" or "leaving". */
export type PlaceAlarm = { title: string | null; proximity: string | null; kind: string };
/** The person a reminder comes up for when messaging them. */
export type MessagingPerson = { name: string | null; handles: string[] };
/** The rest of the phone's Details screen. `urgent` is null until known. */
export type ReminderMore = {
  guid: string;
  repeat: RepeatRule | null;
  timeZone: string | null;
  urgent: boolean | null;
  location: PlaceAlarm | null;
  messaging: MessagingPerson | null;
};

/** A tag across the account, and how many reminders still to do carry it. */
export type TagCount = { name: string; open: number };

/** A smart list on the account: built in (`today`, `flagged`…) or one made by hand (`custom`). */
export type ReminderSmartList = {
  guid: string;
  kind: string;
  title: string | null;
  colour: string | null;
  emblem: string | null;
  pinned: boolean;
  sorting: string | null;
  filter: string | null;
};

/** What a custom smart list holds now, and any part of its rules not read here. */
export type SmartListContents = { reminders: Reminder[]; unknown: string[] };

/** Each reminder still to do with its tags, and how many each custom smart list holds, by id. */
export type ReminderExtras = { tagsByReminder: Record<string, string[]>; customCounts: Record<string, number> };

/** Told as Urgent is read from Apple for rows whose marks came back without it. */
export const REMINDER_URGENT = "quince://reminder-urgent";
export type UrgentRead = { guid: string; urgent: boolean };

export const remindersApi = {
  detail: (guid: string) => invoke<ReminderDetail>("reminder_detail", { guid }),
  more: (guid: string) => invoke<ReminderMore>("reminder_more", { guid }),
  /** Row marks for these reminders; only rows with any come back. */
  marks: (guids: string[]) => invoke<ReminderMarks[]>("reminder_marks", { guids }),
  /** Reminders whose tick now is the last of their repeat: ticked, each is completed. */
  ending: () => invoke<string[]>("reminders_ending"),
  tags: () => invoke<TagCount[]>("reminder_tags"),
  tagged: (name: string) => invoke<Reminder[]>("reminders_tagged", { name }),
  smartLists: () => invoke<ReminderSmartList[]>("reminder_smart_lists"),
  /** Tags by reminder and smart-list counts, in one read. */
  extras: () => invoke<ReminderExtras>("reminder_extras"),
  inSmartList: (guid: string) => invoke<SmartListContents>("reminders_in_smart_list", { guid }),
  tagAdd: (guid: string, name: string) => invoke<void>("reminder_tag_add", { guid, name }),
  tagRemove: (guid: string, name: string) => invoke<void>("reminder_tag_remove", { guid, name }),
  /** A link, or null to clear it. */
  urlSet: (guid: string, url: string | null) => invoke<void>("reminder_url_set", { guid, url }),
  /** Remind `count` units early, or null to clear. */
  earlyAlertSet: (guid: string, count: number | null, unit: number | null) =>
    invoke<void>("reminder_early_alert_set", { guid, count, unit }),
  sectionCreate: (listGuid: string, title: string) =>
    invoke<ReminderSection>("reminder_section_create", { listGuid, title }),
  sectionRename: (guid: string, title: string) =>
    invoke<void>("reminder_section_rename", { guid, title }),
  /** Into a section of its list, or out of every one with null. */
  sectionMove: (listGuid: string, guid: string, section: string | null) =>
    invoke<void>("reminder_section_move", { listGuid, guid, section }),
};

/** A locked note, opened: its text and how it is set. Held in the window's memory only. */
export type UnlockedNote = { body: string; runs: import("../screens/notes/Formatted").Run[] };

/** A file added to a note: where it landed and the note's text with it in place. */
export type NoteAttached = { attachment: string; media: string; uti: string; offset: number; body: string };

/** Locked notes and attachments (src-tauri/src/notes_cmds.rs). */
export const notesLockApi = {
  /**
   * Open a locked note. The password crosses once and is never kept by the window; with none,
   * the one that opened it earlier this session is used.
   */
  noteUnlock: (guid: string, password: string | null) =>
    invoke<UnlockedNote>("note_unlock", { guid, password }),
  /** The hint left with the note's password, if any. */
  noteLockHint: (guid: string) => invoke<string | null>("note_lock_hint", { guid }),
  /** Lock every opened note again. */
  noteLockForget: () => invoke<void>("note_lock_forget"),
  /** Add the file at `path` at `atOffset` (UTF-16 units of `base`, the text on screen). */
  /** A small picture of a file about to be added (empty for one that is not a picture). */
  noteAttachPreview: (path: string) => invoke<ArrayBuffer>("note_attach_preview", { path }),
  noteAttachFile: (guid: string, path: string, atOffset: number, base: string | null) =>
    invoke<NoteAttached>("note_attach_file", { guid, path, atOffset, base }),
  /**
   * Whether adding files to notes is switched on in the backend
   * (`quince_icloud::notes::attach::NOTE_ATTACH_WRITES`). A backend without the command
   * answers no.
   */
  noteAttachEnabled: () => invoke<boolean>("note_attach_enabled").then((on) => on === true, () => false),
};

/* ── Export Everything: the whole local copy, in open formats (src-tauri/src/export.rs) ── */

/** Told to the window as an export moves along. */
export const EXPORT_PROGRESS = "quince://export";
/** What a cancelled export answers, to tell it from a failure. */
export const EXPORT_CANCELLED = "Export cancelled.";

export type ExportService = "notes" | "reminders" | "contacts" | "calendar" | "drive" | "photos";
/** Which service (`step` of `steps`, from 1) and how far through it. */
export type ExportProgress = { service: ExportService; step: number; steps: number; done: number; total: number };
export type ExportSummary = {
  path: string;
  notes: number;
  reminders: number;
  contacts: number;
  events: number;
  driveFiles: number;
  photos: number;
  photosMissing: number;
};

/** Backup: every photo and video copied as the original into a chosen folder, kept up to date. */
export type BackupStatus = {
  folder: string | null;
  hidden: boolean;
  copied: number;
  total: number;
  running: boolean;
  problem: string | null;
  /** Hidden is locked: including it asks for the unlock first. */
  hiddenLocked: boolean;
};

export const backupApi = {
  status: () => invoke<BackupStatus>("backup_status"),
  /** On into `folder` (from the folder dialog), or off with `null`; the Hidden album or not. */
  set: (folder: string | null, hidden: boolean) => invoke<BackupStatus>("backup_set", { folder, hidden }),
};

export const exportApi = {
  /** Writes a new "Skykeep Export …" folder inside `dest`; photos only when asked (they can be huge). */
  start: (dest: string, photos: boolean) => invoke<ExportSummary>("library_export_start", { dest, photos }),
  /** Stops the export under way; nothing it wrote is left. */
  cancel: () => invoke<void>("library_export_cancel"),
  /** Shows the last finished export in the file manager. */
  reveal: () => invoke<void>("library_export_reveal"),
};

/* ---- Calendar: subscriptions by address ------------------------------------------------ */

/** A calendar subscribed to by address (webcal:// or https://), read-only, kept on this computer. */
export type CalendarSubscription = {
  id: string;
  url: string;
  title: string;
  colour: string;
  /** How often it is fetched again, in minutes; 0 for only when asked. */
  refreshMinutes: number;
  fetchedAt: number | null;
  /** Why the last fetch did not land, when it did not. */
  problem: string | null;
};

/** Where a subscription's calendar guid begins. */
export const SUBSCRIPTION_PREFIX = "quince:sub:";

export const calendarFeedsApi = {
  list: () => invoke<CalendarSubscription[]>("calendar_subscriptions"),
  /** Fetched once before it is kept, so a wrong address is said at once. */
  subscribe: (url: string) => invoke<CalendarSubscription>("calendar_subscribe", { url }),
  update: (id: string, change: { title?: string; colour?: string; refreshMinutes?: number }) =>
    invoke<void>("calendar_subscription_update", {
      id, title: change.title ?? null, colour: change.colour ?? null, refreshMinutes: change.refreshMinutes ?? null,
    }),
  unsubscribe: (id: string) => invoke<void>("calendar_unsubscribe", { id }),
  refresh: (id: string) => invoke<CalendarSubscription>("calendar_subscription_refresh", { id }),
};
