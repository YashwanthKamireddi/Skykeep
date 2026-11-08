// Pinning a note, as on the phone: it moves to Pinned at the top of the list and back again.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: false, notesFixture: true, contactsFixture: false, driveFixture: false, remindersFixture: false });
const page = await ctx.newPage();
await page.goto(process.env.BASE ?? process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);

let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };

/** The titles under the Pinned heading, in order. */
const pinned = () => page.evaluate(() => {
  const out = [];
  let inPinned = false;
  for (const el of document.querySelectorAll(".note-rows .notes-section, .note-rows .note-row-title")) {
    if (el.classList.contains("notes-section")) inPinned = el.textContent.trim() === "Pinned";
    else if (inPinned) out.push(el.textContent.trim());
  }
  return out;
});

await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.waitForTimeout(300);
await page.getByRole("button", { name: "Pin Note" }).click();
await page.waitForTimeout(500);
const after = await pinned();
ok(after.includes("Curtains"), "a pinned note moves to Pinned", `Pinned holds ${JSON.stringify(after)}`);
const sent = await page.evaluate(() => window.__pins ?? []);
ok(JSON.stringify(sent) === JSON.stringify([["n1", true]]), "and the pin is sent to iCloud for that note",
  `sent ${JSON.stringify(sent)}`);

await page.getByRole("button", { name: "Unpin Note" }).click();
await page.waitForTimeout(500);
const back = await pinned();
ok(!back.includes("Curtains"), "unpinned, it leaves Pinned", `Pinned still holds ${JSON.stringify(back)}`);

await b.close();
process.exitCode = failures === 0 ? 0 : 1;
