// Copy, as in Photos: Ctrl+C on the selected photo, or on the photo open in the viewer, and
// "Copy" in a photo's menu, put it on the clipboard — the app makes the PNG (stubbed here) — and
// say "Copied" quietly. A video offers no Copy and Ctrl+C on one does nothing; several chosen
// copy the first photo of them and say so; a copy that takes a moment says "Copying photo" until
// it is done; one that fails says why. Where the app hands the PNG back (Windows, macOS) the
// window puts it on the clipboard itself: checked in Chromium, whose clipboard can be read back.
//
//   URL=http://localhost:4173 node .uiverify/photos-copy.mjs   (ENGINE=webkit for WebKit)
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { photosNewestFirst: true, screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(URL, { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.locator('.px-tile[data-id="p3"]').waitFor();

const copies = () => page.evaluate(() => window.__copies ?? []);
const toastText = () => page.locator(".toast-host").innerText().catch(() => "");
const waitCopies = (n) => page.waitForFunction((n) => (window.__copies ?? []).length >= n, n);
const waitToast = (re) => page.waitForFunction((src) => new RegExp(src).test(document.querySelector(".toast-host")?.innerText ?? ""), re.source);
const clearToasts = () => page.evaluate(() => document.querySelectorAll(".toast-close").forEach((b) => b.click()));

// Ctrl+C on one selected photo.
await page.locator('.px-tile[data-id="p2"]').click();
await page.keyboard.press("Control+c");
await waitCopies(1);
await waitToast(/Copied/);
ok(JSON.stringify(await copies()) === '["p2"]', `Ctrl+C copies the selected photo (${JSON.stringify(await copies())})`);
ok(/^\s*Copied\s*$/m.test(await toastText()), `…and says "Copied" (${JSON.stringify(await toastText())})`);
await clearToasts();

// A photo's menu offers Copy; a video's does not, and Ctrl+C on a video copies nothing.
await page.locator('.px-tile[data-id="p4"]').click({ button: "right" });
ok((await page.locator(".row-menu").getByText("Copy", { exact: true }).count()) === 1, "a photo's menu offers Copy");
await page.locator(".row-menu").getByText("Copy", { exact: true }).click();
await waitCopies(2);
ok((await copies()).at(-1) === "p4", "Copy in the menu copies that photo");
await clearToasts();
await page.locator('.px-tile[data-id="p3"]').click({ button: "right" });
ok((await page.locator(".row-menu").getByText("Copy", { exact: true }).count()) === 0, "a video's menu offers no Copy");
await page.keyboard.press("Escape");
await page.locator('.px-tile[data-id="p3"]').click();
await page.keyboard.press("Control+c");
await page.waitForTimeout(250);
ok((await copies()).length === 2, `Ctrl+C on a video copies nothing (${(await copies()).length} copies)`);

// Several chosen, a video first: the first photo of them, and it says so.
await page.locator('.px-tile[data-id="p5"]').click({ modifiers: ["Control"] });
await page.locator('.px-tile[data-id="p1"]').click({ modifiers: ["Control"] });
await page.keyboard.press("Control+c");
await waitCopies(3);
await waitToast(/Copied the first photo/);
ok((await copies()).at(-1) === "p1", `several chosen copy the first photo of them, passing the video (${(await copies()).at(-1)})`);
await clearToasts();

// In the viewer: Ctrl+C copies the photo shown.
await page.keyboard.press("Escape");
await page.locator('.px-tile[data-id="p5"]').dblclick();
await page.locator(".px-viewer").waitFor();
await page.keyboard.press("Control+c");
await waitCopies(4);
ok((await copies()).at(-1) === "p5", "Ctrl+C in the viewer copies the photo shown");
await page.locator(".px-stage").click({ button: "right", position: { x: 300, y: 300 } });
ok((await page.locator(".row-menu").getByText("Copy", { exact: true }).count()) === 1, "the open photo's own menu offers Copy");
await page.keyboard.press("Escape");
await clearToasts();

// A slow copy says so while it is made; a quick one never does.
await page.evaluate(() => { window.__copyDelay = 900; });
await page.keyboard.press("Control+c");
await waitToast(/Copying photo/);
ok(true, "a copy that takes a moment says \"Copying photo\"");
await waitToast(/^\s*Copied\s*$/m);
ok(!/Copying photo/.test(await toastText()), `…which goes once it is copied (${JSON.stringify(await toastText())})`);
await clearToasts();
await page.evaluate(() => { window.__copyDelay = 0; window.__copyFail = true; });
await page.keyboard.press("Control+c");
await waitToast(/clipboard didn’t take/);
ok(true, "a copy that fails says why");
await page.evaluate(() => { window.__copyFail = false; });
await clearToasts();
await page.keyboard.press("Escape");
await ctx.close();

// Windows and macOS: the PNG comes back and the window puts it on the clipboard.
if (engineName === "chromium") {
  const c2 = await b.newContext({ viewport: { width: 1280, height: 800 }, permissions: ["clipboard-read", "clipboard-write"] });
  await c2.addInitScript(installStub, { photosNewestFirst: true, screen: "library", services: SERVICES, twoFactor: null });
  // A 1×1 PNG.
  await c2.addInitScript(() => { window.__copyPng = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg=="; });
  const p2 = await c2.newPage();
  p2.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await p2.goto(URL, { waitUntil: "networkidle" });
  await openScreen(p2, "Photos");
  await p2.locator('.px-tile[data-id="p2"]').click();
  await p2.keyboard.press("Control+c");
  await p2.waitForFunction(() => /Copied/.test(document.querySelector(".toast-host")?.innerText ?? ""));
  const types = await p2.evaluate(async () => (await navigator.clipboard.read()).flatMap((i) => i.types));
  ok(types.includes("image/png"), `where the PNG comes back, it is on the clipboard as image/png (${types.join(", ")})`);
  await c2.close();
}

await b.close();
process.exitCode = failures ? 1 : 0;
