// Walking a long list with the arrow keys, in each app that has one: each step should cost
// the two rows the selection moved between, not a pass over every row. Measured in the page
// itself — the key handled, the screen drawn again, its style and layout worked out — so the
// number is the app's work and not Playwright's round trip. Prints each list's numbers.
//
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/list-walk.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const N = 3000;
const STEPS = 40;
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

/** A page with the stub, and `add` answering each command with a long list of invented rows. */
async function open(fixture, add) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, ...fixture });
  await ctx.addInitScript(([n, add]) => {
    const extra = new Function("n", "cmd", "args", "out", add);
    const wrap = () => {
      const t = window.__TAURI_INTERNALS__;
      if (!t || t.__many) return;
      const inner = t.invoke;
      t.invoke = async (cmd, args) => extra(n, cmd, args, await inner(cmd, args));
      t.__many = true;
    };
    wrap(); document.addEventListener("DOMContentLoaded", wrap);
  }, [N, add]);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  await page.goto(process.env.BASE ?? process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
  return { ctx, page };
}

/** Each step's work: the key handled, React's render committed, style and layout done. */
const walk = (page, selected) => page.evaluate(async ([steps, selected]) => {
  const times = [];
  for (let i = 0; i < steps; i++) {
    const at = document.activeElement ?? document.body;
    const t0 = performance.now();
    at.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", code: "ArrowDown", bubbles: true, cancelable: true }));
    // React flushes a key's update in a microtask; then the style and layout it caused.
    for (let k = 0; k < 4; k++) await Promise.resolve();
    void document.body.offsetHeight;
    times.push(performance.now() - t0);
    // The row brought into view and focused, as the app does in the next frame.
    await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
  }
  const where = document.querySelector(selected)?.textContent?.slice(0, 40) ?? null;
  times.sort((a, z) => a - z);
  return { median: times[steps >> 1], p90: times[Math.floor(steps * 0.9)], where };
}, [STEPS, selected]);

const report = (name, m, limit) => {
  console.log(`${engineName}: ${name} — ${m.median.toFixed(2)} ms a step (p90 ${m.p90.toFixed(2)})  at ${JSON.stringify(m.where)}`);
  ok(m.where !== null, `${name}: the arrows moved the selection`);
  ok(m.median < limit, `${name}: a step stays quick  ${m.median.toFixed(2)} ms`);
};

// Contacts: 3000 people.
{
  const { ctx, page } = await open({ contactsFixture: true }, `
    if (cmd !== "contacts_list" || !Array.isArray(out) || args?.query) return out;
    const letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    return [...out, ...Array.from({ length: n }, (_, i) => ({
      guid: "m" + i, firstName: "Person" + i, lastName: letters[i % 26] + "name" + i, company: null, isCompany: false,
      birthday: null, notes: null, etag: null, methods: [] }))];`);
  await openScreen(page, "Contacts");
  await page.waitForFunction((n) => document.querySelectorAll(".contact-row").length >= n, N, { timeout: 20000 });
  await page.locator(".contact-row").first().click();
  await page.waitForTimeout(300);
  report(`Contacts, ${N} people`, await walk(page, '.contact-row[aria-selected="true"]'), 40);
  await ctx.close();
}

// Notes: 3000 notes in one folder.
{
  const { ctx, page } = await open({ notesFixture: true }, `
    if (cmd !== "notes_in" || !Array.isArray(out)) return out;
    return [...out, ...Array.from({ length: n }, (_, i) => ({
      guid: "bulk" + i, folderGuid: "f1", title: "Note number " + (i + 1), snippet: "A line of what it says",
      body: null, locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1780000000 - i * 600, etag: null }))];`);
  await openScreen(page, "Notes");
  await page.waitForFunction((n) => document.querySelectorAll(".note-row").length >= n, N, { timeout: 20000 });
  await page.locator(".note-row").first().click();
  await page.locator(".note-row").first().focus();
  await page.waitForTimeout(300);
  report(`Notes, ${N} notes`, await walk(page, '.note-row[aria-selected="true"]'), 40);
  await ctx.close();
}

// Drive: 3000 files in one folder.
{
  const { ctx, page } = await open({ driveFixture: true }, `
    if (cmd !== "drive_children" || !Array.isArray(out)) return out;
    return [...out, ...Array.from({ length: n }, (_, i) => ({
      drivewsid: "FILE::bulk" + i, parentId: null, name: "Scan " + String(i).padStart(4, "0"), extension: "pdf",
      kind: "FILE", size: 1000 + i, childCount: null, createdAt: 1780000000, modifiedAt: 1780000000 + i, etag: "e" + i }))];`);
  await openScreen(page, "Drive");
  await page.waitForFunction((n) => document.querySelectorAll(".drive-row-hit").length >= n, N, { timeout: 20000 });
  await page.locator("button.drive-row-hit").first().click();
  await page.waitForTimeout(300);
  report(`Drive, ${N} files`, await walk(page, '.drive-row-hit[aria-pressed="true"]'), 40);
  await ctx.close();
}

// Reminders: 1000 in one list (the arrows move the focus from row to row).
{
  const { ctx, page } = await open({ remindersFixture: true }, "return out;");
  await ctx.addInitScript(() => {
    const now = Math.floor(Date.now() / 1000);
    window.__extraReminders = Array.from({ length: 1000 }, (_, i) => ({
      guid: `bulk-${i}`, listGuid: "l1", title: `Errand number ${i + 1}`, notes: null,
      dueAt: now + (i + 1) * 3600, completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null,
    }));
  });
  await page.reload({ waitUntil: "networkidle" });
  await openScreen(page, "Reminders");
  await page.locator(".rx-list", { hasText: "Daily" }).first().click();
  await page.waitForFunction(() => document.querySelectorAll(".rx-rows .rx-row").length >= 1000, null, { timeout: 20000 });
  await page.locator(".rx-rows .rx-row").first().focus();
  await page.waitForTimeout(300);
  report("Reminders, 1000 reminders", await walk(page, ".rx-rows .rx-row:focus .rx-title"), 40);
  await ctx.close();
}

await b.close();
process.exitCode = failures ? 1 : 0;
