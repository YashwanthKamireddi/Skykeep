// A note saved as Markdown from its toolbar: the save dialog offers the note's title as the
// file's name, and what is written is the note as it is on screen — typing not yet sent
// included — with its formatting. Cancelling the dialog writes nothing.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 820 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Notes");
  await page.waitForTimeout(400);
  await page.locator(".note-row", { hasText: "Curtains" }).click();
  await page.waitForTimeout(400);
  const button = page.getByRole("button", { name: "Export as Markdown" });
  ok(await button.isVisible(), `${scheme}: Export as Markdown is in the note's toolbar`);

  // Cancelled: nothing written.
  await button.click();
  await page.waitForTimeout(300);
  const asked = await page.evaluate(() => window.__saveAsked);
  ok(asked?.options?.defaultPath === "Curtains.md", `${scheme}: the dialog offers the title as its name  (${asked?.options?.defaultPath})`);
  ok((await page.evaluate(() => (window.__exported ?? []).length)) === 0, `${scheme}: cancelled, nothing is written`);

  // Typed into, not yet sent: the export has it.
  await page.locator(".note-rich-host .ProseMirror").click();
  await page.keyboard.press("Control+End");
  await page.keyboard.type(" plus a lamp");
  await page.evaluate(() => { window.__savePath = "/tmp/Curtains.md"; });
  await button.click();
  await page.waitForTimeout(300);
  const sent = await page.evaluate(() => (window.__exported ?? [])[0]);
  ok(sent?.path === "/tmp/Curtains.md", `${scheme}: written where it was asked`);
  ok(/^Curtains\n/.test(sent?.body ?? "") && /plus a lamp/.test(sent?.body ?? ""), `${scheme}: with the text on screen, typing included`);
  ok(Array.isArray(sent?.runs) && sent.runs.length > 0 && sent.runs.reduce((n, r) => n + r.length, 0) === [...(sent.body ?? "")].reduce((n, c) => n + c.length, 0),
    `${scheme}: and its formatting, covering the whole text  (${sent?.runs?.length} runs)`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
