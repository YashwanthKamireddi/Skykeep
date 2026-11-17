// Get Info: a file's kind, size, place and dates, as icloud.com's File Info shows them.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.locator(".drive-row", { hasText: "Semester Plan" }).hover();
await page.getByRole("button", { name: "Actions for Semester Plan.pdf" }).click();
await page.getByRole("menuitem", { name: "Get Info" }).click();
await page.waitForTimeout(200);
const facts = await page.evaluate(() => Object.fromEntries([...document.querySelectorAll(".info-rows > div")].map((d) => [d.querySelector("dt").textContent, d.querySelector("dd").textContent])));
ok(facts.Kind === "PDF document" && facts.Size === "2.3 MB" && facts.Where === "Drive", `File Info  ${JSON.stringify(facts)}`);
await page.keyboard.press("Escape");
await page.waitForTimeout(150);
ok((await page.locator(".info-sheet").count()) === 0, "Escape closes it");
// A right-click opens the same menu at the pointer; Escape puts it away.
await page.locator(".drive-row", { hasText: "Downloads" }).click({ button: "right" });
await page.waitForTimeout(150);
const offered = await page.getByRole("menuitem").allInnerTexts();
ok(JSON.stringify(offered) === JSON.stringify(["Open", "Download a Copy", "Get Info", "Rename", "Move to Folder", "Delete"]),
  `a right-click on a folder offers its menu  ${JSON.stringify(offered)}`);
await page.keyboard.press("Escape");
await page.locator(".drive-row", { hasText: "Semester Plan" }).click({ button: "right" });
await page.waitForTimeout(150);
const forFile = await page.getByRole("menuitem").allInnerTexts();
ok(JSON.stringify(forFile) === JSON.stringify(["Download a Copy", "Get Info", "Duplicate", "Rename", "Move to Folder", "Delete"]),
  `a file's menu adds Duplicate  ${JSON.stringify(forFile)}`);
await page.getByRole("menuitem", { name: "Duplicate" }).click();
await page.waitForTimeout(200);
ok(JSON.stringify(await page.evaluate(() => (window.__driveCopies ?? []).map((c) => c.drivewsid))) === JSON.stringify(["FILE::d"]), "Duplicate asks Apple for a copy of that file");
await page.keyboard.press("Escape");
await page.waitForTimeout(100);
ok((await page.getByRole("menu").count()) === 0, "and Escape puts it away");
// A click selects, as in Finder; it does not download. A double-click opens.
let downloads = 0;
await page.evaluate(() => { const t = window.__TAURI_INTERNALS__; const inner = t.invoke;
  t.invoke = async (c, a) => { if (c === "drive_download") window.__dl = (window.__dl ?? 0) + 1; return inner(c, a); }; });
await page.locator(".drive-row-hit", { hasText: "Semester Plan" }).click();
await page.waitForTimeout(200);
downloads = await page.evaluate(() => window.__dl ?? 0);
ok(downloads === 0 && (await page.locator(".drive-row.is-selected", { hasText: "Semester Plan" }).count()) === 1,
  "a click selects the file, and downloads nothing");
await page.locator(".drive-row-hit", { hasText: "Downloads" }).dblclick();
await page.waitForTimeout(300);
ok((await page.locator(".crumb", { hasText: "Downloads" }).count()) === 1, "a double-click opens the folder");
// Under the folder's name, as icloud.com: its count and what is free, in thousands.
const status = await page.locator(".drive-count").innerText();
ok(status === "4 items, 5.19 GB available", `the folder's status line  [${status}]`);
await b.close();
process.exitCode = failures ? 1 : 0;
