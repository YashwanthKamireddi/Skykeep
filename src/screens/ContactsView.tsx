import { Fragment, memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AppFoot } from "../components/AppFoot";
import { Face } from "../components/Face";
import { Unsent, Waiting } from "../components/Unsent";
import { useOutbox } from "../lib/outbox";
import { toast, toastSaved, useErrorToast } from "../components/Toast";
import { ConfirmSheet, useConfirm } from "../components/Confirm";
import { EmptyState } from "../components/EmptyState";
import { SearchField } from "../components/SearchField";
import { PaneTitle } from "../components/PaneTitle";
import { Camera, ChevronLeft, Plus, UserPlus, UsersRound, UserRound } from "lucide-react";
import {
  api,
  contactsApi,
  displayName,
  messageOf,
  photoSrc,
  type Contact,
  type ContactList,
  type Duplicates,
  type ContactMethod,
  type AddressEdit,
  type AddressPart,
  type PostalAddress,
  type ItemChanges,
} from "../lib/ipc";
import { Arriving } from "../components/EmptyState";
import { useArriving, useItemChanges } from "../lib/sync";
import { overlayOpen, useActiveRef, useScreenActive } from "../lib/screen";
import { Picker } from "../components/Picker";
import { RowMenu, type MenuEntry } from "../components/RowMenu";
import { pickOpen } from "../lib/pick";
import { useReveal } from "../lib/reveal";
import { FIRST_LABEL, OFFERED, labelText, offeredMatch, profileHref } from "../lib/contactLabels";
import { REFUSED_LINK, outsideHref } from "../lib/links";
import { DateField, dateWords } from "./contacts/DateField";
import { PhotoSheet } from "./contacts/PhotoSheet";
import { DuplicatesBanner, DuplicatesView } from "./contacts/Duplicates";
import { keepSame } from "../lib/same";
import { useChosen, useIsChosen, type Chosen } from "../lib/chosen";
import { useArrivals } from "../lib/arrival";
import { copyText } from "../lib/clipboard";
import { carryLabel } from "../lib/dragPicture";

/** The sidebar row that is every contact rather than a list. */
const ALL = "";

type MenuAt = { x: number; y: number; alignRight: boolean };

/** The kinds of detail a card can hold, in the order Apple lists them. */
const KINDS = [
  { value: "phone", label: "phone" },
  { value: "email", label: "email" },
  { value: "url", label: "link" },
] as const;

const CUSTOM = "\u0000custom";

/**
 * A contact's picture where there is one, initials where there is not.
 *
 * The image is allowed to fail — a photo Apple has moved, or no network yet — and when it
 * does the initials underneath are what shows, rather than a broken frame.
 */
function Avatar({ contact, large, preview }: { contact: Contact; large?: boolean; preview?: string | null }) {
  return (
    <span className={`avatar${large ? " large" : ""}`} aria-hidden="true">
      <Face src={preview !== undefined ? preview : photoSrc(contact)} initials={initials(contact)} />
    </span>
  );
}

/** The parts of a name and work that Contacts adds only when asked, in its order. */
const NAME_FIELDS = [
  { key: "prefix", label: "Prefix" },
  { key: "phoneticFirstName", label: "Phonetic first name" },
  { key: "middleName", label: "Middle name" },
  { key: "phoneticLastName", label: "Phonetic last name" },
  { key: "suffix", label: "Suffix" },
  { key: "nickname", label: "Nickname" },
  { key: "jobTitle", label: "Job title" },
  { key: "department", label: "Department" },
  { key: "phoneticCompany", label: "Phonetic company name" },
] as const;
type NameKey = (typeof NAME_FIELDS)[number]["key"];

/** The details a card can gain from Add Field, by Apple's names for them. */
const ADDABLE = [
  { kind: "phone", label: "Phone" },
  { kind: "email", label: "Email" },
  { kind: "url", label: "URL" },
  { kind: "date", label: "Date" },
  { kind: "related", label: "Related Name" },
  { kind: "profile", label: "Social Profile" },
  { kind: "im", label: "Instant Message" },
] as const;

/** The whole name, as the top of a card shows it: prefix and suffix included. */
function fullName(c: Contact): string {
  if (c.isCompany && c.company?.trim()) return c.company.trim();
  const name = [c.prefix, c.firstName, c.middleName, c.lastName].filter(Boolean).join(" ");
  if (!name.trim()) return displayName(c);
  return c.suffix ? `${name}, ${c.suffix}` : name;
}

/** The person's own name on a card, prefix and suffix included; empty when it has none. */
function personName(c: Contact): string {
  const name = [c.prefix, c.firstName, c.middleName, c.lastName].filter(Boolean).join(" ").trim();
  return name && c.suffix ? `${name}, ${c.suffix}` : name;
}

const blank = (): Contact => ({
  guid: "",
  firstName: null,
  middleName: null,
  photoUrl: null,
  photoSignature: null,
  lastName: null,
  company: null,
  isCompany: false,
  birthday: null,
  notes: null,
  etag: null,
  methods: [],
});

const KIND_LABEL: Record<string, string> = {
  phone: "phone",
  email: "email",
  url: "link",
  address: "address",
  profile: "social profile",
  date: "date",
  related: "related name",
  im: "instant message",
};

/** What a value of each kind looks like, as a field's hint. */
const PLACEHOLDER: Record<string, string> = {
  phone: "+1 555 0100",
  email: "name@example.com",
  url: "example.com",
  related: "Name",
  profile: "Username",
  im: "Username",
};

/** Every kind written back here. An address is written part by part, on its own (below). */
const EDITABLE_KINDS = new Set(["phone", "email", "url", "date", "related", "profile", "im"]);
/** The kinds a detail can be switched between, as the first three always could. */
const SWITCHABLE = new Set(["phone", "email", "url"]);

/** Initials for the avatar, from whichever name fields exist. */
function initials(contact: Contact): string {
  const name = displayName(contact).trim();
  const parts = name.split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  // A number-only contact has no useful initial, so show a glyph instead of a digit.
  if (/^[+\d]/.test(parts[0])) return "#";
  return parts
    .slice(0, 2)
    .map((p) => [...p][0]?.toUpperCase() ?? "")
    .join("");
}

export function ContactsView() {
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [search, setSearch] = useState("");
  const [openGuid, setOpenGuid] = useState<string | null>(null);
  /** In a narrow window, the person picked rather than the list — as on iPhone. */
  const [detail, setDetail] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useErrorToast(error, () => setError(null));
  const [confirm, confirmSheet] = useConfirm();
  // A pass that names the cards it changed has those alone read again (`patch`, below); one
  // that can't say moves `due`, and the book is read whole. Either, landing while another
  // screen is in front, is taken when this one comes back, once (lib/sync, lib/screen).
  const patch = useRef<(items: ItemChanges) => void>(() => {});
  const due = useItemChanges("contacts", (items) => patch.current(items));
  const active = useScreenActive();
  const arriving = useArriving("contacts");
  /** The card being written, or null while simply looking. */
  const [draft, setDraft] = useState<Contact | null>(null);
  /** Details whose label is being typed rather than picked, by their place on the card. */
  const [customAt, setCustomAt] = useState<Set<number>>(() => new Set());
  /** The photo sheet, open over the form. */
  const [photoSheet, setPhotoSheet] = useState(false);
  /** A picture chosen in the form, small, shown until iCloud has it; null once taken away. */
  const [draftPreview, setDraftPreview] = useState<string | null | undefined>(undefined);
  /** Name and work fields added to the form, besides those already holding something. */
  const [shownFields, setShownFields] = useState<Set<NameKey>>(() => new Set());
  const [addMenu, setAddMenu] = useState<MenuAt | null>(null);
  /** A detail just added takes the focus, once. */
  const [focusAt, setFocusAt] = useState<number | null>(null);
  /** A name field or the birthday just added takes the focus, once. */
  const [focusName, setFocusName] = useState<string | null>(null);
  /** Cards that look like one person, and whether they are what the pane shows. */
  const [dupes, setDupes] = useState<Duplicates[]>([]);
  const [showDupes, setShowDupes] = useState(false);
  /** The cards just made one, while their merge can still be undone. */
  const [lastMerge, setLastMerge] = useState<string[]>([]);
  /**
   * The card as the form opened it. Saving writes only what differs from it, over Apple's
   * card as it is then — so a change made on the phone while the form was open stays.
   */
  const draftBase = useRef<Contact | null>(null);
  /**
   * The open card's addresses part by part: as last read, or as just edited here — drawn at
   * once, and put back if Apple turns the change down.
   */
  const [addresses, setAddresses] = useState<{ guid: string; list: PostalAddress[] } | null>(null);
  /** Address writes go to Apple one after another; while any is out, a read does not undo it. */
  const addressQueue = useRef<Promise<unknown>>(Promise.resolve());
  const addressesOut = useRef(0);
  /** The addresses being written, and the list the form opened them from. */
  const [draftAddresses, setDraftAddresses] = useState<DraftAddress[]>([]);
  const addressBase = useRef<PostalAddress[]>([]);
  /** Addresses whose label is being typed, by their key. */
  const [addressCustom, setAddressCustom] = useState<Set<number>>(() => new Set());
  const addressKey = useRef(0);
  /** A just-added address takes the focus, once. */
  const [addedKey, setAddedKey] = useState<number | null>(null);
  const startAddresses = (list: PostalAddress[]) => {
    addressBase.current = list;
    setDraftAddresses(list.map((address, from) => ({ key: ++addressKey.current, from, address })));
    setAddressCustom(new Set());
    setAddedKey(null);
  };
  const resetForm = () => {
    setCustomAt(new Set());
    setDraftPreview(undefined);
    setShownFields(new Set());
    setFocusAt(null);
    setFocusName(null);
  };
  const startNew = () => {
    resetForm();
    setShowDupes(false);
    setDraft(blank());
    startAddresses([]);
  };
  const edit = (c: Contact) => {
    draftBase.current = c;
    setDraft({ ...c });
    resetForm();
    if (addresses?.guid === c.guid) startAddresses(addresses.list);
    else {
      startAddresses([]);
      contactsApi
        .addresses(c.guid)
        .then((list) => {
          if (draftBase.current === c) startAddresses(Array.isArray(list) ? list : []);
        })
        .catch(() => {});
    }
  };
  const [busy, setBusy] = useState(false);
  /** The detail whose label was just clicked to copy it, said for a moment. */
  const [copied, setCopied] = useState<string | null>(null);
  useEffect(() => {
    if (!copied) return undefined;
    const timer = window.setTimeout(() => setCopied(null), 1200);
    return () => window.clearTimeout(timer);
  }, [copied]);
  const [lists, setLists] = useState<ContactList[]>([]);
  /** The list being looked at, or ALL. */
  const [list, setList] = useState<string>(ALL);
  // A contact found by the search across everything: shown, from All Contacts and with this
  // screen's own search cleared, so nothing here hides it.
  useReveal("contacts", (r) => {
    setList(ALL);
    setSearch("");
    setDraft(null);
    setOpenGuid(r.id);
    setDetail(true);
  });
  /** A list whose name is being typed: "new" for one not made yet. */
  const [naming, setNaming] = useState<string | null>(null);
  const [listMenu, setListMenu] = useState<(MenuAt & { list: ContactList }) | null>(null);
  /** A contact's menu: its actions, or — one step in — the lists it can go into. */
  const [contactMenu, setContactMenu] = useState<(MenuAt & { contact: Contact; lists: boolean }) | null>(null);
  /** The card that is this person's own. */
  const [me, setMe] = useState<string | null>(null);
  const [moreMenu, setMoreMenu] = useState<MenuAt | null>(null);
  /** A contact being deleted while a list is open: out of the list, or gone everywhere? */
  const [deleting, setDeleting] = useState<Contact | null>(null);
  /** The people being dragged onto a list — one, or everyone selected. */
  const [dragging, setDragging] = useState<string[] | null>(null);
  /** More than one person selected, as Command- and Shift-click select in Contacts. */
  const [picked, setPicked] = useState<string[]>([]);
  const [pickMenu, setPickMenu] = useState<MenuAt | null>(null);
  const [dropOn, setDropOn] = useState<string | null>(null);
  /** List writes go to Apple one after another, in the order they were made. */
  const queue = useRef<Promise<unknown>>(Promise.resolve());

  /** Cards saved here and not yet at Apple, and any Apple turned down. */
  const outbox = useOutbox("contacts");
  // A card sent while another screen is in front is read when this one comes back, once — as
  // a landed pass is (`due`): out of sight, each send read the whole book, its duplicates and
  // the open card's addresses again, on the lock the screen in front was waiting for.
  const seenTick = useRef(outbox.tick);
  if (active) seenTick.current = outbox.tick;
  const tickDue = seenTick.current;
  /** Each read is numbered: one overtaken by a later one (a key typed since) never draws. */
  const reads = useRef(0);
  /** Whole reads on their way: a pass's cards landing meanwhile are read whole too (`patch`). */
  const reading = useRef(0);
  const read = useCallback(async (query: string) => {
    const mine = ++reads.current;
    reading.current += 1;
    let found: Contact[];
    try {
      found = await api.contactsList(query.trim() || null);
    } finally {
      reading.current -= 1;
    }
    if (mine !== reads.current) return;
    // Each card unchanged keeps its row as it was (lib/same).
    setContacts((was) => keepSame(was, Array.isArray(found) ? found : [], (c) => c.guid));
    setLoaded(true);
  }, []);
  /** The book as last drawn, for a handler that runs after awaiting something. */
  const shownBook = useRef(contacts);
  shownBook.current = contacts;
  /** Moves when the open card was among those a pass changed: its addresses are read again. */
  const [cardRead, setCardRead] = useState(0);

  // The library is the only thing this screen reads. `revision` ticks whenever a refresh
  // finishes — pressed or on the timer — so a change made on the phone lands here without
  // this screen knowing anything about the network.
  useEffect(() => {
    let cancelled = false;
    read(search).catch((e) => {
      if (!cancelled) {
        setError(messageOf(e));
        setLoaded(true);
      }
    });
    return () => {
      cancelled = true;
    };
    // Re-reading on every keystroke is the point: the filter runs in SQLite. `outbox.tick`: a
    // card sent, or turned down, changes what the library holds.
  }, [read, search, due, tickDue]);

  const readLists = useCallback(async () => {
    const [found, mine] = await Promise.all([api.contactLists(), api.contactMe()]);
    setLists(Array.isArray(found) ? found : []);
    setMe(typeof mine === "string" ? mine : null);
  }, []);

  /** People out as a vCard, named as icloud.com names the file. */
  const exportCards = async (people: Contact[], label?: string) => {
    if (people.length === 0) return;
    const name =
      people.length === 1 ? displayName(people[0])
      : people.length === 2 ? `${displayName(people[0])} and ${displayName(people[1])}`
      : label ?? "iCloud vCards";
    try {
      const path = await api.contactsExport(people.map((c) => c.guid), `${name}.vcf`);
      toastSaved(path);
    } catch (e) {
      setError(messageOf(e));
    }
  };
  /**
   * Export All Contacts, or the open list: everyone in it, whatever the search shows. With a
   * search typed it exported only the people found, under the whole book's name.
   */
  const exportWhole = async () => {
    let people = sorted;
    if (search.trim()) {
      try {
        const all = await api.contactsList(null);
        const members = list === ALL ? null : new Set((lists.find((l) => l.guid === list)?.members ?? []).map((m) => m.toUpperCase()));
        people = sortedContacts(members ? all.filter((c) => members.has(c.guid.toUpperCase())) : all);
      } catch (e) {
        setError(messageOf(e));
        return;
      }
    }
    await exportCards(people, openList?.name);
  };
  const importCards = async () => {
    try {
      // Any number of files at once, vCard or a CSV exported from Google Contacts or Outlook.
      const paths =
        (await pickOpen({
          multiple: true,
          title: "Import Contacts",
          filters: [{ name: "Contacts", extensions: ["vcf", "vcard", "csv"] }],
        })) ?? [];
      if (paths.length === 0) return;
      setBusy(true);
      const [added, failed, known] = await api.contactsImport(paths);
      await Promise.all([read(search), readLists()]);
      const people = (n: number) => `${n} ${n === 1 ? "contact" : "contacts"}`;
      toast(
        [
          `${people(added)} imported`,
          known ? `${people(known)} already in Contacts ${known === 1 ? "was" : "were"} left out` : "",
          failed ? `${failed} could not be read` : "",
        ]
          .filter(Boolean)
          .join(", ") + ".",
      );
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  };
  const makeMine = async (c: Contact) => {
    const was = me;
    setMe(c.guid);
    try {
      await api.contactSetMe(c.guid);
    } catch (e) {
      setMe(was);
      setError(messageOf(e));
    }
  };
  useEffect(() => {
    readLists().catch((e) => setError(messageOf(e)));
  }, [readLists, due]);

  const readDupes = useCallback(async () => {
    const found = await api.contactsDuplicates();
    setDupes(Array.isArray(found) ? found : []);
  }, []);
  useEffect(() => {
    readDupes().catch(() => {});
  }, [readDupes, due, tickDue]);
  /**
   * A pass that named the cards it changed: those alone are read and put in their places, and
   * every other row is left as it is. It used to read the whole book again for one card edited
   * on the phone — every card, every number — and compare each row. With a search typed, or a
   * read of the whole book already on its way, the book is read as the search has it instead:
   * that read decides who shows.
   */
  patch.current = (items: ItemChanges) => {
    const ids = [...items.added, ...items.changed];
    const next = () => {
      readDupes().catch(() => {});
      if (openGuid && (ids.includes(openGuid) || items.removed.includes(openGuid))) setCardRead((n) => n + 1);
    };
    if (search.trim() || reading.current > 0 || !loaded) {
      read(search).then(next, () => {});
      return;
    }
    const before = reads.current;
    (ids.length ? contactsApi.get(ids) : Promise.resolve([] as Contact[]))
      .then((found) => {
        // A whole read started since has every card as it is now.
        if (before !== reads.current) return;
        const fresh = new Map((Array.isArray(found) ? found : []).map((c) => [c.guid, c]));
        // Named but not found: gone since, as a removal is.
        const gone = new Set([...items.removed, ...ids.filter((id) => !fresh.has(id))]);
        setContacts((was) => {
          const placed = new Set<string>();
          const kept = was
            .filter((c) => !gone.has(c.guid))
            .map((c) => {
              const now = fresh.get(c.guid);
              if (!now) return c;
              placed.add(c.guid);
              return now;
            });
          const added = [...fresh.values()].filter((c) => !placed.has(c.guid));
          return added.length ? [...kept, ...added] : kept;
        });
        next();
      })
      .catch(() => read(search).then(next, () => {}));
  };

  /** Everyone by id, for the duplicates view to draw from. */
  const byGuid = useMemo(() => new Map(contacts.map((c) => [c.guid, c])), [contacts]);

  /**
   * Cards made one: shown at once, sent shortly after (contacts_merge.rs), and undoable until
   * they are. Merging a group the view shows goes on with the rest; the last shows the card.
   */
  const undoMerge = useCallback(async (kept: string[]) => {
    const undone = await Promise.all(kept.map((k) => api.contactsMergeUndo(k).catch(() => false)));
    setLastMerge([]);
    await Promise.all([read(search), readLists(), readDupes()]).catch(() => {});
    if (undone.every(Boolean)) toast({ message: "Merge undone.", tone: "done" });
    else toast({ message: "iCloud already has this merge, so the cards stay merged.", tone: "error" });
  }, [read, readLists, readDupes, search]);
  const mergeGroups = async (groups: Duplicates[]) => {
    if (groups.length === 0) return;
    const name = (g: Duplicates) => {
      const people = g.guids.map((id) => byGuid.get(id)).filter((c): c is Contact => !!c);
      return people.map(displayName).find((n) => /\p{L}/u.test(n)) ?? "this person";
    };
    setBusy(true);
    setError(null);
    const kept: string[] = [];
    try {
      for (const g of groups) kept.push(await api.contactsMerge(g.guids));
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
    await Promise.all([read(search), readLists(), readDupes()]).catch(() => {});
    if (kept.length === 0) return;
    setLastMerge(kept);
    const left = dupes.length - groups.length;
    if (left <= 0) {
      setShowDupes(false);
      setOpenGuid(kept[0]);
    }
    toast({
      message: groups.length === 1
        ? `Merged ${groups[0].guids.length} cards into one for ${name(groups[0])}.`
        : `Merged the cards of ${groups.length} people.`,
      tone: "done",
      duration: 10000,
      action: { label: "Undo", run: () => void undoMerge(kept) },
    });
  };
  // A list deleted elsewhere is not left open, empty, with nothing to say why.
  useEffect(() => {
    if (list !== ALL && !lists.some((l) => l.guid === list)) setList(ALL);
  }, [lists, list]);

  // The open card's addresses, part by part — read again whenever the library may have moved,
  // unless a change of this screen's own is still on its way to Apple.
  useEffect(() => {
    if (!openGuid || openGuid.startsWith("pending-")) return undefined;
    let cancelled = false;
    if (addressesOut.current === 0)
      contactsApi
        .addresses(openGuid)
        .then((list) => {
          if (!cancelled && addressesOut.current === 0) setAddresses({ guid: openGuid, list: Array.isArray(list) ? list : [] });
        })
        .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [openGuid, due, tickDue, cardRead]);

  /**
   * Address edits: drawn at once, then sent. Turned down, the list goes back to what it was,
   * then to what Apple holds, and a toast says so with the way to try again.
   */
  const sendAddresses = useCallback(
    (guid: string, base: PostalAddress[], edits: AddressEdit[], after: PostalAddress[]) => {
      setAddresses({ guid, list: after });
      addressesOut.current += 1;
      addressQueue.current = addressQueue.current.then(async () => {
        try {
          const written = await contactsApi.addressEdit(guid, base, edits);
          if (Array.isArray(written)) setAddresses((cur) => (cur?.guid === guid ? { guid, list: written } : cur));
          read(search).catch(() => {});
        } catch (e) {
          setAddresses((cur) => (cur?.guid === guid ? { guid, list: base } : cur));
          toast({
            message: `Address not saved. ${messageOf(e)}`,
            tone: "error",
            action: { label: "Try Again", run: () => sendAddresses(guid, base, edits, after) },
          });
          const held = await contactsApi.addresses(guid).catch(() => null);
          if (Array.isArray(held)) setAddresses((cur) => (cur?.guid === guid ? { guid, list: held } : cur));
        } finally {
          addressesOut.current -= 1;
        }
      });
    },
    [read, search],
  );

  /** Who shows: everyone, or the people in the open list. */
  const shown = useMemo(() => {
    if (list === ALL) return contacts;
    const members = new Set((lists.find((l) => l.guid === list)?.members ?? []).map((m) => m.toUpperCase()));
    return contacts.filter((c) => members.has(c.guid.toUpperCase()));
  }, [contacts, lists, list]);

  useEffect(() => {
    if (shown.length === 0) return setOpenGuid(null);
    setOpenGuid((current) =>
      current && shown.some((c) => c.guid === current) ? current : sortedContacts(shown)[0].guid
    );
  }, [shown]);

  /**
   * A change to a list: drawn at once, then sent. When Apple says no, the lists are read
   * back as Apple holds them, so nothing stays on screen that did not happen.
   */
  const changeList = useCallback(
    (predict: (lists: ContactList[]) => ContactList[], change: Parameters<typeof api.contactListWrite>[0]) => {
      setLists(predict);
      setError(null);
      const sent = queue.current.then(async () => {
        try {
          const guid = await api.contactListWrite(change);
          await readLists();
          return guid;
        } catch (e) {
          setError(messageOf(e));
          await readLists().catch(() => {});
          return null;
        }
      });
      queue.current = sent;
      return sent;
    },
    [readLists]
  );

  const createList = useCallback(
    async (name: string, members: string[] = []) => {
      const title = name.trim() || "Untitled List";
      const standIn = `pending-list-${Date.now()}`;
      const guid = await changeList(
        (ls) => [...ls, { guid: standIn, name: title, members, etag: null }],
        { name: title, add: members }
      );
      if (guid) setList((current) => (current === standIn ? guid : current));
      else setList((current) => (current === standIn ? ALL : current));
    },
    [changeList]
  );
  const renameList = (l: ContactList, name: string) => {
    setNaming(null);
    const title = name.trim();
    if (!title || title === l.name) return;
    void changeList((ls) => ls.map((x) => (x.guid === l.guid ? { ...x, name: title } : x)), { guid: l.guid, name: title });
  };
  const deleteList = (l: ContactList) => {
    // icloud.com's own words for it.
    void confirm({
      title: `Delete \u201c${l.name}\u201d?`,
      body: "This list will be deleted from every device. The contacts in it stay in Contacts.",
      action: "Delete",
      danger: true,
      symbol: "trash",
    }).then((sure) => {
      if (!sure) return;
      if (list === l.guid) setList(ALL);
      void changeList((ls) => ls.filter((x) => x.guid !== l.guid), { guid: l.guid, delete: true });
    });
  };
  const addToList = (l: ContactList, people: string[]) => {
    const fresh = people.filter((p) => !l.members.some((m) => m.toUpperCase() === p.toUpperCase()));
    if (fresh.length === 0) return;
    void changeList(
      (ls) => ls.map((x) => (x.guid === l.guid ? { ...x, members: [...x.members, ...fresh] } : x)),
      { guid: l.guid, add: fresh }
    );
  };
  const removeFromList = (l: ContactList, people: string[]) => {
    const gone = new Set(people.map((p) => p.toUpperCase()));
    void changeList(
      (ls) => ls.map((x) => (x.guid === l.guid ? { ...x, members: x.members.filter((m) => !gone.has(m.toUpperCase())) } : x)),
      { guid: l.guid, remove: people }
    );
  };
  const openList = lists.find((l) => l.guid === list) ?? null;

  /** What a contact's menu offers — the same whether opened by right-click or the card. */
  const contactEntries = (c: Contact, choosingList: boolean): MenuEntry[] => {
    if (choosingList) {
      const into = lists.filter((l) => !l.guid.startsWith("pending-") && !l.members.some((m) => m.toUpperCase() === c.guid.toUpperCase()));
      return [
        ...into.map((l) => ({ label: l.name, run: () => addToList(l, [c.guid]) })),
        ...(into.length ? [null] : []),
        { label: "New List", run: () => void createList("Untitled List", [c.guid]) },
      ];
    }
    return [
      // A submenu of the lists, beside the line, as Contacts on the Mac has it.
      { label: "Add to List", submenu: contactEntries(c, true) },
      ...(openList ? [{ label: `Remove from \u201c${openList.name}\u201d`, run: () => removeFromList(openList, [c.guid]) }] : []),
      null,
      { label: "Edit Contact", run: () => { setOpenGuid(c.guid); edit(c); } },
      ...(me === c.guid ? [] : [{ label: "Make This My Card", run: () => void makeMine(c) }]),
      { label: "Export vCard", run: () => void exportCards([c]) },
      { label: "Delete Contact", danger: true, run: () => {
        askDelete(c);
      } },
    ];
  };

  /** After a write lands in the library: the book and the lists as it now holds them — a
   * deleted card leaves every list it was in — and who looks like whom. */
  const settleWrite = useCallback(() => {
    Promise.all([read(search), readLists()]).catch((e) => setError(messageOf(e)));
    readDupes().catch(() => {});
  }, [read, readLists, readDupes, search]);

  const saveDraft = useCallback(async () => {
    if (!draft) return;
    // A birthday added and left empty is none.
    const saving = { ...draft, birthday: draft.birthday || null };
    let guid = saving.guid;
    // Addresses are written part by part on their own; the rest of the card only if it changed.
    // A new card's go as additions, queued behind the card itself.
    const base = addressBase.current;
    const edits = addressEdits(base, draftAddresses);
    const after = predicted(base, draftAddresses);
    const cardChanged = !saving.guid || JSON.stringify(saving) !== JSON.stringify(draftBase.current);
    if (!cardChanged) {
      setDraft(null);
      if (edits.length) sendAddresses(saving.guid, base, edits, after);
      return;
    }
    // Shown at once as saved — the form put away, the card in the list and open — as every
    // write here is: the library takes it behind (it can wait out a pass storing, seconds on a
    // first sync of Photos), and Apple after that. Turned down, the card goes back to what it
    // was and the form comes back as it was being written, so nothing typed is lost.
    const was = draftBase.current;
    const stand: Contact = saving.guid ? saving : { ...saving, guid: `pending-card-${Date.now()}` };
    setContacts((all) => (saving.guid ? all.map((c) => (c.guid === saving.guid ? stand : c)) : [...all, stand]));
    setDraft(null);
    setPicked([]);
    setShowDupes(false);
    setOpenGuid(stand.guid);
    setError(null);
    if (edits.length && saving.guid) setAddresses({ guid: saving.guid, list: after });
    try {
      guid = await api.contactSave(saving, saving.guid ? was : null);
    } catch (e) {
      setContacts((all) =>
        saving.guid && was ? all.map((c) => (c.guid === saving.guid ? was : c)) : all.filter((c) => c.guid !== stand.guid),
      );
      if (edits.length && saving.guid) setAddresses({ guid: saving.guid, list: base });
      draftBase.current = was;
      setDraft(saving);
      setOpenGuid((cur) => (cur === stand.guid ? saving.guid || null : cur));
      setError(messageOf(e));
      return;
    }
    // The stand-in takes the id the card will keep, so the row and the selection stay put.
    if (!saving.guid) {
      setContacts((all) => all.map((c) => (c.guid === stand.guid ? { ...stand, guid } : c)));
      setOpenGuid((cur) => (cur === stand.guid ? guid : cur));
    }
    settleWrite();
    if (edits.length) sendAddresses(guid, base, edits, after);
  }, [draft, draftAddresses, sendAddresses, settleWrite]);

  /**
   * Deleting a person is not something to discover by accident. Inside a list, icloud.com
   * asks whether the card goes or only its place in the list — the same here.
   */
  const askDelete = (c: Contact) => {
    if (openList) setDeleting(c);
    else
      void confirm({ title: `Delete ${displayName(c)}?`, body: "The card is deleted from Contacts on every device.", action: "Delete", danger: true, symbol: "trash" }).then(
        (sure) => sure && void removeContact(c.guid),
      );
  };

  /**
   * Cards deleted: gone from the list at once, then from the library and Apple behind. Any the
   * library turns down come back where they were, and say why.
   */
  const removeContacts = useCallback(
    async (guids: string[]) => {
      const going = new Set(guids);
      const gone = shownBook.current.filter((c) => going.has(c.guid));
      setContacts((all) => all.filter((c) => !going.has(c.guid)));
      setDraft((d) => (d && going.has(d.guid) ? null : d));
      setError(null);
      const kept: string[] = [];
      let why: unknown = null;
      for (const guid of guids) {
        try {
          await api.contactDelete(guid);
        } catch (e) {
          kept.push(guid);
          why = e;
        }
      }
      if (kept.length) {
        const back = gone.filter((c) => kept.includes(c.guid));
        setContacts((all) => [...all.filter((c) => !kept.includes(c.guid)), ...back]);
        setError(messageOf(why));
      }
      settleWrite();
    },
    [settleWrite]
  );
  const removeContact = useCallback((guid: string) => removeContacts([guid]), [removeContacts]);

  /** Editing a method in the draft, by position. */
  const editMethod = (index: number, patch: Partial<ContactMethod>) =>
    setDraft((d) =>
      d
        ? {
            ...d,
            methods: d.methods.map((m, i) => (i === index ? { ...m, ...patch } : m)),
          }
        : d
    );

  const open = useMemo(
    () => contacts.find((c) => c.guid === openGuid) ?? null,
    [contacts, openGuid]
  );
  /** The open card's addresses part by part, once read; until then the lines Apple formatted. */
  const structured = open && addresses?.guid === open.guid ? addresses.list : null;
  // Who is still here of those picked — a search or a list change narrows them.
  const pickedPeople = useMemo(() => {
    const chosen = new Set(picked);
    return sortedContacts(shown.filter((c) => chosen.has(c.guid)));
  }, [shown, picked]);
  const multi = pickedPeople.length > 1;
  /** Who is selected — everyone picked, or the open card — told to the list's rows one by one. */
  const chosen = useChosen(multi ? picked : openGuid ? [openGuid] : []);
  /** How many of each list's people are here, counted once per change rather than per render. */
  const listCounts = useMemo(() => {
    const here = new Set(contacts.map((c) => c.guid.toUpperCase()));
    return new Map(lists.map((l) => [l.guid, l.members.filter((m) => here.has(m.toUpperCase())).length]));
  }, [contacts, lists]);
  const removeMany = async (people: Contact[]) => {
    const sure = await confirm({
      title: `Delete ${people.length} contacts?`,
      body: "They will be deleted from Contacts on every device.",
      action: "Delete",
      danger: true,
      symbol: "trash",
    });
    if (!sure) return;
    setPicked([]);
    await removeContacts(people.map((c) => c.guid));
  };

  /**
   * Sorted once per change to who is shown, not on every render: typing in the card's form
   * re-sorted the whole address book with each key, and redrew every row with it.
   */
  const sorted = useMemo(() => sortedContacts(shown), [shown]);
  /** Each person's letter, worked out once per list rather than twice per row per render. */
  const lettered = useMemo(() => sorted.map((c, i, all) => {
    const letter = letterOf(c);
    return { contact: c, letter, starts: i === 0 || letterOf(all[i - 1]) !== letter };
  }), [sorted]);
  const rowsRef = useRef<HTMLUListElement | null>(null);
  // A card made on the phone, landing in the list on screen, fades in (lib/arrival).
  const contactIds = useMemo(() => contacts.map((c) => c.guid), [contacts]);
  useArrivals(rowsRef, contactIds, `${list}\u0000${search.trim()}`, outbox.waiting.size > 0);
  const [listFocused, setListFocused] = useState(false);
  const searchRef = useRef<HTMLInputElement | null>(null);
  /** The row for a person brought into view and given focus, as the arrows and the index do. */
  const showRow = (guid: string, focus: boolean) =>
    requestAnimationFrame(() => {
      const row = rowsRef.current?.querySelector<HTMLButtonElement>(`.contact-row[data-guid="${CSS.escape(guid)}"]`);
      row?.scrollIntoView({ block: "nearest" });
      if (focus) row?.focus({ preventScroll: true });
    });

  /**
   * What a row does, as one object that never changes, so a row is redrawn only when what it
   * shows changes — with a long address book, a click redrew every row in it. The handlers
   * read the selection as it is at the moment they run.
   */
  const latest = useRef({ picked, openGuid, sorted });
  latest.current = { picked, openGuid, sorted };
  const rowActions = useMemo<RowActions>(
    () => ({
      click: (contact, e) => {
        const { picked, openGuid, sorted } = latest.current;
        if (e.metaKey || e.ctrlKey) {
          // Command/Ctrl-click adds or takes away one, keeping the rest.
          const base = picked.length ? picked : openGuid ? [openGuid] : [];
          const next = base.includes(contact.guid) ? base.filter((g) => g !== contact.guid) : [...base, contact.guid];
          setPicked(next.length > 1 ? next : []);
          if (next.length === 1) setOpenGuid(next[0]);
          return;
        }
        if (e.shiftKey && openGuid) {
          // Shift-click takes everyone between the last picked and this one.
          const order = sorted.map((c) => c.guid);
          const [a, z] = [order.indexOf(openGuid), order.indexOf(contact.guid)].sort((x, y) => x - y);
          if (a >= 0) {
            const range = order.slice(a, z + 1);
            setPicked(range.length > 1 ? range : []);
            return;
          }
        }
        setPicked((was) => (was.length ? [] : was));
        setShowDupes(false);
        setOpenGuid(contact.guid);
        setDetail(true);
      },
      dragStart: (contact, e) => {
        const { picked } = latest.current;
        e.dataTransfer.effectAllowed = "copy";
        e.dataTransfer.setData("text/plain", displayName(contact));
        const carried = picked.includes(contact.guid) ? picked : [contact.guid];
        carryLabel(e, displayName(contact), carried.length);
        setDragging(carried);
      },
      dragEnd: () => {
        setDragging(null);
        setDropOn(null);
      },
      menu: (contact, e) => {
        e.preventDefault();
        setOpenGuid(contact.guid);
        setContactMenu({ x: e.clientX, y: e.clientY, alignRight: false, contact, lists: false });
      },
    }),
    [],
  );

  /** The letters the shown names begin with, for the index down the list's edge. */
  const letters = useMemo(() => lettered.filter((r) => r.starts).map((r) => r.letter), [lettered]);
  /** The index earns its column only when the list is longer than the window shows. */
  const [overflowing, setOverflowing] = useState(false);
  useEffect(() => {
    const list = rowsRef.current;
    if (!list || typeof ResizeObserver === "undefined") return undefined;
    const measure = () => setOverflowing(list.scrollHeight > list.clientHeight + 1);
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(list);
    return () => watch.disconnect();
  }, [sorted]);
  const jumpTo = (letter: string) => {
    const list = rowsRef.current;
    const head = list?.querySelector<HTMLElement>(`.contact-letter[data-letter="${CSS.escape(letter)}"]`);
    if (list && head) list.scrollTop = head.offsetTop;
  };

  // The keys Contacts on the Mac answers (Ctrl in place of Command here): ⌘N a new card, ⇧⌘N
  // a new list, ⌘F the search, ⌘L edits the card, the arrows walk the list — Shift with them
  // widens the selection — Return edits the card, Delete deletes it, and Escape leaves a card
  // being written unsaved.
  const live = useActiveRef();
  const listsColumn = useRef<HTMLElement | null>(null);
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e: KeyboardEvent) => {
    if (!live.current || e.defaultPrevented) return;
    // A menu, a picker's list or a sheet open over the screen has the keys to itself.
    if (overlayOpen()) return;
    const target = e.target as HTMLElement | null;
    const inSearch = target === searchRef.current;
    const typing = !inSearch && !!target?.closest?.("input, textarea, select, [contenteditable]");
    const command = (e.metaKey || e.ctrlKey) && !e.altKey;
    const k = e.key.toLowerCase();
    if (command && !e.shiftKey && k === "n") {
      e.preventDefault();
      if (!busy) {
        setPicked([]);
        startNew();
      }
      return;
    }
    if (command && e.shiftKey && k === "n") {
      e.preventDefault();
      if (draft) return;
      // Named in place in the lists column; where that has folded away, made as Untitled List.
      if (listsColumn.current?.offsetParent) setNaming("new");
      else void createList("Untitled List");
      return;
    }
    if (command && !e.shiftKey && k === "l") {
      e.preventDefault();
      if (!draft && open && !multi && !busy) edit(open);
      return;
    }
    if (command && !e.shiftKey && k === "f") {
      e.preventDefault();
      if (draft) return;
      setDetail(false);
      searchRef.current?.focus();
      searchRef.current?.select();
      return;
    }
    if (draft) {
      // Escape, from a field or not, leaves the card being written, as Cancel does.
      if (e.key === "Escape") {
        e.preventDefault();
        setDraft(null);
      }
      return;
    }
    if (typing || e.altKey || command) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (sorted.length === 0) return;
      e.preventDefault();
      const order = sorted.map((c) => c.guid);
      const at = openGuid ? order.indexOf(openGuid) : -1;
      const nextAt = Math.min(order.length - 1, Math.max(0, at + (e.key === "ArrowDown" ? 1 : -1)));
      const next = order[at < 0 ? 0 : nextAt];
      if (e.shiftKey && openGuid && at >= 0) {
        // Shift and an arrow: the run from where it began to here, as in Finder.
        const base = picked.length ? picked : [openGuid];
        const anchor = base[0];
        const [a, z] = [order.indexOf(anchor), order.indexOf(next)].sort((x, y) => x - y);
        const range = order.slice(a, z + 1);
        setPicked(range.length > 1 ? (anchor === range[0] ? range : [anchor, ...range.filter((g) => g !== anchor)]) : []);
        setOpenGuid(next);
      } else {
        // Left as it was when nothing is picked: a new empty list each step had who is picked
        // worked out again across the whole address book.
        setPicked((was) => (was.length ? [] : was));
        setOpenGuid(next);
      }
      showRow(next, !inSearch);
      return;
    }
    if (inSearch) {
      // Return in the search field goes to the first match, as Contacts does.
      if (e.key === "Enter" && sorted.length) {
        e.preventDefault();
        setOpenGuid(sorted[0].guid);
        setDetail(true);
        showRow(sorted[0].guid, true);
      }
      return;
    }
    if (e.key === "Enter" && open && !multi && !busy) {
      // A button with the focus would press itself; Return on the list means "edit".
      if (target?.closest?.("button") && !target.closest(".contact-row")) return;
      e.preventDefault();
      edit(open);
    } else if ((e.key === "Delete" || e.key === "Backspace") && !busy) {
      if (target?.closest?.(".contact-detail button")) return;
      e.preventDefault();
      if (multi) void removeMany(pickedPeople);
      else if (open) askDelete(open);
    }
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => keys.current(e);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    // A contact being written is always what shows; it has its own Cancel.
    <div className={`contacts${detail || draft ? " is-detail" : ""}`}>
      <aside className="contacts-lists" aria-label="Contact lists" ref={listsColumn}>
        <p className="side-heading">iCloud</p>
        <ListRow
          name="All Contacts"
          count={contacts.length}
          current={list === ALL}
          everyone
          onOpen={() => setList(ALL)}
        />
        {lists.map((l) =>
          naming === l.guid ? (
            <NameField key={l.guid} initial={l.name} label={`Rename ${l.name}`} onDone={(name) => renameList(l, name)} onCancel={() => setNaming(null)} />
          ) : (
            <ListRow
              key={l.guid}
              name={l.name}
              count={listCounts.get(l.guid) ?? 0}
              current={list === l.guid}
              menuOpen={listMenu?.list.guid === l.guid}
              dropping={dropOn === l.guid}
              onOpen={() => setList(l.guid)}
              onMenu={l.guid.startsWith("pending-") ? undefined : (at) => setListMenu({ ...at, list: l })}
              onDragOver={(e) => {
                if (!dragging || l.guid.startsWith("pending-")) return;
                e.preventDefault();
                e.dataTransfer.dropEffect = "copy";
                if (dropOn !== l.guid) setDropOn(l.guid);
              }}
              onDragLeave={() => setDropOn((d) => (d === l.guid ? null : d))}
              onDrop={(e) => {
                e.preventDefault();
                const people = dragging;
                setDragging(null);
                setDropOn(null);
                if (people) addToList(l, people);
              }}
            />
          )
        )}
        {naming === "new" && (
          <NameField initial="Untitled List" label="New list name" onDone={(name) => { setNaming(null); void createList(name); }} onCancel={() => setNaming(null)} />
        )}
        <button className="side-add" onClick={() => setNaming("new")}>
          <Plus size={15} strokeWidth={2} aria-hidden="true" />
          New List
        </button>
        {listMenu && (
          <RowMenu
            at={listMenu}
            onClose={() => setListMenu(null)}
            entries={[
              { label: "Rename List", run: () => setNaming(listMenu.list.guid) },
              null,
              { label: "Delete List", danger: true, run: () => deleteList(listMenu.list) },
            ]}
          />
        )}
        <AppFoot />
      </aside>

      <section className="contacts-list" aria-label="Contacts">
        <header className="pane-head contacts-head">
          <PaneTitle count={`${shown.length} ${shown.length === 1 ? "contact" : "contacts"}`}>{openList?.name ?? "Contacts"}</PaneTitle>
          <div className="contacts-head-actions">
            <button className="icon-button" aria-label="New contact" title="New Contact" disabled={busy} onClick={startNew}>
              <UserPlus size={18} strokeWidth={2} aria-hidden="true" />
            </button>
            <button
              className="icon-button"
              aria-label="More"
              aria-haspopup="menu"
              title="More"
              disabled={busy}
              onClick={(e) => {
                const r = e.currentTarget.getBoundingClientRect();
                setMoreMenu({ x: r.left, y: r.bottom + 4, alignRight: false });
              }}
            >
              •••
            </button>
          </div>
        </header>

        <SearchField
          ref={searchRef}
          className="contacts-search"
          value={search}
          onChange={setSearch}
          placeholder="Search"
          aria-label="Search contacts"
        />

        {/* Shown only when the lists column has folded away. */}
        <div className="contacts-list-picker">
          <Picker
            label="List"
            value={list}
            onChange={setList}
            options={[{ value: ALL, label: "All Contacts" }, ...lists.map((l) => ({ value: l.guid, label: l.name }))]}
          />
        </div>

        <Unsent refused={outbox.refused} what="a change to a contact" onResend={(r) => void outbox.resend(r)} onDiscard={(r) => void outbox.discard(r)} />
        {moreMenu && (
          <RowMenu
            at={moreMenu}
            onClose={() => setMoreMenu(null)}
            entries={[
              { label: "Import Contacts", run: () => void importCards() },
              { label: openList ? `Export \u201c${openList.name}\u201d` : "Export All Contacts", run: () => void exportWhole() },
            ]}
          />
        )}

        {dupes.length > 0 && list === ALL && !search && !showDupes && (
          <DuplicatesBanner
            count={dupes.length}
            onOpen={() => {
              setPicked([]);
              setDraft(null);
              setShowDupes(true);
              setDetail(true);
            }}
          />
        )}
        <div className="contact-rows-wrap">
        <ul
          className={`contact-rows${listFocused ? " is-focused" : ""}${letters.length > 1 && overflowing ? " has-index" : ""}`}
          ref={rowsRef}
          // A list box of people: the open card is the selected option, or every picked one.
          role="listbox"
          aria-label="Contacts"
          aria-multiselectable="true"
          data-rows=""
          // Whether the focus is in the list, for the selection's colour — a class rather than
          // :focus-within, which made WebKit restyle every row at each arrow press.
          onFocus={() => setListFocused(true)}
          onBlur={(e) => {
            if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setListFocused(false);
          }}
          onKeyDown={(e) => {
            // Command/Ctrl-A: everyone shown.
            if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a") {
              e.preventDefault();
              const everyone = shown.map((c) => c.guid);
              setPicked(everyone.length > 1 ? everyone : []);
            } else if (e.key === "Escape" && picked.length) {
              setPicked([]);
            }
          }}
        >
          <ContactRows
            lettered={lettered}
            chosen={chosen}
            me={me}
            draggable={lists.length > 0}
            waiting={outbox.waiting}
            actions={rowActions}
          />
        </ul>
        {/* The letters down the edge, as icloud.com and the phone index a long list: a press,
            or a drag along them, goes straight to that letter. */}
        {letters.length > 1 && overflowing && (
          <nav
            className="contact-index"
            aria-label="Jump to letter"
            onPointerDown={(e) => {
              if (e.button !== 0) return;
              const letter = (e.target as HTMLElement).closest?.<HTMLElement>(".contact-index button")?.dataset.letter;
              if (letter) jumpTo(letter);
              e.currentTarget.setPointerCapture?.(e.pointerId);
            }}
            onPointerMove={(e) => {
              if (!e.currentTarget.hasPointerCapture?.(e.pointerId)) return;
              const under = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
              const letter = under?.closest?.<HTMLElement>(".contact-index button")?.dataset.letter;
              if (letter) jumpTo(letter);
            }}
          >
            {letters.map((l) => (
              <button key={l} type="button" tabIndex={-1} data-letter={l} aria-label={`Contacts under ${l}`} onClick={() => jumpTo(l)}>
                {l}
              </button>
            ))}
          </nav>
        )}
        </div>

        {shown.length === 0 && !search && <Arriving service="contacts" what="contacts" />}
        {loaded && shown.length === 0 && !error && !(arriving && !search) && (
          <EmptyState className="contact-empty">{search ? "No matches." : openList ? "No one in this list yet." : "No contacts yet."}</EmptyState>
        )}
        {deleting && openList && (
          <ConfirmSheet<"list" | "card">
            title={`Delete ${displayName(deleting)}?`}
            label="Delete contact"
            body={<>Remove this contact from &ldquo;{openList.name}&rdquo;, or delete the card from Contacts on every device.</>}
            choices={[
              { label: "Remove from List", value: "list", primary: true, focus: true },
              { label: "Delete Card", value: "card", danger: true },
            ]}
            onAnswer={(how) => {
              const c = deleting;
              setDeleting(null);
              if (how === "list") removeFromList(openList, [c.guid]);
              else void removeContact(c.guid);
            }}
            onCancel={() => setDeleting(null)}
          />
        )}
        {confirmSheet}
        {contactMenu && (
          <RowMenu
            at={contactMenu}
            onClose={() => setContactMenu(null)}
            entries={contactEntries(contactMenu.contact, contactMenu.lists)}
          />
        )}
      </section>

      <article className="contact-detail" aria-label="Contact">
        {!draft && (
          <button className="compact-back" onClick={() => { setDetail(false); setShowDupes(false); }}>
            <ChevronLeft size={18} strokeWidth={2.25} aria-hidden="true" />
            <span>Contacts</span>
          </button>
        )}
        {draft ? (
          <form
            className="contact-form"
            onSubmit={(e) => {
              e.preventDefault();
              void saveDraft();
            }}
          >
            <div className="contact-form-head">
              <button
                type="button"
                className="contact-photo-edit"
                aria-label={draft.photoUrl ? "Edit photo" : "Add photo"}
                onClick={() => setPhotoSheet(true)}
              >
                <Avatar contact={draft} large preview={draftPreview} />
                <span className="contact-photo-badge" aria-hidden="true">
                  <Camera size={13} strokeWidth={2} />
                </span>
              </button>
              <h2>{draft.guid ? "Edit contact" : "New contact"}</h2>
            </div>
            {photoSheet && (
              <PhotoSheet
                hasPhoto={!!draft.photoUrl}
                onCancel={() => setPhotoSheet(false)}
                onRemove={() => {
                  setPhotoSheet(false);
                  setDraftPreview(null);
                  setDraft({ ...draft, photoUrl: null, photoSignature: null, photoCrop: null });
                }}
                onDone={(photo) => {
                  setPhotoSheet(false);
                  setDraftPreview(photo.preview);
                  setDraft({ ...draft, photoUrl: photo.url, photoSignature: null, photoCrop: photo.crop });
                }}
              />
            )}

            {(() => {
                const shown = (key: NameKey) => shownFields.has(key) || !!draft[key];
                const field = (key: NameKey | "firstName" | "lastName" | "company", label: string, first?: boolean) => (
                  <label key={key}>
                    <span>{label}</span>
                    <input
                      value={draft[key] ?? ""}
                      autoFocus={first || focusName === key}
                      onChange={(e) => setDraft({ ...draft, [key]: e.target.value || null })}
                    />
                  </label>
                );
                // The name as Contacts orders it, each phonetic spelling beside its part; then
                // the work, on lines of its own.
                return (
                  <>
                    <div className="contact-names">
                      {shown("prefix") && field("prefix", "Prefix")}
                      {field("firstName", "First", true)}
                      {shown("phoneticFirstName") && field("phoneticFirstName", "Phonetic first")}
                      {shown("middleName") && field("middleName", "Middle")}
                      {field("lastName", "Last")}
                      {shown("phoneticLastName") && field("phoneticLastName", "Phonetic last")}
                      {shown("suffix") && field("suffix", "Suffix")}
                      {shown("nickname") && field("nickname", "Nickname")}
                    </div>
                    <div className="contact-names">
                      {field("company", "Company")}
                      {shown("phoneticCompany") && field("phoneticCompany", "Phonetic company")}
                      {shown("jobTitle") && field("jobTitle", "Job title")}
                      {shown("department") && field("department", "Department")}
                    </div>
                  </>
                );
              })()}

            <ul className="contact-method-rows">
              {draft.methods.map((m, index) =>
                // An existing card's addresses are written part by part, below.
                m.kind === "address" && draft.guid ? null : !EDITABLE_KINDS.has(m.kind) ? (
                  // A new card's address lines, from a vCard: kept as they came.
                  <li key={index} className="contact-method-fixed">
                    <span>{KIND_LABEL[m.kind] ?? m.kind}</span>
                    <span className="contact-address">{m.value}</span>
                  </li>
                ) : (
                <li key={index}>
                  {SWITCHABLE.has(m.kind) ? (
                    <Picker
                      label={`Kind of detail ${index + 1}`}
                      className="contact-kind"
                      value={m.kind}
                      onChange={(kind) => editMethod(index, { kind })}
                      options={KINDS.map((k) => ({ value: k.value, label: k.label }))}
                    />
                  ) : (
                    <span className="contact-kind-fixed">{KIND_LABEL[m.kind] ?? m.kind}</span>
                  )}
                  {(() => {
                    const standard = OFFERED[m.kind] ?? [];
                    // Apple's names match in either spelling, so "mobile" is the phone's own.
                    const match = offeredMatch(m.kind, m.label);
                    // A service is chosen from Apple's list alone, as icloud.com offers it.
                    const fixedList = m.kind === "im";
                    const custom = !fixedList && (customAt.has(index) || (m.label !== null && !match));
                    return (
                      <span className="contact-label-cell">
                        <Picker
                          label={`${m.kind === "im" ? "Service" : "Label"} for detail ${index + 1}`}
                          className="contact-label-pick"
                          value={custom ? CUSTOM : (match ?? "")}
                          onChange={(v) => {
                            if (v === CUSTOM) {
                              setCustomAt((was) => new Set(was).add(index));
                              return;
                            }
                            setCustomAt((was) => { const next = new Set(was); next.delete(index); return next; });
                            // The label already on the card, in whichever spelling it came, when
                            // it is the one picked again.
                            editMethod(index, { label: v ? (match === v ? m.label : v) : null });
                          }}
                          options={[
                            ...(m.label === null && !custom ? [{ value: "", label: "no label" }] : []),
                            ...standard.map((v) => ({ value: v, label: labelText(v) })),
                            ...(fixedList ? [] : [{ value: CUSTOM, label: "Custom" }]),
                          ]}
                        />
                        {custom && (
                          <input
                            className="contact-method-label"
                            value={m.label ?? ""}
                            placeholder="label"
                            aria-label={`Custom label for detail ${index + 1}`}
                            onChange={(e) => editMethod(index, { label: e.target.value || null })}
                          />
                        )}
                      </span>
                    );
                  })()}
                  {m.kind === "date" ? (
                    <DateField
                      compact
                      label={`Date ${index + 1}`}
                      value={m.value || null}
                      autoFocus={focusAt === index}
                      onChange={(v) => editMethod(index, { value: v ?? "" })}
                    />
                  ) : (
                    <input
                      className="contact-method-value"
                      value={m.value}
                      placeholder={PLACEHOLDER[m.kind] ?? ""}
                      aria-label={`Value for detail ${index + 1}`}
                      autoFocus={focusAt === index}
                      onChange={(e) => editMethod(index, { value: e.target.value })}
                    />
                  )}
                  <button
                    type="button"
                    className="quiet danger"
                    aria-label={`Remove detail ${index + 1}`}
                    onClick={() => {
                      setDraft({
                        ...draft,
                        methods: draft.methods.filter((_, i) => i !== index),
                      });
                      // Places shift up by one; which were being typed is worked out again.
                      setCustomAt(new Set());
                      setFocusAt(null);
                    }}
                  >
                    ✕
                  </button>
                </li>
                ),
              )}
            </ul>

            {draftAddresses.length > 0 && (
              <ul className="contact-method-rows contact-address-rows" aria-label="Addresses">
                {draftAddresses.map((d, i) => (
                  <AddressEditor
                    key={d.key}
                    n={i + 1}
                    address={d.address}
                    custom={addressCustom.has(d.key)}
                    autoFocus={addedKey === d.key}
                    onChange={(patch) =>
                      setDraftAddresses((all) => all.map((x) => (x.key === d.key ? { ...x, address: { ...x.address, ...patch } } : x)))
                    }
                    onCustom={(on) =>
                      setAddressCustom((was) => {
                        const next = new Set(was);
                        if (on) next.add(d.key);
                        else next.delete(d.key);
                        return next;
                      })
                    }
                    onRemove={() => setDraftAddresses((all) => all.filter((x) => x.key !== d.key))}
                  />
                ))}
              </ul>
            )}

            {draft.birthday !== null && (
              <div className="contact-birthday-field">
                <span className="contact-kind-fixed">birthday</span>
                <DateField
                  label="Birthday"
                  value={draft.birthday || null}
                  autoFocus={focusName === "birthday"}
                  onChange={(v) => setDraft({ ...draft, birthday: v ?? "" })}
                />
                <button
                  type="button"
                  className="quiet danger"
                  aria-label="Remove birthday"
                  onClick={() => setDraft({ ...draft, birthday: null })}
                >
                  ✕
                </button>
              </div>
            )}

            <div className="contact-form-adds">
              <button
                type="button"
                className="quiet"
                aria-haspopup="menu"
                onClick={(e) => {
                  const r = e.currentTarget.getBoundingClientRect();
                  setAddMenu({ x: r.left, y: r.bottom + 4, alignRight: false });
                }}
              >
                <Plus size={14} strokeWidth={2} aria-hidden="true" />
                Add Field
              </button>
              {addMenu && (
                <RowMenu
                  at={addMenu}
                  onClose={() => setAddMenu(null)}
                  entries={[
                    ...ADDABLE.map((a) => ({
                      label: a.label,
                      run: () => {
                        setFocusAt(draft.methods.length);
                        setDraft({ ...draft, methods: [...draft.methods, { kind: a.kind, label: FIRST_LABEL[a.kind] ?? null, value: "" }] });
                      },
                    })),
                    {
                      label: "Address",
                      run: () => {
                        const key = ++addressKey.current;
                        setDraftAddresses((all) => [...all, { key, from: null, address: blankAddress() }]);
                        setAddedKey(key);
                      },
                    },
                    ...(draft.birthday === null
                      ? [{ label: "Birthday", run: () => { setFocusName("birthday"); setDraft({ ...draft, birthday: "" }); } }]
                      : []),
                    ...(() => {
                      const hidden = NAME_FIELDS.filter((f) => !shownFields.has(f.key) && !draft[f.key]);
                      return hidden.length
                        ? [null, ...hidden.map((f) => ({
                            label: f.label.charAt(0).toUpperCase() + f.label.slice(1),
                            run: () => {
                              setFocusName(f.key);
                              setShownFields((was) => new Set(was).add(f.key));
                            },
                          }))]
                        : [];
                    })(),
                  ]}
                />
              )}
            </div>

            <label className="contact-notes-field">
              <span>Notes</span>
              <textarea
                rows={3}
                value={draft.notes ?? ""}
                onChange={(e) => setDraft({ ...draft, notes: e.target.value || null })}
              />
            </label>

            <div className="cal-form-actions">
              <button
                type="submit"
                className="primary"
                disabled={
                  busy || !(draft.firstName || draft.lastName || draft.company)
                }
              >
                {busy ? "Saving" : draft.guid ? "Save" : "Add contact"}
              </button>
              <button type="button" className="quiet" onClick={() => setDraft(null)}>
                Cancel
              </button>
            </div>
          </form>
        ) : showDupes ? (
          <DuplicatesView
            groups={dupes}
            book={byGuid}
            Face={Avatar}
            busy={busy}
            onMerge={(g) => void mergeGroups([g])}
            onMergeAll={() => void mergeGroups(dupes)}
            onShow={(guid) => {
              setShowDupes(false);
              setOpenGuid(guid);
              showRow(guid, false);
            }}
          />
        ) : multi ? (
          <div className="contact-many">
            <div className="contact-many-faces" aria-hidden="true">
              {pickedPeople.slice(0, 3).map((c) => <Avatar key={c.guid} contact={c} large />)}
            </div>
            <h2>{pickedPeople.length} Contacts Selected</h2>
            <div className="cal-form-actions">
              <button
                className="quiet"
                aria-haspopup="menu"
                onClick={(e) => {
                  const r = e.currentTarget.getBoundingClientRect();
                  setPickMenu({ x: r.left, y: r.bottom + 4, alignRight: false });
                }}
              >
                Add to List
              </button>
              {openList && (
                <button className="quiet" onClick={() => { removeFromList(openList, pickedPeople.map((c) => c.guid)); setPicked([]); }}>
                  Remove from List
                </button>
              )}
              <button className="quiet" onClick={() => void exportCards(pickedPeople)}>Export vCards</button>
              <button className="quiet danger" disabled={busy} onClick={() => void removeMany(pickedPeople)}>Delete</button>
            </div>
            {pickMenu && (
              <RowMenu
                at={pickMenu}
                onClose={() => setPickMenu(null)}
                entries={[
                  ...lists.filter((l) => !l.guid.startsWith("pending-")).map((l) => ({ label: l.name, run: () => addToList(l, pickedPeople.map((c) => c.guid)) })),
                  ...(lists.length ? [null] : []),
                  { label: "New List", run: () => void createList("Untitled List", pickedPeople.map((c) => c.guid)) },
                ]}
              />
            )}
          </div>
        ) : open ? (
          <>
            {/* Photo and name together at the top of the card, as Contacts arranges them.
                The avatar used to be pinned to the pane's left edge while the name was
                centred somewhere else entirely, so they read as two unrelated things. */}
            <div className="contact-head">
              <Avatar contact={open} large />
              <div className="contact-head-names">
                {(open.phoneticFirstName || open.phoneticLastName) && (
                  <p className="contact-phonetic">{[open.phoneticFirstName, open.phoneticLastName].filter(Boolean).join(" ")}</p>
                )}
                <h2>{fullName(open)}</h2>
                {open.nickname && <p className="contact-nickname">{`\u201c${open.nickname}\u201d`}</p>}
                {(open.jobTitle || open.department) && (
                  <p className="contact-company">{[open.jobTitle, open.department].filter(Boolean).join(", ")}</p>
                )}
                {open.company && !open.isCompany && (
                  <p className="contact-company">{open.company}</p>
                )}
                {/* A company's card is headed by the company; the person on it comes under. */}
                {open.isCompany && open.company?.trim() && personName(open) && (
                  <p className="contact-company">{personName(open)}</p>
                )}
                {me === open.guid && <p className="contact-company">My Card</p>}
              </div>
            </div>

            <dl className="contact-methods">
              {(structured ? open.methods.filter((m) => m.kind !== "address") : open.methods).map((m, i) => {
                const key = `${m.kind}-${i}`;
                const label = (m.label && labelText(m.label)) || KIND_LABEL[m.kind] || m.kind;
                return (
                <div key={key}>
                  {/* The label copies what is beside it, as a click on one does in Contacts. */}
                  <dt>
                    <button
                      type="button"
                      className="contact-copy"
                      title={`Copy ${label}`}
                      aria-label={`Copy ${label}`}
                      onClick={() => void copyText(m.value).then((ok) => ok && setCopied(key))}
                    >
                      <span className={copied === key ? "is-hidden" : undefined}>{label}</span>
                      <span className={copied === key ? undefined : "is-hidden"} aria-hidden={copied !== key}>copied</span>
                    </button>
                  </dt>
                  <dd>
                    {m.kind === "email" ? (
                      <a href={`mailto:${m.value}`}>{m.value}</a>
                    ) : m.kind === "phone" ? (
                      <a href={`tel:${m.value.replace(/[^\d+#*,;]/g, "")}`}>{m.value}</a>
                    ) : m.kind === "url" ? (
                      webHref(m.value) ? <a href={webHref(m.value)!}>{m.value}</a> : <span title={REFUSED_LINK}>{m.value}</span>
                    ) : m.kind === "date" ? (
                      dateWords(m.value)
                    ) : m.kind === "profile" && profileHref(m.label, m.value) ? (
                      <a href={profileHref(m.label, m.value)!}>{m.label?.toUpperCase() === "TWITTER" ? `@${m.value.replace(/^@+/, "")}` : m.value}</a>
                    ) : m.kind === "address" ? (
                      // A postal address is several lines and stays that way.
                      <>
                        <span className="contact-address">{m.value}</span>
                        <a className="contact-maps" href={mapsHref(m.value)}>Show in Maps</a>
                      </>
                    ) : (
                      m.value
                    )}
                  </dd>
                </div>
                );
              })}
              {structured?.map((a, i) => {
                const key = `postal-${i}`;
                const label = (a.label && labelText(a.label)) || "address";
                const text = formatAddress(a);
                return (
                  <div key={key} className="contact-postal">
                    <dt>
                      <button
                        type="button"
                        className="contact-copy"
                        title={`Copy ${label} address`}
                        aria-label={`Copy ${label} address`}
                        onClick={() => void copyText(text).then((ok) => ok && setCopied(key))}
                      >
                        <span className={copied === key ? "is-hidden" : undefined}>{label}</span>
                        <span className={copied === key ? undefined : "is-hidden"} aria-hidden={copied !== key}>copied</span>
                      </button>
                    </dt>
                    <dd>
                      <span className="contact-address">{text}</span>
                      <a className="contact-maps" href={mapsHref(text)}>Show in Maps</a>
                    </dd>
                  </div>
                );
              })}
              {open.birthday && (
                <div>
                  <dt>birthday</dt>
                  <dd>{dateWords(open.birthday)}</dd>
                </div>
              )}
            </dl>

            {open.methods.length === 0 && !structured?.length && !open.birthday && (
              <p className="contact-none">No phone numbers or addresses saved.</p>
            )}
            {open.notes && <p className="contact-notes">{open.notes}</p>}

            <div className="cal-form-actions">
              <button className="quiet" disabled={busy} onClick={() => edit(open)}>
                Edit
              </button>
              {lastMerge.includes(open.guid) && outbox.waiting.has(open.guid) && (
                <button className="quiet" onClick={() => void undoMerge(lastMerge)}>
                  Undo Merge
                </button>
              )}
              <button
                className="quiet"
                aria-haspopup="menu"
                onClick={(e) => {
                  const r = e.currentTarget.getBoundingClientRect();
                  setContactMenu({ x: r.left, y: r.bottom + 4, alignRight: false, contact: open, lists: true });
                }}
              >
                Add to List
              </button>
              {openList && (
                <button className="quiet" onClick={() => removeFromList(openList, [open.guid])}>
                  Remove from List
                </button>
              )}
              <button
                className="quiet danger"
                disabled={busy}
                onClick={() => askDelete(open)}
              >
                Delete
              </button>
            </div>
          </>
        ) : (
          loaded && <EmptyState className="contact-pick">Select a contact.</EmptyState>
        )}
      </article>
    </div>
  );
}

type RowActions = {
  click: (contact: Contact, e: React.MouseEvent) => void;
  dragStart: (contact: Contact, e: React.DragEvent) => void;
  dragEnd: () => void;
  menu: (contact: Contact, e: React.MouseEvent) => void;
};

/**
 * Everyone in the list, under their letters. Drawn again only when who is in it changes — not
 * when the selection moves: the selection reaches each row through `chosen` (lib/chosen), so an
 * arrow press in an address book of thousands draws the two rows it moved between, without
 * making every row anew to find them.
 */
const ContactRows = memo(function ContactRows({ lettered, chosen, me, draggable, waiting, actions }: {
  lettered: { contact: Contact; letter: string; starts: boolean }[];
  chosen: Chosen;
  me: string | null;
  draggable: boolean;
  waiting: Set<string>;
  actions: RowActions;
}) {
  return (
    <>
      {lettered.map(({ contact, letter, starts }) => (
        <Fragment key={contact.guid}>
          {starts && (
            <li className="contact-letter" data-letter={letter} role="presentation" aria-hidden="true">{letter}</li>
          )}
          <ContactRow
            contact={contact}
            chosen={chosen}
            mine={me === contact.guid}
            draggable={draggable}
            waiting={waiting.has(contact.guid)}
            actions={actions}
          />
        </Fragment>
      ))}
    </>
  );
});

/**
 * One person in the list. Drawn again only when what it shows changes — its name, its picture,
 * whether it is selected — so a click or a key in a long address book touches two rows, not
 * every one. The layout of a row off screen is skipped too (content-visibility, in the CSS).
 */
const ContactRow = memo(function ContactRow({ contact, chosen, mine, draggable, waiting, actions }: {
  contact: Contact;
  chosen: Chosen;
  mine: boolean;
  draggable: boolean;
  /** Saved here, not yet at Apple. */
  waiting: boolean;
  actions: RowActions;
}) {
  const current = useIsChosen(chosen, contact.guid);
  return (
    <li
      role="presentation"
      // Dragged onto a list, a person joins it — as in Contacts on the Mac.
      draggable={draggable}
      onDragStart={(e) => actions.dragStart(contact, e)}
      onDragEnd={actions.dragEnd}
      onContextMenu={(e) => actions.menu(contact, e)}
    >
      <button
        className="contact-row"
        role="option"
        data-guid={contact.guid}
        aria-selected={current}
        onClick={(e) => actions.click(contact, e)}
      >
        <Avatar contact={contact} />
        <span className="contact-row-name"><RowName contact={contact} /></span>
        {mine && <span className="contact-me" title="My Card">me</span>}
        <Waiting shown={waiting} />
      </button>
    </li>
  );
});

/**
 * A name in the list as Contacts on the Mac sets it: the part it is sorted by in bold — the
 * last name, or a company's name — the rest of it plain, so the order reads at a glance.
 */
function RowName({ contact: c }: { contact: Contact }) {
  const company = c.company?.trim();
  const last = c.lastName?.trim();
  if (!(c.isCompany && company) && last) {
    const lead = [c.firstName, c.middleName].filter((p) => p?.trim()).join(" ");
    return (
      <>
        {lead && `${lead} `}
        <b>{last}</b>
      </>
    );
  }
  const name = displayName(c);
  // A card with only a number has nothing it is sorted by: plain.
  const named = (c.isCompany && company) || c.firstName?.trim() || c.middleName?.trim() || company;
  return named ? <b>{name}</b> : <>{name}</>;
}

/** A link a person typed without its scheme still opens: "example.com" is the web. One of a
 * kind Skykeep does not open (lib/links.ts) is no link: null, and it shows as its text. */
const webHref = (value: string) => {
  const href = /^[a-z][a-z0-9+.-]*:/i.test(value.trim()) ? value.trim() : `https://${value.trim()}`;
  return outsideHref(href) ? href : null;
};

/** A row in the lists column: All Contacts, or one list, with its count and ⋯. */
function ListRow({
  name,
  count,
  current,
  everyone = false,
  menuOpen = false,
  dropping = false,
  onOpen,
  onMenu,
  onDragOver,
  onDragLeave,
  onDrop,
}: {
  name: string;
  count: number;
  current: boolean;
  everyone?: boolean;
  menuOpen?: boolean;
  dropping?: boolean;
  onOpen: () => void;
  onMenu?: (at: MenuAt) => void;
  onDragOver?: React.DragEventHandler;
  onDragLeave?: React.DragEventHandler;
  onDrop?: React.DragEventHandler;
}) {
  const Glyph = everyone ? UsersRound : UserRound;
  return (
    <div
      className={`notes-folder-row${menuOpen ? " is-menu" : ""}${dropping ? " is-drop" : ""}`}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu?.({ x: e.clientX, y: e.clientY, alignRight: false });
      }}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <button className="source-row notes-folder" aria-current={current} onClick={onOpen}>
        <Glyph className="contacts-list-glyph" size={16} strokeWidth={2} aria-hidden="true" />
        <span className="source-name notes-folder-name">{name}</span>
        <span className="count notes-folder-count">{count || ""}</span>
      </button>
      {onMenu && (
        <button
          className="icon-button small round row-more notes-folder-more"
          aria-label={`Actions for ${name}`}
          aria-haspopup="menu"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            onMenu({ x: r.right, y: r.bottom + 4, alignRight: true });
          }}
        >
          <span aria-hidden="true">•••</span>
        </button>
      )}
    </div>
  );
}

/** A list's name being typed, in place: Return keeps it, Escape leaves it as it was. */
function NameField({ initial, label, onDone, onCancel }: {
  initial: string;
  label: string;
  onDone: (name: string) => void;
  onCancel: () => void;
}) {
  const settled = useRef(false);
  return (
    <input
      className="field notes-folder-rename"
      defaultValue={initial}
      aria-label={label}
      autoFocus
      onFocus={(e) => e.currentTarget.select()}
      onBlur={(e) => {
        if (settled.current) return;
        settled.current = true;
        onDone(e.currentTarget.value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") {
          settled.current = true;
          onCancel();
        }
      }}
    />
  );
}

/**
 * The order Contacts keeps: by last name, then first — a company by its name — letters
 * before anything else, accents and case aside, and a card with no name at all ("#") last.
 */
const contactCollator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });
function sortKey(c: Contact): string {
  if (c.isCompany && c.company) return c.company;
  return [c.lastName, c.firstName, c.middleName].filter(Boolean).join(" ") || c.company || "";
}
function letterOf(c: Contact): string {
  const first = sortKey(c).trim().charAt(0).toLocaleUpperCase();
  return /\p{L}/u.test(first) ? first.normalize("NFD").charAt(0) : "#";
}
function sortedContacts(list: Contact[]): Contact[] {
  return [...list].sort((a, b) => {
    const la = letterOf(a) === "#", lb = letterOf(b) === "#";
    if (la !== lb) return la ? 1 : -1;
    return contactCollator.compare(sortKey(a), sortKey(b));
  });
}

/* ── Postal addresses ─────────────────────────────────────────────────────────────────── */

/**
 * The lines of an address in the order its country writes them — the backend's
 * `PostalAddress::formatted`, line for line, so an address just edited here reads exactly as
 * it will once the card is read back from Apple.
 */
type AddressLine = [AddressPart, string][];
const LAYOUTS: Record<string, AddressLine[]> = {
  us: [[["street", ""]], [["subLocality", ""]], [["city", ""], ["state", ", "], ["postalCode", " "]], [["country", ""]]],
  gb: [[["street", ""]], [["subLocality", ""]], [["city", ""]], [["subAdministrativeArea", ""]], [["postalCode", ""]], [["country", ""]]],
  in: [[["street", ""]], [["subLocality", ""]], [["city", ""], ["postalCode", " "]], [["state", ""]], [["country", ""]]],
  de: [[["street", ""]], [["postalCode", ""], ["city", " "]], [["country", ""]]],
  jp: [[["postalCode", ""]], [["state", ""], ["city", " "], ["subLocality", " "]], [["street", ""]], [["country", ""]]],
  other: [[["street", ""]], [["subLocality", ""]], [["city", ""], ["state", " "], ["postalCode", " "]], [["country", ""]]],
};
LAYOUTS.fr = LAYOUTS.de;
const CODES: Record<string, string> = { us: "us", gb: "gb", uk: "gb", in: "in", de: "de", fr: "fr", jp: "jp" };
const NAMES: Record<string, string> = Object.fromEntries(
  ([
    ["us", ["united states", "united states of america", "usa", "us", "u.s.a", "u.s", "america"]],
    ["gb", ["united kingdom", "uk", "u.k", "great britain", "britain", "england", "scotland", "wales", "northern ireland", "gb"]],
    ["in", ["india", "bharat", "in"]],
    ["de", ["germany", "deutschland", "de"]],
    ["fr", ["france", "fr"]],
    ["jp", ["japan", "nippon", "日本", "jp"]],
  ] as const).flatMap(([code, names]) => names.map((n) => [n, code])),
);
/** Apple's code for a country typed by name, for the countries laid out by their own rules. */
function countryCodeFor(name: string): string | null {
  const folded = name.trim().normalize("NFD").replace(/\p{M}/gu, "").toLowerCase().replace(/\.+$/, "");
  return NAMES[folded] ?? null;
}
function formatAddress(a: PostalAddress): string {
  const key = (a.countryCode && CODES[a.countryCode.trim().toLowerCase()]) || (a.country && countryCodeFor(a.country)) || "other";
  const lines: string[] = [];
  for (const line of LAYOUTS[key]) {
    let text = "";
    for (const [part, separator] of line) {
      const value = a[part];
      if (!value) continue;
      if (text) text += separator;
      text += value;
    }
    lines.push(...text.split("\n").map((l) => l.trim()).filter(Boolean));
  }
  return lines.join("\n");
}
/** The address on one line, for a map to search for. */
const mapsHref = (text: string) => `geo:0,0?q=${encodeURIComponent(text.split("\n").join(", "))}`;

/** The parts the editor shows, in the order Tab walks them. */
const ADDRESS_PARTS = ["street", "city", "state", "postalCode", "country"] as const;
const PART_NAME: Record<(typeof ADDRESS_PARTS)[number], string> = {
  street: "Street",
  city: "City",
  state: "State",
  postalCode: "Postcode",
  country: "Country",
};
const ADDRESS_LABELS = OFFERED.address;
const ALL_PARTS: AddressPart[] = ["street", "subLocality", "city", "subAdministrativeArea", "state", "postalCode", "country", "countryCode"];

const blankAddress = (): PostalAddress => ({
  label: "HOME", street: null, subLocality: null, city: null, subAdministrativeArea: null,
  state: null, postalCode: null, country: null, countryCode: null,
});
/** A part as stored: trimmed, and nothing rather than an empty string. */
const tidy = (v: string | null | undefined) => (v && v.trim() ? v.trim() : null);
const isEmptyAddress = (a: PostalAddress) => ALL_PARTS.every((p) => !tidy(a[p]));

/** An address in the editor: where it sat in the list the form opened (null for a new one). */
type DraftAddress = { key: number; from: number | null; address: PostalAddress };

/**
 * What the editor's addresses change from the list it opened with, as the backend's edits:
 * parts and labels first (by their places in that list), then removals from the end back,
 * then additions — so every index names the address it meant.
 */
function addressEdits(base: PostalAddress[], draft: DraftAddress[]): AddressEdit[] {
  const edits: AddressEdit[] = [];
  for (const d of draft) {
    if (d.from === null) continue;
    const was = base[d.from];
    for (const part of ADDRESS_PARTS) {
      const value = tidy(d.address[part]);
      if (value !== tidy(was[part])) edits.push({ op: "setField", index: d.from, field: part, value });
    }
    const label = tidy(d.address.label);
    if (label !== tidy(was.label)) edits.push({ op: "setLabel", index: d.from, label });
  }
  const kept = new Set(draft.map((d) => d.from));
  for (let i = base.length - 1; i >= 0; i--) if (!kept.has(i)) edits.push({ op: "remove", index: i });
  for (const d of draft) {
    if (d.from === null && !isEmptyAddress(d.address)) edits.push({ op: "add", address: settled(d.address, null) });
  }
  return edits;
}

/** An address as the backend will store it: trimmed, and its country's code kept in step. */
function settled(a: PostalAddress, was: PostalAddress | null): PostalAddress {
  const out = { ...a, label: tidy(a.label) };
  for (const p of ALL_PARTS) out[p] = tidy(a[p]);
  if (!was ? !out.countryCode : tidy(was.country) !== out.country) out.countryCode = out.country ? countryCodeFor(out.country) : null;
  return out;
}
/** The list the edits will leave, drawn before Apple has it. */
const predicted = (base: PostalAddress[], draft: DraftAddress[]) =>
  draft.map((d) => settled(d.address, d.from === null ? null : base[d.from])).filter((a) => !isEmptyAddress(a));

/**
 * One address being written, laid out as Contacts on the Mac lays one out: the street over
 * as many lines as it takes, then town, region and postcode on one line, then the country.
 */
function AddressEditor({ n, address, custom, autoFocus, onChange, onCustom, onRemove }: {
  n: number;
  address: PostalAddress;
  custom: boolean;
  autoFocus: boolean;
  onChange: (patch: Partial<PostalAddress>) => void;
  onCustom: (custom: boolean) => void;
  onRemove: () => void;
}) {
  const match = ADDRESS_LABELS.find((v) => v === address.label?.toUpperCase());
  const typing = custom || (address.label !== null && !match);
  const field = (part: (typeof ADDRESS_PARTS)[number], extra?: { autoFocus?: boolean }) => (
    <input
      className={`contact-address-${part}`}
      value={address[part] ?? ""}
      placeholder={PART_NAME[part]}
      aria-label={`${PART_NAME[part]}, address ${n}`}
      autoComplete="off"
      autoFocus={extra?.autoFocus}
      onChange={(e) => onChange({ [part]: e.target.value || null })}
    />
  );
  const street = address.street ?? "";
  return (
    <li className="contact-address-edit">
      <span className="contact-address-kind">address</span>
      <Picker
        label={`Label for address ${n}`}
        className="contact-label-pick"
        value={typing ? CUSTOM : (match ?? "")}
        onChange={(v) => {
          onCustom(v === CUSTOM);
          if (v !== CUSTOM) onChange({ label: v || null });
        }}
        options={[
          ...(address.label === null && !typing ? [{ value: "", label: "no label" }] : []),
          ...ADDRESS_LABELS.map((v) => ({ value: v, label: labelText(v) })),
          { value: CUSTOM, label: "Custom" },
        ]}
      />
      {typing ? (
        <input
          className="contact-method-label contact-address-custom"
          value={address.label ?? ""}
          placeholder="label"
          aria-label={`Custom label for address ${n}`}
          onChange={(e) => onChange({ label: e.target.value || null })}
        />
      ) : (
        <span aria-hidden="true" />
      )}
      <button type="button" className="quiet danger" aria-label={`Remove address ${n}`} onClick={onRemove}>
        ✕
      </button>
      <div className="contact-address-fields">
        <textarea
          className="contact-address-street"
          value={street}
          rows={Math.max(2, street.split("\n").length)}
          placeholder="Street"
          aria-label={`Street, address ${n}`}
          autoFocus={autoFocus}
          onChange={(e) => onChange({ street: e.target.value || null })}
          onKeyDown={(e) => {
            // Return saves, as in every other field; Shift-Return starts another line.
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              e.currentTarget.form?.requestSubmit();
            }
          }}
        />
        <div className="contact-address-line">
          {field("city")}
          {field("state")}
          {field("postalCode")}
        </div>
        {field("country")}
      </div>
    </li>
  );
}
