// Notes reopens where it was left, as Apple's does: the folder and the note open in it, kept on
// this computer (lib/place); a folder deleted since falls back to All, a note gone to the first.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
const url = `${process.env.URL ?? process.env.BASE ?? "http://localhost:4173"}/?app=notes`;
const kept = () => page.evaluate(() => JSON.parse(localStorage.getItem("quince.notes.place") ?? "null"));
const current = () => page.locator('.notes-folder[aria-current="true"]').first().innerText();
const settle = () => page.waitForTimeout(600);

await page.goto(url, { waitUntil: "networkidle" });
await settle();
await page.locator(".notes-folder", { hasText: /^Notes/ }).first().click();
await settle();
await page.getByText("These sources outline", { exact: false }).first().click();
await settle();
ok(JSON.stringify(await kept()) === '{"container":"f1","item":"n3"}', `the folder and note are kept  ${JSON.stringify(await kept())}`);
await page.reload({ waitUntil: "networkidle" });
await settle();
ok((await current()).startsWith("Notes"), `reopened, the Notes folder is the one open  (${await current()})`);
ok((await kept())?.item === "n3", `and the same note is open  ${JSON.stringify(await kept())}`);

await page.evaluate(() => localStorage.setItem("quince.notes.place", JSON.stringify({ container: "gone", item: "n3" })));
await page.reload({ waitUntil: "networkidle" });
await settle();
ok((await kept())?.container === "__all__", `a deleted folder falls back to All  ${JSON.stringify(await kept())}`);
ok((await kept())?.item === "n3", "its note, still there, is open in All");

await page.evaluate(() => localStorage.setItem("quince.notes.place", JSON.stringify({ container: "f1", item: "deleted" })));
await page.reload({ waitUntil: "networkidle" });
await settle();
const k = await kept();
ok(k?.container === "f1" && ["n1", "n2", "n3"].includes(k?.item), `a deleted note falls back to the folder's first  ${JSON.stringify(k)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
