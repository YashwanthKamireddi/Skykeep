// Names iCloud Drive won't take are refused before anything is sent, in icloud.com's words:
// a leading dot (hidden), ":" or a line break, and a name already used in the folder.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: false, notesFixture: false, contactsFixture: false, driveFixture: true, remindersFixture: false });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(500);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const notice = () => page.locator(".toast.is-error").allInnerTexts();
const sent = () => page.evaluate(() => (window.__driveNames ?? []).length);

await page.getByRole("button", { name: "New Folder" }).click();
await page.keyboard.type(".hidden");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok((await notice()).some((t) => /shouldn.t begin with a .\..* \(dot\)/.test(t)) && (await sent()) === 0, `a folder named ".hidden" is refused, nothing sent  ${JSON.stringify(await notice())}`);

await page.getByRole("button", { name: "New Folder" }).click();
await page.keyboard.type("downloads");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok((await notice()).some((t) => /already taken/.test(t)) && (await sent()) === 0, "a name already in the folder, in any case, is refused");

const row = page.locator(".drive-row-hit", { hasText: "Downloads" }).first();
await row.click({ button: "right" });
await page.getByRole("menuitem", { name: "Rename" }).click();
const field = page.getByLabel("Rename Downloads");
await field.fill("a:b");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok((await notice()).some((t) => /can.t be used/.test(t)) && (await sent()) === 0 && (await page.locator(".drive-name", { hasText: "Downloads" }).count()) > 0,
  "a rename to \"a:b\" is refused and the name stays");

await page.getByRole("button", { name: "New Folder" }).click();
await page.keyboard.type("Trips 2026");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok((await sent()) === 1, "an ordinary name still goes to iCloud");
await b.close();
process.exitCode = failures ? 1 : 0;
