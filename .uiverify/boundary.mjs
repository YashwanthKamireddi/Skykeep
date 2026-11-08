// One screen failing to draw must not take the app with it.
//
// Replaying the real library, a contact row that threw while drawing its photo unmounted
// everything: the sidebar, every screen, the way back — a blank window until a restart.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true });
// A row that throws while it draws: a contact with a photo, whose address cannot be made.
await ctx.addInitScript(() => {
  const internals = window.__TAURI_INTERNALS__;
  const invoke = internals.invoke;
  internals.convertFileSrc = () => { throw new Error("cannot make a photo address"); };
  internals.invoke = async (cmd, args) => {
    const answer = await invoke(cmd, args);
    if (cmd === "contacts_list" && Array.isArray(answer) && answer[0]) answer[0] = { ...answer[0], photoUrl: "x" };
    return answer;
  };
});
const page = await ctx.newPage();
page.on("pageerror", () => {});
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });

let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };

// Contacts' own window, opened without waiting for it to draw: it is made to fail.
await page.goto("http://localhost:4173/?app=contacts", { waitUntil: "networkidle" });
await page.waitForTimeout(500);
const after = await page.evaluate(() => ({
  window: Boolean(document.querySelector("main.content[data-screen]")),
  said: document.querySelector(".screen-failed")?.textContent ?? null,
}));
ok(after.window, "a screen that fails to draw leaves its window standing", `the app went blank: ${JSON.stringify(after)}`);
ok(Boolean(after.said?.includes("Contacts couldn’t be shown")), "and says so in its place", `nothing said: ${JSON.stringify(after)}`);
// Every other app still opens, each in its own window.
await openScreen(page, "Notes");
await page.waitForTimeout(400);
ok((await page.locator(".note-row").count()) > 0, "and every other app still opens", "Notes did not open after");
await b.close();
process.exitCode = failures === 0 ? 0 : 1;
