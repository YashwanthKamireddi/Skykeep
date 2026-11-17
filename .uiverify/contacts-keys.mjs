// Contacts on the Mac by keyboard and by eye: ⌘N (Ctrl here) a new card and Escape out of it,
// the arrows walk the list, Return edits, Delete deletes, ⌘F the search and Return there the
// first match; a label click copies its value; a link is a link; and a long list gets the
// letter index down its edge and stays quick to walk.
import { engine, engineName } from "./engine.mjs";
import { SERVICES, autoConfirm, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
const page = await ctx.newPage();
await ctx.addInitScript(autoConfirm);
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const current = () => page.locator('.contact-row[aria-selected="true"] .contact-row-name').allInnerTexts();

await page.locator(".contact-row", { hasText: "Zoya Ahuja" }).click();
await page.keyboard.press("ArrowDown");
ok(JSON.stringify(await current()) === JSON.stringify(["Amma"]), `Down selects the next person  ${JSON.stringify(await current())}`);
ok((await page.locator(".contact-detail h2").innerText()) === "Amma", "and the card follows");
await page.keyboard.press("ArrowUp");
ok(JSON.stringify(await current()) === JSON.stringify(["Zoya Ahuja"]), "Up goes back");
await page.keyboard.press("Shift+ArrowDown");
await page.keyboard.press("Shift+ArrowDown");
ok((await page.locator(".contact-many h2").innerText().catch(() => "")) === "3 Contacts Selected", "Shift and the arrows widen the selection");
await page.keyboard.press("ArrowDown");
ok((await current()).length === 1, "a plain arrow narrows it to one again");

await page.locator(".contact-row", { hasText: "Zoya Ahuja" }).click();
await page.keyboard.press("Enter");
ok(await page.locator(".contact-form h2", { hasText: "Edit contact" }).isVisible(), "Return edits the card");
await page.keyboard.press("Escape");
ok(!(await page.locator(".contact-form").count()), "Escape leaves it unsaved");

await page.keyboard.press("Control+n");
ok(await page.locator(".contact-form h2", { hasText: "New contact" }).isVisible(), "Ctrl-N starts a new card");
ok(await page.evaluate(() => document.activeElement?.closest(".contact-names") !== null), "with the first name ready to type");
await page.keyboard.press("Escape");
ok(!(await page.locator(".contact-form").count()), "and Escape puts it away");

await page.locator(".contact-row", { hasText: "Amma" }).click();
await page.keyboard.press("Delete");
await page.waitForTimeout(200);
const asked = await page.evaluate(() => window.__asked ?? []);
ok(asked.some((m) => m.startsWith("Delete Amma")), `Delete asks first  ${JSON.stringify(asked)}`);
ok((await page.evaluate(() => window.__contactDeletes ?? [])).some((a) => a.guid === "c2"), "then deletes that card");

await page.keyboard.press("Control+f");
ok(await page.evaluate(() => document.activeElement?.closest(".contacts-search") !== null), "Ctrl-F goes to the search");
await page.keyboard.type("a");
await page.waitForTimeout(250);
await page.keyboard.press("Enter");
await page.waitForTimeout(100);
// (The stub answers every search with everyone; the first row shown is the first match.)
const first = await page.locator(".contact-row-name").first().innerText();
ok((await page.locator(".contact-detail h2").innerText()) === first, `Return in the search opens the first match  ${first}`);
ok(await page.evaluate(() => document.activeElement?.classList.contains("contact-row")), "and the list has the keys from there");
await page.locator(".contacts-search").fill("");
await page.locator(".contact-row", { hasText: "Northfield" }).click();
const link = await page.locator(".contact-methods a", { hasText: "northfield.example" }).getAttribute("href").catch(() => null);
ok(link === "https://northfield.example", `a link without its scheme opens on the web  ${link}`);
const tel = await page.locator(".contact-methods a", { hasText: "+1 555" }).getAttribute("href");
ok(tel === "tel:+15550100199", `a number dials without its spaces  ${tel}`);

await page.locator(".contact-row", { hasText: "Zoya Ahuja" }).click();
await ctx.grantPermissions(["clipboard-read", "clipboard-write"]).catch(() => {});
await page.getByRole("button", { name: "Copy mobile" }).click();
await page.waitForTimeout(120);
ok(await page.locator(".contact-copy span:not(.is-hidden)", { hasText: "copied" }).isVisible(), "a click on a label says it copied");
if (engineName === "chromium") {
  const clip = await page.evaluate(() => navigator.clipboard.readText()).catch(() => null);
  ok(clip === "+91 90000 00001", `and the value is on the clipboard  ${clip}`);
}
await page.waitForTimeout(1400);
ok(!(await page.locator(".contact-copy span:not(.is-hidden)", { hasText: "copied" }).count()), "then the label comes back");
ok(!(await page.locator(".contact-index").count()), "a short list has no index");
await ctx.close();

// A long address book: the index, and the arrows still quick.
const big = await b.newContext({ viewport: { width: 1280, height: 800 } });
await big.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
await big.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__many) return;
    const inner = t.invoke;
    const letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const people = Array.from({ length: 3000 }, (_, i) => ({
      guid: `m${i}`, firstName: `Person${i}`, lastName: `${letters[i % 26]}name${i}`, company: null, isCompany: false,
      birthday: null, notes: null, etag: null, methods: [],
    }));
    t.invoke = async (cmd, args) => {
      const out = await inner(cmd, args);
      if (cmd === "contacts_list" && Array.isArray(out) && !args?.query) return [...out, ...people];
      return out;
    };
    t.__many = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const p2 = await big.newPage();
await p2.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(p2, "Contacts");
await p2.waitForTimeout(800);
ok((await p2.locator(".contact-index button").count()) >= 26, "a long list has the letters down its edge");
// The letters take a narrow strip at the list's edge, not a column beside its scroll bar.
const room = await p2.evaluate(() => {
  const wrap = document.querySelector(".contact-rows-wrap").getBoundingClientRect();
  const row = document.querySelector(".contact-row").getBoundingClientRect();
  const index = document.querySelector(".contact-index").getBoundingClientRect();
  return { lost: Math.round(wrap.width - row.width), index: Math.round(index.width), edge: Math.round(wrap.right - index.right), overlap: row.right > index.left + 0.5 };
});
ok(room.index <= 16 && room.edge === 0 && room.lost <= 16 && !room.overlap, `the letters take a narrow strip at the edge, and the names nearly all the width  ${JSON.stringify(room)}`);
await p2.locator(".contact-index button", { hasText: "M" }).click();
await p2.waitForTimeout(100);
const top = await p2.evaluate(() => {
  const list = document.querySelector(".contact-rows");
  const head = document.querySelector('.contact-letter[data-letter="M"]');
  return Math.abs(head.offsetTop - list.scrollTop);
});
ok(top < 2, `a letter goes straight to its names  ${top}`);
await p2.locator(".contact-row").first().click();
const t0 = await p2.evaluate(() => performance.now());
for (let i = 0; i < 10; i++) await p2.keyboard.press("ArrowDown");
await p2.waitForFunction(() => document.querySelectorAll('.contact-row[aria-selected="true"]').length === 1);
const per = ((await p2.evaluate(() => performance.now())) - t0) / 10;
ok(per < 100, `walking 3000 people stays quick  ${per.toFixed(1)} ms a step`);
await b.close();
process.exitCode = failures ? 1 : 0;
