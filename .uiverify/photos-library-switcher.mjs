// The Shared Library switcher, as Photos has it: under the Photos heading, only for an account
// with a Shared Library; Both Libraries, Personal Library or Shared Library, each showing only
// its own photos in the grid, kept as chosen.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

async function open(withShared) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  await ctx.addInitScript((shared) => {
    window.__hasShared = shared;
    // p2 and p4 are the Shared Library's.
    window.__photoExtra = shared ? { p2: { zone: "SharedSync-1" }, p4: { zone: "SharedSync-1" } } : {};
  }, withShared);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Photos");
  await page.waitForFunction(() => document.querySelectorAll(".px-tile").length > 0);
  return { ctx, page };
}
const tiles = (page) => page.evaluate(() => [...document.querySelectorAll(".px-tile[data-id]")].map((t) => t.dataset.id).sort().join(","));

{
  const { ctx, page } = await open(false);
  ok((await page.locator(".px-library").count()) === 0, "no Shared Library: no switcher");
  await ctx.close();
}
{
  const { ctx, page } = await open(true);
  const sw = page.locator(".px-library .picker-button");
  ok((await sw.count()) === 1 && /Both Libraries/.test(await sw.innerText()), "with one: the switcher, on Both Libraries");
  const both = await tiles(page);
  ok(both.includes("p2") && both.includes("p1"), `Both shows every photo  ${both}`);
  for (const [label, want, gone] of [["Personal Library", "p1", "p2"], ["Shared Library", "p2", "p1"]]) {
    await sw.click();
    await page.getByRole("menuitemradio", { name: label }).or(page.getByRole("option", { name: label })).or(page.getByRole("menuitem", { name: label })).first().click();
    await page.waitForFunction((g) => !document.querySelector(`.px-tile[data-id="${g}"]`), gone, { timeout: 3000 }).catch(() => {});
    const now = await tiles(page);
    ok(now.includes(want) && !now.includes(gone), `${label} shows only its own  ${now}`);
    ok(new RegExp(label).test(await sw.innerText()) && (await page.evaluate(() => window.__library)) === (label.startsWith("Personal") ? "personal" : "shared"), `${label} is kept as chosen`);
  }
  await page.screenshot({ path: "/tmp/library-switcher.png" });
  await ctx.close();
}
await b.close();
console.log(failures ? `${failures} FAILED` : "all passed");
process.exit(failures ? 1 : 0);
