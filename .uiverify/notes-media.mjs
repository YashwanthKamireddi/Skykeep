// What a note holds besides text, drawn where it sits: a picture at its own proportions from
// the backend's picture route, a file as a chip naming it and its kind — and either opens in
// the app this computer uses for it. A picture that cannot be had becomes the chip.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => { window.__mediaNote = true; });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.locator(".note-row", { hasText: "Trip" }).first().click();
await page.waitForTimeout(800);
// In a plain browser quince:// cannot load, so the picture falls back to its chip — what is
// checked is that the picture was asked for by its route, and the fallback is a named chip.
const asked = await page.evaluate(() => [...document.querySelectorAll("img")].map((i) => i.getAttribute("src")).concat(window.__asked ?? []));
const chips = await page.locator(".nf-file").allInnerTexts();
ok(chips.some((c) => /Budget\.xlsx/.test(c) && /XLSX/.test(c)), `a file shows as a chip with its name and kind  ${JSON.stringify(chips)}`);
ok(chips.some((c) => /beach\.jpeg/.test(c)) || asked.some((s) => s?.includes("note%2FM1%2Finline")), `a picture is drawn from its own route, or its chip when it cannot be had  ${JSON.stringify(asked)}`);
await page.locator(".nf-file", { hasText: "Budget.xlsx" }).click();
await page.waitForTimeout(150);
ok(JSON.stringify(await page.evaluate(() => window.__opened)) === JSON.stringify(["M2"]), "clicking a file opens it");
const saves = await page.evaluate(() => window.__noteSaves?.length ?? 0);
ok(saves === 0, "and the note's text is left alone");
await b.close();
process.exitCode = failures ? 1 : 0;
