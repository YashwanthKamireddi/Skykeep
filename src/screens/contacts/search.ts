/**
 * Finding and ordering people in the address book the window already holds.
 *
 * The search used to go to the engine on every key: it read every card, number and detail out
 * of the library, matched them in Rust and sent the whole book back (about 5 MB at 10,000
 * cards), and the list sorted it twice more, rebuilding each card's sort key and running a
 * Unicode test inside every comparison. The window has the whole book already, so it matches
 * here, with the same rules as `Contact::matches` in quince-core (`contacts.rs`), and each
 * card's folded text and sort key are worked out once per card object (a card unchanged by a
 * read keeps its object, lib/same) rather than once per key or comparison.
 */
import type { Contact } from "../../lib/ipc";

/** Case and accents gone, as `quince_core::fold::fold` does: "Émile", "EMILE" and "emile" match. */
export function fold(text: string): string {
  return text
    .normalize("NFD")
    .replace(/[̀-ͯ᪰-᫿᷀-᷿⃐-⃿︠-︯]/g, "")
    .toLowerCase();
}

/** The words of a query, folded; none when there is nothing to look for. */
export function words(query: string): string[] {
  return query.split(/\s+/).filter(Boolean).map(fold);
}

const digits = (text: string) => text.replace(/[^0-9]/g, "");
/** A query word that reads as (part of) a phone number rather than a name. */
const looksLikeNumber = (w: string) => /[0-9]/.test(w) && /^[0-9+\-().\/]+$/.test(w);

type Haystack = { text: string; numbers: string[] };
const haystacks = new WeakMap<Contact, Haystack>();
function haystack(c: Contact): Haystack {
  let h = haystacks.get(c);
  if (!h) {
    let text = [c.firstName, c.middleName, c.lastName, c.company, c.nickname, c.jobTitle, c.department, c.phoneticFirstName, c.phoneticLastName]
      .filter((s): s is string => !!s)
      .join(" ");
    // A date is not something a person searches for by its digits.
    for (const m of c.methods ?? []) if (m.kind !== "date") text += `\n${m.value}`;
    h = { text: fold(text), numbers: (c.methods ?? []).filter((m) => m.kind === "phone").map((m) => digits(m.value)) };
    haystacks.set(c, h);
  }
  return h;
}

/** Whether a card is found by these folded words: every word in its names, work or methods. */
export function matches(c: Contact, found: string[]): boolean {
  if (found.length === 0) return true;
  const { text, numbers } = haystack(c);
  // A whole number typed or pasted (its words together: "+91 98765 43210") is found however
  // the card writes it, with or without the country code or a trunk 0, by its last digits.
  const typed = found.filter(looksLikeNumber).map(digits).join("");
  const tail = (d: string) => d.slice(Math.max(0, d.length - 10));
  const whole =
    typed.length >= 7 &&
    numbers.some((n) => n.includes(typed) || (n.length >= 7 && (tail(n).endsWith(tail(typed)) || tail(typed).endsWith(tail(n)))));
  return found.every(
    (w) => text.includes(w) || (looksLikeNumber(w) && (whole || numbers.some((n) => n.includes(digits(w))))),
  );
}

/** The people a query finds, in the order given. */
export function search(list: Contact[], query: string): Contact[] {
  const found = words(query);
  return found.length ? list.filter((c) => matches(c, found)) : list;
}

/**
 * The order Contacts keeps: by last name, then first (a company by its name), letters before
 * anything else, accents and case aside, and a card with no name at all ("#") last.
 */
const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });
type Order = { key: string; letter: string };
const orders = new WeakMap<Contact, Order>();
function order(c: Contact): Order {
  let o = orders.get(c);
  if (!o) {
    const key = c.isCompany && c.company ? c.company : [c.lastName, c.firstName, c.middleName].filter(Boolean).join(" ") || c.company || "";
    const first = key.trim().charAt(0).toLocaleUpperCase();
    o = { key, letter: /\p{L}/u.test(first) ? first.normalize("NFD").charAt(0) : "#" };
    orders.set(c, o);
  }
  return o;
}
export const sortKey = (c: Contact) => order(c).key;
export const letterOf = (c: Contact) => order(c).letter;
export function sortedContacts(list: Contact[]): Contact[] {
  return list
    .map((c) => ({ c, o: order(c) }))
    .sort((a, b) => {
      const la = a.o.letter === "#", lb = b.o.letter === "#";
      if (la !== lb) return la ? 1 : -1;
      return collator.compare(a.o.key, b.o.key);
    })
    .map((x) => x.c);
}
/** Who comes first in that order, without sorting everyone. */
export function firstContact(list: Contact[]): Contact | undefined {
  let best: { c: Contact; o: Order } | undefined;
  for (const c of list) {
    const o = order(c);
    if (!best) { best = { c, o }; continue; }
    const la = o.letter === "#", lb = best.o.letter === "#";
    if (la !== lb ? !la : collator.compare(o.key, best.o.key) < 0) best = { c, o };
  }
  return best?.c;
}
