// Duplicate cards, as the iPhone finds them: a quiet "Duplicates Found" at the top of the
// list, each person with what their cards share, one press to merge — nothing lost — and
// Undo until iCloud has it.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(() => {
  window.__contactsExtra = [
    { guid: "c5", firstName: "Zoya", lastName: null, company: null, isCompany: false, birthday: null,
      notes: "Prefers WhatsApp", etag: null,
      methods: [{ kind: "phone", label: null, value: "090000 00001" }, { kind: "email", label: "WORK", value: "zoya@studio.example" }] },
    { guid: "c6", firstName: "Ravi", lastName: "Nair", company: null, isCompany: false, birthday: null, notes: null, etag: null,
      methods: [{ kind: "email", label: "HOME", value: "ravi@example.com" }] },
    { guid: "c7", firstName: "Ravi", lastName: "Nair", company: null, isCompany: false, birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: "MOBILE", value: "+91 98000 00007" }] },
  ];
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const rows = () => page.locator(".contact-row").count();

const banner = page.locator(".dupes-banner");
ok((await banner.innerText()).replace(/\s+/g, " ").includes("Duplicates Found 2 people have more than one card"), `the list says so, quietly  ${await banner.innerText()}`);
ok((await rows()) === 7, "every card is in the list until merged");
await banner.click();
await page.waitForTimeout(200);
const view = (await page.locator(".dupes").innerText()).replace(/\s+/g, " ");
ok(/Zoya Ahuja 2 cards\. Same phone number\./.test(view), `Zoya's cards share a number  ${view.slice(0, 220)}`);
ok(/Ravi Nair 2 cards\. Same name\./.test(view), "Ravi's cards share a name");
ok(view.includes("Merge All"), "and all of them can be merged at once");

await page.getByRole("button", { name: "Merge Zoya Ahuja" }).click();
await page.waitForTimeout(300);
ok(JSON.stringify(await page.evaluate(() => window.__merges?.at(-1))) === JSON.stringify({ guids: ["c1", "c5"] }), "the merge names both cards");
ok((await rows()) === 6, "one card where there were two, at once");
const toastText = await page.locator(".toast").last().innerText();
ok(/Merged 2 cards into one for Zoya Ahuja\./.test(toastText) && /Undo/.test(toastText), `said, with Undo  ${toastText}`);
ok((await page.locator(".dupes-group").count()) === 1, "the view goes on with the people left");

await page.locator(".toast").getByRole("button", { name: "Undo" }).click();
await page.waitForTimeout(300);
ok(JSON.stringify(await page.evaluate(() => window.__mergeUndos?.at(-1))) === JSON.stringify({ kept: "c1" }), "Undo takes the merge back");
ok((await rows()) === 7, "and both cards are back");
ok((await page.locator(".dupes-group").count()) === 2, "found again as duplicates");

await page.getByRole("button", { name: "Merge All" }).click();
await page.waitForTimeout(400);
ok((await rows()) === 5, "Merge All makes each person one card");
ok((await page.locator(".dupes-banner").count()) === 0, "and the list stops saying so");
const card = (await page.locator(".contact-detail").innerText()).replace(/\s+/g, " ");
ok(/zoya@studio\.example/.test(card) && /Prefers WhatsApp/.test(card), `the kept card holds what the other had  ${card.slice(0, 160)}`);

// Once iCloud has it, it stays merged, and the window says so.
await page.evaluate(() => { window.__mergeSent = true; });
await page.locator(".toast").getByRole("button", { name: "Undo" }).click();
await page.waitForTimeout(300);
ok(/iCloud already has this merge/.test(await page.locator(".toast").last().innerText()), "an Undo too late says why");
await page.screenshot({ path: process.env.SHOT ?? "/dev/null" }).catch(() => {});
await b.close();
process.exitCode = failures === 0 ? 0 : 1;
