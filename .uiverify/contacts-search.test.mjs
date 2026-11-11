// Contacts' search runs in the window over the book it already holds, so it must find exactly
// whom the library's own search (`Contact::matches`, quince-core contacts.rs) finds: these are
// that file's search tests, case for case. And the list's order, worked out once per card.
import { test } from "node:test";
import assert from "node:assert/strict";
import { search, sortedContacts, firstContact, letterOf } from "../src/screens/contacts/search.ts";

const card = (guid, first, last, phone = `+91${guid}`, more = {}) => ({
  guid, firstName: first, middleName: null, lastName: last, company: null, isCompany: false,
  birthday: null, notes: null, etag: null, photoUrl: null, photoSignature: null,
  methods: [{ kind: "phone", label: "mobile", value: phone }], ...more,
});
const found = (book, q) => search(book, q).map((c) => c.guid);

test("case, accents and word order aside", () => {
  const book = [card("1", "Émile", "Zola"), card("2", "Zoë", "Ahuja")];
  assert.deepEqual(found(book, "emile"), ["1"]);
  assert.deepEqual(found(book, "ÉMILE"), ["1"]);
  assert.deepEqual(found(book, "zola emile"), ["1"]);
  assert.deepEqual(found(book, "zoe"), ["2"]);
  assert.deepEqual(found(book, "%"), []);
  assert.deepEqual(found(book, "_"), []);
});

test("a number with or without its country code or trunk 0", () => {
  const book = [card("1", "Asha", null, "98765 43210"), card("2", "Bela", null, "+44 7700 900123")];
  assert.deepEqual(found(book, "+91 98765 43210"), ["1"]);
  assert.deepEqual(found(book, "+919876543210"), ["1"]);
  assert.deepEqual(found(book, "098765 43210"), ["1"]);
  assert.deepEqual(found(book, "07700 900123"), ["2"]);
  assert.deepEqual(found(book, "7700"), ["2"]);
  assert.deepEqual(found(book, "98765"), ["1"]);
  assert.deepEqual(found(book, "+91 12345 67890"), []);
});

test("a number however it was typed", () => {
  const book = [card("1", "Meera", null, "+91 98765-43210")];
  for (const typed of ["9876543210", "98765 43210", "(987) 65", "+91-98765"]) assert.equal(found(book, typed).length, 1, typed);
  assert.deepEqual(found(book, "12345"), []);
});

test("a name or a number; a blank search is no filter", () => {
  const book = [card("555", "Meera", "Iyer"), card("777", "Rahul", "Nair")];
  assert.deepEqual(found(book, "meera"), ["555"]);
  assert.deepEqual(found(book, "+91777"), ["777"]);
  assert.equal(found(book, "   ").length, 2);
});

test("the rest of the name and work, and every method but a date", () => {
  const book = [
    card("1", "Ravi", null, "1", { nickname: "Bunny", jobTitle: "Architect", department: "Design", phoneticFirstName: "Raavi" }),
    card("2", "Zed", null, "2", { methods: [{ kind: "address", label: null, value: "2 New Rd\nPune 411001" }, { kind: "date", label: null, value: "1990-05-04" }] }),
    card("3", "Notes", null, "3", { notes: "pune" }),
  ];
  for (const q of ["bunny", "architect", "design", "raavi"]) assert.deepEqual(found(book, q), ["1"], q);
  assert.deepEqual(found(book, "pune"), ["2"], "an address finds; notes don't");
  assert.deepEqual(found(book, "1990"), [], "a date's digits don't");
});

test("the list's order: last name, a company by its name, no name last", () => {
  const book = [
    card("1", "Zoya", "Zutshi"),
    card("4", null, null, "+919"),
    card("2", "Émile", "Écrivain"),
    { ...card("3", null, null), company: "Acme", isCompany: true },
    card("5", "Ann", "Bose"),
  ];
  assert.deepEqual(sortedContacts(book).map((c) => c.guid), ["3", "5", "2", "1", "4"]);
  assert.equal(letterOf(book[2]), "E");
  assert.equal(letterOf(book[1]), "#");
  assert.equal(firstContact(book)?.guid, "3");
  assert.equal(firstContact([]), undefined);
});
