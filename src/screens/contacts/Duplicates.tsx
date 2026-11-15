import { ChevronRight, UsersRound } from "lucide-react";
import { t, activeLanguage, type Key } from "../../lib/i18n";
import { displayName, type Contact, type Duplicates } from "../../lib/ipc";

/** What a group of cards shares, in a sentence. */
function shared(reasons: string[]): string {
  const one: Record<string, Key> = { name: "contacts.dupes.sameName", phone: "contacts.dupes.samePhone", email: "contacts.dupes.sameEmail" };
  const word: Record<string, Key> = { name: "contacts.dupes.partName", phone: "contacts.dupes.partPhone", email: "contacts.dupes.partEmail" };
  const known = reasons.filter((r) => word[r]);
  if (known.length === 0) return t("contacts.dupes.lookAlike");
  if (known.length === 1) return t(one[known[0]]);
  const list = new Intl.ListFormat(activeLanguage(), { type: "conjunction" }).format(known.map((r) => t(word[r])));
  return t("contacts.dupes.sameMany", { list });
}

/** The person a group is, by the name most of its cards give. */
function groupName(people: Contact[]): string {
  const named = people.map(displayName);
  return named.find((n) => /\p{L}/u.test(n)) ?? named[0] ?? t("contacts.dupes.noName");
}

/**
 * At the top of the list when some cards look like one person, as the iPhone says it: a
 * quiet line, never in the way, that opens the cards to look at.
 */
export function DuplicatesBanner({ count, onOpen }: { count: number; onOpen: () => void }) {
  return (
    <button type="button" className="dupes-banner" onClick={onOpen}>
      <UsersRound size={18} strokeWidth={1.8} aria-hidden="true" />
      <span className="dupes-banner-text">
        <span className="dupes-banner-title">{t("ui.duplicatesFound")}</span>
        <span className="dupes-banner-sub">
          {count === 1 ? "1 person has more than one card" : `${count} people have more than one card`}
        </span>
      </span>
      <ChevronRight size={16} strokeWidth={2} aria-hidden="true" className="dupes-banner-go" />
    </button>
  );
}

/**
 * The cards that look like one person, group by group: who, what they share, and one press to
 * make them one card. Nothing is lost in a merge — every number, address and date is kept —
 * and it can be undone until iCloud has it.
 */
export function DuplicatesView({ groups, book, Face, onMerge, onMergeAll, onShow, busy }: {
  groups: Duplicates[];
  book: Map<string, Contact>;
  Face: (props: { contact: Contact; large?: boolean }) => React.ReactNode;
  onMerge: (group: Duplicates) => void;
  onMergeAll: () => void;
  onShow: (guid: string) => void;
  busy: boolean;
}) {
  const shown = groups
    .map((g) => ({ g, people: g.guids.map((id) => book.get(id)).filter((c): c is Contact => !!c) }))
    .filter((x) => x.people.length > 1);
  if (shown.length === 0) {
    return (
      <div className="dupes-done">
        <UsersRound size={28} strokeWidth={1.5} aria-hidden="true" />
        <p>{t("contacts.dupes.allOne")}</p>
      </div>
    );
  }
  return (
    <section className="dupes" aria-label={t("contacts.dupes.title")}>
      <header className="dupes-head">
        <h2>{t("contacts.dupes.title")}</h2>
        <p>
          {t("contacts.dupes.intro")}
        </p>
        {shown.length > 1 && (
          <button type="button" className="primary" disabled={busy} onClick={onMergeAll}>
            {t("contacts.dupes.mergeAll")}
          </button>
        )}
      </header>
      <ul className="dupes-list">
        {shown.map(({ g, people }) => (
          <li key={g.guids.join("|")} className="dupes-group">
            <div className="dupes-faces" aria-hidden="true">
              {people.slice(0, 3).map((c) => <Face key={c.guid} contact={c} />)}
            </div>
            <div className="dupes-who">
              <span className="dupes-name">{groupName(people)}</span>
              <span className="dupes-why">
                {people.length} cards. {shared(g.reasons)}
              </span>
              <span className="dupes-cards">
                {people.map((c) => (
                  <button key={c.guid} type="button" className="link-button" onClick={() => onShow(c.guid)}>
                    {cardSummary(c)}
                  </button>
                ))}
              </span>
            </div>
            <button
              type="button"
              className="quiet dupes-merge"
              disabled={busy}
              aria-label={t("contacts.dupes.merge", { name: groupName(people) })}
              onClick={() => onMerge(g)}
            >
              {t("contacts.dupes.mergeButton")}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** One card in a group, told apart from the others: its name and the first thing on it. */
function cardSummary(c: Contact): string {
  const first = c.methods.find((m) => m.kind === "phone" || m.kind === "email");
  return first ? `${displayName(c)}, ${first.value}` : displayName(c);
}
