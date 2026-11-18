// Reminders reopens where it was left, as Apple's does: the list and the reminder whose details
// were open, kept on this computer (lib/place); a list deleted since falls back to Today.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
const url = `${process.env.URL ?? process.env.BASE ?? "http://localhost:4173"}/?app=reminders`;
const kept = () => page.evaluate(() => JSON.parse(localStorage.getItem("quince.reminders.place") ?? "null"));
const settle = () => page.waitForTimeout(700);

await page.goto(url, { waitUntil: "networkidle" });
await settle();
await page.locator("button, [role=button]", { hasText: /^GYM/ }).first().click();
await settle();
ok(JSON.stringify(await kept()) === '{"container":{"type":"list","guid":"l2"},"item":null}', `the list is kept  ${JSON.stringify(await kept())}`);
await page.reload({ waitUntil: "networkidle" });
await settle();
ok(JSON.stringify(await kept()) === '{"container":{"type":"list","guid":"l2"},"item":null}', `reopened, GYM is the list open  ${JSON.stringify(await kept())}`);
ok((await page.getByRole("checkbox", { name: "Evening run" }).count()) > 0, "with its reminders");

await page.evaluate(() => localStorage.setItem("quince.reminders.place", JSON.stringify({ container: { type: "list", guid: "l2" }, item: "r3" })));
await page.reload({ waitUntil: "networkidle" });
await settle();
ok(JSON.stringify(await kept()) === '{"container":{"type":"list","guid":"l2"},"item":"r3"}', `the reminder's details open again  ${JSON.stringify(await kept())}`);

await page.evaluate(() => localStorage.setItem("quince.reminders.place", JSON.stringify({ container: { type: "list", guid: "gone" }, item: "r3" })));
await page.reload({ waitUntil: "networkidle" });
await settle();
ok(JSON.stringify(await kept()) === '{"container":{"type":"smart","kind":"today"},"item":null}', `a deleted list falls back to Today  ${JSON.stringify(await kept())}`);
await b.close();
process.exitCode = failures ? 1 : 0;
